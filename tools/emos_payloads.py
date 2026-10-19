#!/usr/bin/env python3
"""EchoMuse payload 安装：把 controller 层三件套装进 emOS 设备。

等价于官方向导 provisioning 的写入动作（dashboard.jsx 逐文件写 + md5
校验），但传输一律走网络：USB 串口（TTYGS0）只作命令通道（探测、mkdir、
mv、md5、reboot），文件本体由设备端 wget/curl 从本机临时 HTTP 服务拉取
（同网段 WiFi，实测 ~5 MB/s，12MB 固件秒级）。串口 heredoc 传输（6 KB/s、
12MB 半小时）已移除——真机验证网络拉取一次成功后无保留价值。

写入内容（均按上游 em_payloads.py / em_endpoints.py 核实）:
  /data/local/bin/server_a                  Go 固件（release 资产 `server`）
  /data/local/bin/server -> server_a        A/B 槽软链（回滚机制依赖它）
  /data/local/bin/start_server.sh           监督脚本（755）
  /data/local/etc/echomuse/controller.json  控制器地址表（固件每次拨号重读）

用法:
  python3 tools/emos_payloads.py                          # 下载+安装+校验
  python3 tools/emos_payloads.py --host 192.168.99.249 --port 8000
  python3 tools/emos_payloads.py --reboot                 # 完成后重启设备
  python3 tools/emos_payloads.py --local-server PATH      # 用本地固件跳过下载

前置: 设备已跑 emOS 并入网（wlan0 有 IP），设备端有 wget 或 curl，
      服务器 [echomuse] enabled=true。
"""
from __future__ import annotations

import argparse
import functools
import hashlib
import http.server
import json
import re
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from emos_install import Console, die, find_console_port, log  # noqa: E402

RELEASE_API = "https://api.github.com/repos/wilbowes/EchoMuse/releases/latest"
SERVER_URL = ("https://github.com/wilbowes/EchoMuse/releases/download/"
              "{tag}/server")
START_SCRIPT_URL = ("https://raw.githubusercontent.com/wilbowes/EchoMuse/"
                    "main/controller/device_payloads/start_server.sh")
# GitHub 直连不稳时经 gh-proxy 型前缀代理下载（实测 tvv.tw 可代理 raw 与 api）。
PROXY_BASE = "https://tvv.tw/"
USE_PROXY = True  # main() 里按 --no-proxy 关闭

BIN_DIR = "/data/local/bin"
ETC_DIR = "/data/local/etc/echomuse"


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def md5_bytes(data: bytes) -> str:
    return hashlib.md5(data).hexdigest()


def fetch(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "emos-payloads"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read()


def fetch_github(url: str) -> bytes:
    """GitHub 资源下载：默认走代理前缀，失败回退直连。"""
    if USE_PROXY:
        purl = PROXY_BASE + url
        try:
            return fetch(purl)
        except OSError as e:
            log(f"代理下载失败（{e}），回退直连 GitHub ...")
    return fetch(url)


# ---------- 下载 ----------

def download_firmware(work: Path, local: str | None) -> Path:
    if local:
        p = Path(local).expanduser()
        if not p.exists():
            die(f"本地固件不存在: {p}")
        log(f"使用本地固件: {p} ({p.stat().st_size} 字节)")
        return p
    cache = work / "server.fw"
    info = json.loads(fetch_github(RELEASE_API))
    tag, digest = info["tag_name"], ""
    for a in info.get("assets", []):
        if a["name"] == "server":
            digest = (a.get("digest") or "").removeprefix("sha256:")
    if cache.exists() and digest and sha256_file(cache) == digest:
        log(f"缓存命中且 sha256 校验通过: {cache.name}（{tag}）")
        return cache
    url = SERVER_URL.format(tag=tag)
    log(f"下载固件 {url} ...")
    data = fetch_github(url)
    if digest and hashlib.sha256(data).hexdigest() != digest:
        die("固件 sha256 与 release 清单不符，拒绝使用", "重跑重试下载")
    cache.write_bytes(data)
    log(f"固件已下载: {tag} {len(data)} 字节 sha256 前 12 位 {digest[:12]}")
    return cache


def controller_json(host: str, port: int, tls_port: int) -> bytes:
    """格式与 em_endpoints.file_bytes 完全一致（固件 JSON 解码器所期望）。"""
    doc = {"managed_by": "echomuse-controller",
           "endpoints": [{"host": host, "port": port, "tls_port": tls_port}]}
    return (json.dumps(doc, separators=(",", ":")) + "\n").encode("ascii")


# ---------- 设备命令通道（串口） ----------

class DeviceShell:
    """Console 之上的设备命令层：命令执行 + 设备端 md5 查询。

    只跑短命令，不做数据传输（文件一律走 net_pull 的 WiFi 拉取）。
    标记拆分坑保留：__EO""C__ / M'D'5 拆开写，否则设备回显命令行本身时
    会把标记原文带进输出，expect/md5 解析永远误命中（真机踩过）。
    """

    def __init__(self, con: Console):
        self.con = con
        self.PING_TIMEOUT_S = 6  # 探活等待

    # ---- 会话管理 ----

    def sync(self) -> None:
        """清场：中断的 heredoc 会把 shell 留在续行态（提示符 '> '，输入
        被吞、永不回显标记）。Ctrl-C 连发退回干净 shell。"""
        for _ in range(3):
            self.con.send("\x03")
            time.sleep(0.15)
        self.con.send("\n")
        self.con.drain(0.6)
        self.con.buf = ""

    def _ping(self) -> bool:
        """非致命探活：shell 是否执行我们发出的标记。"""
        self.con.buf = ""
        self.con.send('echo "__PI""NG__"\n')
        deadline = time.time() + self.PING_TIMEOUT_S
        while time.time() < deadline:
            if "__PING__" in self.con.drain(0.3):
                return True
        return False

    def ensure_shell(self) -> None:
        """确保 shell 可响应；不响应则重开串口（对应官方的通道重连）。"""
        self.sync()
        if self._ping():
            return
        log("  shell 未响应，重开串口 ...")
        self.con.reopen()
        self.sync()
        if not self._ping():
            die("串口 shell 无法恢复响应", "拔插一次 USB 线后重跑本脚本")

    # ---- 命令 ----

    def run(self, cmd: str, timeout: float = 30) -> str:
        self.con.buf = ""
        self.con.send(cmd + '; echo "__EO""C__"\n')
        return self.con.expect("__EOC__", timeout=timeout)

    def md5_of(self, path: str) -> str | None:
        """官方 M'D'5 拆分标记：命令回显是 M'D'5:...，不会命中 MD5: 匹配。"""
        out = self.run(f"echo M'D'5:$(busybox md5sum {path} | "
                       f"busybox cut -d' ' -f1)", timeout=120)
        for line in out.splitlines():
            line = line.strip()
            if line.startswith("MD5:") and len(line) == 4 + 32:
                return line[4:]
        return None

    # ---- 文件传输已移除：net_pull 是唯一路径（本模块底部） ----


def mdns_device_present(timeout: float = 5.0) -> bool:
    """Mac 侧浏览 _emcontroller._tcp 广播，确认 emOS 设备在网。

    用 dns-sd 一次性 browse（macOS 自带），超时未见实例即 False——
    失败只影响识别路径，不阻断写入（设备可能尚未入网）。
    """
    if sys.platform != "darwin":
        return False
    try:
        p = subprocess.run(["dns-sd", "-B", "_emcontroller._tcp"],
                           capture_output=True, text=True, timeout=timeout)
        # dns-sd 被 timeout 杀掉也会在 stdout 留下浏览结果
        return "_emcontroller._tcp" in (p.stdout + p.stderr) and \
            "0 added" not in p.stdout
    except (OSError, subprocess.TimeoutExpired):
        return False


# ---------- 网络拉取（唯一传输路径） ----------
# 串口 heredoc 已移除：实测设备端 wget 拉取 12MB 仅数秒（5.2 MB/s），而
# 串口 heredoc 只有 6 KB/s（base64 膨胀 x4/3 + canonical 逐行回显 x4/3 +
# 每块 ~1300 行的行规程开销）。前置不满足时 net_pull 返回 False 并说明
# 原因，调用方 pull_or_die 终止并给出排查指引。

class _QuietHandler(http.server.SimpleHTTPRequestHandler):
    """静态文件服务，静默访问日志（避免刷串口会话的本地终端）。"""

    def log_message(self, fmt: str, *args) -> None:
        pass


def device_fetcher(dev: DeviceShell) -> str | None:
    """探测设备端 HTTP 拉取器：wget > curl，都没有返回 None。

    注意标记拆分坑：R1/R2 前缀只出现在命令的执行输出里，回显的命令行
    本身含 '$(' 原文，正则按行首匹配 + 值以 wget/curl 结尾双重排除。
    设备 tty 是 onlcr，回显行尾为 \r\n，'$' 锚点不吃 '\r'——先归一化
    再解析，否则设备明明有 wget 也会被解析成没有（真机踩过）。
    """
    out = dev.run('echo "R1:$(command -v wget 2>/dev/null)"; '
                  'echo "R2:$(command -v curl 2>/dev/null)"')
    out = out.replace("\r\n", "\n")
    m1 = re.search(r"^R1:(\S*)$", out, re.M)
    m2 = re.search(r"^R2:(\S*)$", out, re.M)
    if m1 and m1.group(1).endswith("wget"):
        return "wget"
    if m2 and m2.group(1).endswith("curl"):
        return "curl"
    return None


def device_ip(dev: DeviceShell) -> str | None:
    """设备 wlan0 的 IPv4；没入网返回 None（ifconfig/ip 两种格式都解析）。"""
    out = dev.run("ifconfig wlan0 2>/dev/null; ip addr show wlan0 2>/dev/null")
    for m in re.finditer(r"inet (?:addr:)?(\d+\.\d+\.\d+\.\d+)", out):
        if m.group(1) != "127.0.0.1":
            return m.group(1)
    return None


def mac_ip_toward(device_ip: str) -> str | None:
    """Mac 上朝 device_ip 方向说话的接口地址（route get -> ipconfig）。"""
    try:
        r = subprocess.run(["route", "-n", "get", device_ip],
                           capture_output=True, text=True, timeout=5)
        m = re.search(r"interface:\s*(\S+)", r.stdout)
        if not m:
            return None
        r = subprocess.run(["ipconfig", "getifaddr", m.group(1)],
                           capture_output=True, text=True, timeout=5)
        return r.stdout.strip() or None
    except (OSError, subprocess.TimeoutExpired):
        return None


def net_pull(dev: DeviceShell, dest: str, name: str, data: bytes,
             want: str, fetcher: str) -> bool:
    """本机临时 HTTP 服务 -> 设备端 wget/curl 拉取 data 到 dest，md5 判定。

    前置（设备 wlan0 有 IP、本机有朝设备方向的地址）不满足时返回 False
    并说明原因；md5 不一致同样返回 False，不 die（由调用方定夺）。
    """
    dev_ip = device_ip(dev)
    if not dev_ip:
        log("设备 wlan0 无 IP（未入网）")
        return False
    my_ip = mac_ip_toward(dev_ip)
    if not my_ip:
        log("无法确定本机朝设备方向的 IP")
        return False
    staging = tempfile.mkdtemp(prefix="emos-pull-")
    Path(staging, name).write_bytes(data)
    handler = functools.partial(_QuietHandler, directory=staging)
    httpd = http.server.ThreadingHTTPServer(("0.0.0.0", 0), handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    url = f"http://{my_ip}:{httpd.socket.getsockname()[1]}/{name}"
    if fetcher == "wget":
        cmd = f"rm -f {dest}; wget -q -T 60 -O {dest} {url}"
    else:
        cmd = f"rm -f {dest}; curl -fsS -m 180 -o {dest} {url}"
    log(f"设备端 {fetcher} 拉取 {url}（设备 {dev_ip} -> 本机 {my_ip}）...")
    t0 = time.monotonic()
    try:
        out = dev.run(cmd, timeout=300)
        got = dev.md5_of(dest)
        if got != want:
            log(f"网络拉取 md5 不一致（设备 {got}）；若反复失败检查 macOS "
                f"防火墙是否放行 python3 入站、路由器是否开了 AP 隔离。"
                f"设备端输出尾部: {out[-120:]!r}")
            return False
        rate = len(data) / max(time.monotonic() - t0, 0.1) / 1048576
        log(f"网络拉取完成并校验 ✔（{rate:.1f} MB/s）")
        return True
    finally:
        httpd.shutdown()
        httpd.server_close()


def pull_or_die(dev: DeviceShell, part: str, name: str, data: bytes,
                want: str, label: str, fetcher: str) -> None:
    """网络拉取是唯一传输路径；失败即终止（不再有串口回落）。"""
    if not net_pull(dev, part, name, data, want, fetcher):
        die(f"{label} 经 WiFi 拉取失败",
            "确认设备已入网（wlan0 有 IP）、macOS 防火墙放行 python3 入站、"
            "路由器未开 AP 隔离，然后重跑本脚本")


# ---------- 安装流程 ----------

def install(dev: DeviceShell, fw: Path, script: bytes, endpoints: bytes,
            reboot: bool, use_net: bool = True) -> None:
    dev.ensure_shell()  # 探活；必要时补灌 NUL 复位上次残留的 raw 传输
    # 0. mDNS 广播识别设备（用户指定路径）：emOS 的 controller 守护进程会
    #    advertise/browse _emcontroller._tcp，Mac 侧能浏览到即设备在跑 emOS。
    found = mdns_device_present()
    # 串口侧仅作辅助确认。注意标记必须拆开写（__ND""O__），否则设备回显
    # 命令行本身时会把标记原文带进输出，守卫永远误触发（已踩坑）。
    out = dev.run('uname -o 2>/dev/null; '
                  '[ -d /data ] && echo DATA_OK || echo "__ND""O__"')
    console_ok = "__NDO__" not in out and "android" not in out.lower()
    if found:
        log("mDNS 广播识别到设备 ✔" + ("" if console_ok else
            "（串口探测异常，按广播结果继续）"))
    elif console_ok:
        log("emOS 确认 ✔（串口；未看到 mDNS 广播，可能未入网，"
            "不影响本脚本写入）")
    else:
        die("既没有 mDNS 广播，串口也不像 emOS 控制台",
            "先完成 emOS 安装（tools/emos_install.py 全流程）")
    # payload 目标目录，裸 emOS 可能都还没有
    dev.run(f"mkdir -p {BIN_DIR} {ETC_DIR}")

    # 1. 固件二进制 -> server_a -> 软链 server（A/B 回滚机制要求软链形态）
    want = md5_bytes(fw.read_bytes())
    if dev.md5_of(f"{BIN_DIR}/server_a") == want:
        log("server_a 已是目标版本，跳过二进制传输")
    else:
        if use_net and net_pull(dev, "/tmp/emos-server.part", fw, want):
            pass  # 设备端已就位，net_pull 内部 md5 已过，下方再复检一次
        else:
            dev.push_file("/tmp/emos-server.part", fw.read_bytes())
        got = dev.md5_of("/tmp/emos-server.part")
        if got != want:
            die(f"固件 md5 不匹配: 本地 {want} 设备 {got}",
                "串口传输受损，重跑本脚本（会重传）")
        dev.run(f"mv /tmp/emos-server.part {BIN_DIR}/server_a && "
                f"chmod 755 {BIN_DIR}/server_a")
        dev.run(f"ln -sf server_a {BIN_DIR}/server")
        if dev.md5_of(f"{BIN_DIR}/server_a") != want:
            die("server_a 落盘后 md5 复检失败", "eMMC 写入异常，重跑")
        log("固件 server_a 写入并校验 ✔")

    # 2. 监督脚本（755）
    if dev.md5_of(f"{BIN_DIR}/start_server.sh") != md5_bytes(script):
        dev.push_file("/tmp/emos-start.part", script)
        dev.run(f"NEW=$(md5sum /tmp/emos-start.part | cut -d' ' -f1); "
                f"if [ \"$NEW\" = \"{md5_bytes(script)}\" ]; then "
                f"mv /tmp/emos-start.part {BIN_DIR}/start_server.sh && "
                f"chmod 755 {BIN_DIR}/start_server.sh && echo SCRIPT_OK; "
                f"else echo SCRIPT_BAD; fi")
        if "SCRIPT_OK" not in dev.run(f"ls -l {BIN_DIR}/start_server.sh && "
                                      "echo SCRIPT_OK"):
            die("start_server.sh 安装校验失败", "重跑本脚本")
    log("start_server.sh 就位 ✔")

    # 3. 控制器地址表（644；固件每次拨号重读，mDNS 仍兜底）
    dev.push_file(f"{ETC_DIR}/controller.json", endpoints)
    dev.run(f"chmod 644 {ETC_DIR}/controller.json")
    if dev.md5_of(f"{ETC_DIR}/controller.json") != md5_bytes(endpoints):
        die("controller.json md5 校验失败", "重跑本脚本")
    log("controller.json 就位 ✔")

    # 4. 终态展示
    out = dev.run(f"ls -l {BIN_DIR}/ {ETC_DIR}/controller.json; "
                  f"readlink {BIN_DIR}/server")
    print(out)
    if reboot:
        log("重启设备使 payload 生效（init 将拉起 echomuse 服务）...")
        dev.run("reboot", timeout=10)
        log("已发送 reboot。冷启动到联网约 35s，之后设备会拨 "
            "controller.json 指向的地址；服务器日志流出现 echomuse register "
            "即接入成功。")
    else:
        log("完成。在串口或拔插电执行 `reboot` 后，设备将连向 controller.json "
            "里的地址；服务器日志流出现 echomuse register 即接入成功。")


def main() -> None:
    ap = argparse.ArgumentParser(description="EchoMuse payload 串口推送")
    ap.add_argument("--workdir", default=str(Path.home() / "emos-install"))
    ap.add_argument("--port", help="串口设备，默认自动发现 /dev/tty.usbmodem*")
    ap.add_argument("--host", default="192.168.99.249",
                    help="控制器地址（写入 controller.json）")
    ap.add_argument("--ctrl-port", type=int, default=80, help="明文 ws 端口")
    ap.add_argument("--tls-port", type=int, default=0,
                    help="TLS 端口，0=明文拨号（家庭环境默认）")
    ap.add_argument("--local-server", help="使用本地固件文件，跳过 GitHub 下载")
    ap.add_argument("--no-proxy", action="store_true",
                    help="GitHub 下载不走 tvv.tw 代理，直连")
    ap.add_argument("--no-net", action="store_true",
                    help="跳过设备端 WiFi 拉取，固件直接走串口推送")
    ap.add_argument("--reboot", action="store_true", help="完成后重启设备")
    args = ap.parse_args()

    global USE_PROXY
    USE_PROXY = not args.no_proxy
    work = Path(args.workdir).expanduser()
    work.mkdir(parents=True, exist_ok=True)
    fw = download_firmware(work, args.local_server)
    script = fetch_github(START_SCRIPT_URL)
    eps = controller_json(args.host, args.ctrl_port, args.tls_port)
    log(f"controller.json -> {args.host}:{args.ctrl_port} "
        f"(tls_port={args.tls_port})")

    con = Console(args.port or find_console_port())
    try:
        install(DeviceShell(con), fw, script, eps, args.reboot,
                use_net=not args.no_net)
    finally:
        try:
            con.close()
        except OSError:
            pass


if __name__ == "__main__":
    main()
