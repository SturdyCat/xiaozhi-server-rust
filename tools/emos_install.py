#!/usr/bin/env python3
"""Echo Dot 2 手动安装 emOS（不依赖 EchoMuse 官方控制器/docker）。

按 EchoMuse emos/README 的 "Build and install" + "Flashing safely" 实现，
每步落盘状态（state JSON），失败后重跑自动从断点继续。

用法:
  python3 tools/emos_install.py                 # 从断点继续到完成
  python3 tools/emos_install.py --status        # 只看当前进度
  python3 tools/emos_install.py --reset pull    # 作废某步及之后的断点
  python3 tools/emos_install.py --workdir DIR   # 镜像/状态目录，默认 ~/emos-install
  python3 tools/emos_install.py --wifi          # 串口配网：停下来询问 WiFi 的
                                                # SSID/密码，再经串口同步给设备

前置: Dot 已用 firebreak 解锁并在 TWRP（adb devices 可见，无授权弹窗）。
"""
from __future__ import annotations

import argparse
import getpass
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import termios
import time
from pathlib import Path

BOOT_PART = "/dev/block/mmcblk0p10"
BOOT_SIZE = 32768 * 512  # 16MB
ECHOMUSE_REPO = "https://github.com/wilbowes/EchoMuse.git"
STEPS = ["prereq", "pull", "build", "flash", "boot"]


def log(msg: str) -> None:
    print(f"[emos-install] {msg}", flush=True)


def die(msg: str, hint: str = "") -> None:
    print(f"[emos-install] 失败: {msg}", file=sys.stderr)
    if hint:
        print(f"  提示: {hint}", file=sys.stderr)
    sys.exit(1)


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    log("$ " + " ".join(cmd))
    return subprocess.run(cmd, text=True, capture_output=True, **kw)


def adb(args: str, binary: bool = False):
    cmd = ["adb", "exec-out", args] if binary else ["adb", "shell", args]
    return subprocess.run(cmd, capture_output=True, text=not binary)


def md5_file(p: Path) -> str:
    h = hashlib.md5()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


class State:
    def __init__(self, path: Path):
        self.path = path
        self.data = json.loads(path.read_text()) if path.exists() else {}

    def done(self, step: str) -> bool:
        return self.data.get(step, {}).get("ok") is True

    def meta(self, step: str) -> dict:
        return self.data.get(step, {})

    def mark(self, step: str, **meta) -> None:
        self.data[step] = {"ok": True, **meta}
        self.save()

    def reset(self, step: str) -> None:
        for s in STEPS[STEPS.index(step):]:
            self.data.pop(s, None)
        self.save()

    def save(self) -> None:
        self.path.write_text(json.dumps(self.data, indent=2, ensure_ascii=False))


# ---------- steps ----------

def adb_state() -> str:
    st = subprocess.run(["adb", "get-state"], text=True, capture_output=True)
    return (st.stdout or "").strip()


def wait_recovery(timeout: int = 45) -> None:
    """reboot recovery 后设备会掉线再以 recovery 状态回来，轮询等到位。"""
    log("等待设备进入 recovery（最多 45s）...")
    deadline = time.time() + timeout
    while time.time() < deadline:
        time.sleep(2)
        subprocess.run(["adb", "wait-for-device"], capture_output=True, timeout=10)
        if adb_state() == "recovery" and adb("getprop ro.twrp.version").stdout.strip():
            return
    die("设备未在 45s 内进入 recovery",
        "`adb devices` 看状态；卡在侧载界面则按住按钮重来，或检查是否真的解锁过")


def ensure_recovery() -> None:
    """智能切换：device 系统模式自动 reboot recovery；offline 重试；无人性阻塞。"""
    state = adb_state()
    if state == "recovery":
        return
    if state == "device":
        log("设备在系统模式，自动 reboot recovery ...")
        subprocess.run(["adb", "reboot recovery"], capture_output=True)
        wait_recovery()
        return
    if state in ("offline", "unauthorized"):
        for _ in range(10):
            time.sleep(3)
            state = adb_state()
            if state == "recovery":
                return
            if state == "device":
                return ensure_recovery()
        die(f"adb 设备状态 {state} 持续未恢复",
            "重插 USB；unauthorized 需在设备上确认调试授权弹窗")
    die(f"设备不可见（adb state={state or '空'}）",
        "adb devices 应能看到 G090 开头序列号；不识别则换线换口或重装驱动")


def step_prereq(work: Path) -> None:
    if not shutil.which("adb"):
        die("未找到 adb", "brew install --cask android-platform-tools")
    ensure_recovery()
    serial = adb("getprop ro.serialno").stdout.strip()
    if not serial:
        die("读不到序列号", "adb 未授权或设备异常，重插 USB 后检查弹窗")
    (work / "serial.txt").write_text(serial)
    log(f"设备 TWRP v{adb('getprop ro.twrp.version').stdout.strip()} serial={serial}")


def remote_md5(path: str) -> str:
    r = adb(f"md5sum {path}")
    parts = r.stdout.split()
    if r.returncode != 0 or not parts:
        die(f"设备端 md5sum {path} 失败: {r.stderr.strip()[:200]}")
    return parts[0]


def step_pull(work: Path) -> None:
    """读出 boot 分区：既是构建输入，也是 10 秒恢复镜像。

    本机 TWRP adbd 的 exec-out 流会被 pty 破坏（实测 64KiB 内多出 91 字节），
    必须走设备端 dd 落盘 + adb pull 二进制安全通道，并用三重 md5 验证：
    分区直接 md5 == /tmp 文件 md5（eMMC 读取稳定）== 本地文件 md5（传输无损）。
    """
    img = work / "boot_a_x.img"
    tmp = "/tmp/emos-boot-read.img"
    old = img.with_suffix(".img.bak")
    if img.exists():
        img.rename(old)
        log(f"旧备份已轮换: {old.name}")
    if adb(f"dd if={BOOT_PART} of={tmp} bs=4096").returncode != 0:
        die("设备端 dd 读取 boot 分区失败", "确认 TWRP 正常挂载；重跑重试")
    h_part = remote_md5(BOOT_PART)
    h_tmp = remote_md5(tmp)
    if h_part != h_tmp:
        adb(f"rm -f {tmp}")
        die("两次读取 boot 分区 md5 不一致，eMMC 读取不稳定，先排查设备再继续")
    p = subprocess.run(["adb", "pull", tmp, str(img)], capture_output=True, text=True)
    adb(f"rm -f {tmp}")
    if p.returncode != 0 or img.stat().st_size != BOOT_SIZE:
        die(f"adb pull 拉取失败（本地 {img.stat().st_size if img.exists() else 0} 字节）",
            "重跑即可；多次失败换线/换口，确认是数据线不是充电线")
    h_local = md5_file(img)
    if h_local != h_tmp:
        die(f"传输 md5 不匹配: 本地 {h_local}, 设备 {h_tmp}", "重跑本步重试")
    log(f"boot 备份完成 md5={h_local}（此文件即恢复镜像，务必另外复制一份保存）")


def latest_emos_tag(repo: Path) -> str:
    run(["git", "-C", str(repo), "fetch", "--tags", "origin"])
    r = run(["git", "-C", str(repo), "describe", "--tags", "--match", "emos-v*",
             "origin/main"])
    tag = r.stdout.strip()
    if not tag:
        die("远端没有任何 emos-v* 标签")
    return tag


def step_build(work: Path) -> None:
    """构建 emos-boot.img：复用你自己 boot 里的内核+DTB，拼上 emOS init。"""
    repo = work / "EchoMuse"
    if not repo.exists():
        run(["git", "clone", ECHOMUSE_REPO, str(repo)])
        if run(["git", "-C", str(repo), "rev-parse", "HEAD"]).returncode != 0:
            die("克隆 EchoMuse 失败", "检查网络；国内可设 https_proxy")
    tag = latest_emos_tag(repo)
    run(["git", "-C", str(repo), "checkout", tag])
    log(f"EchoMuse @ {tag}")
    img = work / "boot_a_x.img"
    if not img.exists():
        die("缺少参考镜像 boot_a_x.img", "--reset pull 重新从设备读取")
    out = work / "emos-boot.img"
    out.unlink(missing_ok=True)
    # build.sh 在 emos/ 子目录；编译 init 需要 NDK 交叉工具链，找不到时它会
    # 尝试 docker run echomuse-compiler 镜像，本地两者都没有则必然失败——
    # 提前探测给出明确指引，而不是等 build.sh 跑一半报错
    env = dict(os.environ)
    # NDK 未显式指定时自动发现：任意版本均可——build.sh 只需要 API 21 的
    # clang wrapper（aarch64-linux-android21-clang），各版本 NDK 都自带
    if not env.get("NDK"):
        sdk_ndk = Path.home() / "Library/Android/sdk/ndk"
        host = "darwin-x86_64" if sys.platform == "darwin" else "linux-x86_64"
        candidates = sorted(sdk_ndk.glob(f"*/toolchains/llvm/prebuilt/{host}/bin"),
                            reverse=True)
        if candidates:
            env["NDK"] = str(candidates[0])
            log(f"自动发现 NDK: {env['NDK']}")
        elif shutil.which("docker") is None:
            die("构建链不可用：未找到 NDK（$NDK 或 ~/Library/Android/sdk/ndk/）"
                "，也无 docker",
                "任选其一：① 安装任意版本 NDK（Android Studio SDK Manager 或 "
                "sdkmanager 'ndk;30.0.16138531'）后重跑；② docker build -t "
                "echomuse-compiler 按 EchoMuse CONTRIBUTING 构建镜像后重跑（从此步续跑）")
    # a 槽的 boot 与 system 槽位对应关系由设备分区表决定；官方向导会读取，
    # 手动构建可用 EMOS_SYSTEM_PART 指定（不设则 emOS 回退到硬编码分区）
    r = run(["./build.sh", str(img), str(out)], cwd=str(repo / "emos"), env=env)
    if r.returncode != 0 or not out.exists():
        (work / "build.log").write_text((r.stdout or "") + "\n" + (r.stderr or ""))
        die("build.sh 构建失败，详见 build.log",
            "多数是 NDK/Docker 构建链问题，装好后重跑（会从此步续跑）")
    log(f"构建完成: {out} ({out.stat().st_size} 字节)")


def step_flash(work: Path) -> None:
    """安全写入：busybox dd conv=fsync + drop_caches 后回读 md5 比对。"""
    img = work / "emos-boot.img"
    if not img.exists():
        die("emos-boot.img 不存在", "断点被破坏，--reset build 重跑构建")
    local_md5 = md5_file(img)
    if img.stat().st_size > BOOT_SIZE:
        die(f"镜像大于分区（{img.stat().st_size} > {BOOT_SIZE}）", "镜像构建异常，--reset build 重跑")
    # 回读校验是 md5 整个分区：镜像必须先补零到分区大小，否则必然误报不匹配
    padded = work / "emos-boot.padded.img"
    with open(img, "rb") as f, open(padded, "wb") as g:
        g.write(f.read())
        while g.tell() < BOOT_SIZE:
            g.write(b"\x00" * (1 << 20))
        g.truncate(BOOT_SIZE)  # 1MB 块写入可能超出，精确截到分区大小
    local_md5 = md5_file(padded)
    img = padded
    img_confirmed = work / "boot_a_x.img"
    if not img_confirmed.exists():
        die("缺少 boot_a_x.img 恢复镜像备份", "绝对不能在没有备份的情况下刷写；--reset pull 重读")

    push = run(["adb", "push", str(img), "/tmp/emos-boot.img"])
    if push.returncode != 0:
        die("推送镜像到设备失败")
    # Toolbox dd 拒绝 conv=fsync，部分 TWRP 也没有 busybox：先探测写入器，
    # 没有 busybox 就用裸 dd（去掉 conv），落盘由下方显式 sync + 回读 md5 保证
    if adb("command -v busybox").stdout.strip():
        dd_cmd = ("busybox dd if=/tmp/emos-boot.img of=" + BOOT_PART +
                  " bs=2048 conv=fsync")
        log("设备端写入器: busybox")
    else:
        dd_cmd = "dd if=/tmp/emos-boot.img of=" + BOOT_PART + " bs=2048"
        log("设备端写入器: dd（无 busybox/conv=fsync，改用显式 sync 兜底）")
    w = adb(dd_cmd)
    if w.returncode != 0 or "error" in (w.stderr or "").lower():
        die(f"写入失败: {(w.stderr or '').strip()[:300]}")
    adb("sync")
    adb("echo 3 > /proc/sys/vm/drop_caches")
    # 关键：drop_caches 之后回读，避免页缓存假成功
    rb = adb(f"md5sum {BOOT_PART}", binary=False)
    remote_hash = rb.stdout.split()[0] if rb.stdout.split() else ""
    if remote_hash != local_md5:
        die(f"回读 md5 不匹配: 本地 {local_md5}, 设备端输出: {rb.stdout.strip()[:200]!r}",
            "不要重启设备！重跑本步（--reset flash 后重跑）或用 boot_a_x.img 恢复")
    log(f"写入并校验通过 md5={local_md5}")


def step_boot(work: Path) -> None:
    adb("reboot")
    log("已发送 reboot，冷启动到联网约 35 秒。")
    log("确认：server 日志流（ws /api/logs）出现 echomuse register 即接入成功；"
        "也可 `dns-sd -B _emcontroller._tcp` 看到设备广播。")
    log("若未连上 WiFi：USB 连电脑后运行 `python3 tools/emos_install.py --wifi`，"
        "会停下来询问 SSID/密码，再经串口同步给设备（em-wifi）。")


# ---------- 配网：询问用户，经 USB 串口同步给设备 ----------

def prompt_wifi_config() -> tuple[str, str]:
    """停住询问 SSID 与密码；密码不回显，只在内存传递、落盘仅设备侧 wpa.conf。"""
    ssid = ""
    while not ssid:
        ssid = input("WiFi SSID: ").strip()
        if not ssid:
            print("SSID 不能为空，请重新输入")
    psk = getpass.getpass(f"WiFi 密码（{ssid}）: ")
    return ssid, psk


class Console:
    """emOS USB 串口控制台：raw 模式（不关 ECHO 会被自己的回显执行，返回 127）。

    参照 EchoMuse emos/tools/emconsole.py，macOS 用 /dev/tty.usbmodem*，
    pyserial 不是必需——直接 termios 置 raw。
    """

    def __init__(self, port: str):
        self.fd = os.open(port, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
        attrs = termios.tcgetattr(self.fd)
        iflag, oflag, cflag, lflag, ispeed, ospeed, cc = attrs
        iflag &= ~(termios.IGNBRK | termios.BRKINT | termios.PARMRK
                   | termios.ISTRIP | termios.INLCR | termios.IGNCR
                   | termios.ICRNL | termios.IXON)
        oflag &= ~termios.OPOST
        lflag &= ~(termios.ECHO | termios.ECHONL | termios.ICANON
                   | termios.ISIG | termios.IEXTEN)
        cflag &= ~(termios.CSIZE | termios.PARENB)
        cflag |= termios.CS8 | termios.CLOCAL | termios.CREAD
        cc[termios.VMIN], cc[termios.VTIME] = 0, 0
        # 直接覆盖为 raw（速率沿用 attrs 里驱动设置的默认值）
        termios.tcsetattr(self.fd, termios.TCSANOW,
                          [iflag, oflag, cflag, lflag, ispeed, ospeed, cc])
        self.buf = ""

    def send(self, s: str) -> None:
        os.write(self.fd, s.encode())

    def drain(self, secs: float) -> str:
        deadline = time.time() + secs
        out = ""
        while time.time() < deadline:
            try:
                b = os.read(self.fd, 4096)
                if b:
                    out += b.decode("utf-8", "replace")
                    deadline = time.time() + secs  # 有数据就续窗
                    continue
            except BlockingIOError:
                pass
            time.sleep(0.05)
        self.buf += out
        return out

    def expect(self, needle: str, timeout: float) -> str:
        deadline = time.time() + timeout
        while time.time() < deadline:
            i = self.buf.find(needle)
            if i >= 0:
                out = self.buf[:i + len(needle)]
                self.buf = self.buf[i + len(needle):]
                return out
            self.drain(0.3)
        die(f"串口 60s 内没等到 {needle!r}",
            f"缓冲区尾部: {self.buf[-300:]!r}；确认设备已进 emOS（串口在、adb 无）")

    def close(self) -> None:
        os.close(self.fd)


def find_console_port() -> str:
    ports = sorted(Path("/dev").glob("tty.usbmodem*"))
    if len(ports) == 1:
        return str(ports[0])
    if not ports:
        die("未发现 USB 串口（/dev/tty.usbmodem*）",
            "确认 Dot 插着 USB 且已启动 emOS；macOS 首次接入需在弹窗里点允许")
    die(f"发现多个串口: {[p.name for p in ports]}，请用 --port 指定")


def step_wifi(work: Path, ssid: str, psk: str, port: str) -> None:
    """驱动 em-wifi：扫描 → 按 SSID 选号 → 输密码 → 等加入结果。"""
    if not psk and ssid:
        log("无密码，按开放网络处理")
    con = Console(port or find_console_port())
    try:
        con.drain(1.0)
        con.send("\n")
        con.drain(0.5)
        con.send("em-wifi\n")
        menu = con.expect("Number (or q to quit): ", timeout=40)
        # 菜单行形如 " 3) MyHome  -52 dBm"；SSID 文本在 ') ' 与 ' dBm' 之间
        entries = re.findall(r"^\s*(\d+)\)\s+(.*?)\s+-?\d+\s*dBm", menu, re.M)
        num = next((n for n, name in entries if name.strip() == ssid), "")
        if not num:
            die(f"设备扫描结果里没有 {ssid!r}",
                f"可见网络: {[n for _, n in entries]}；隐藏 SSID 请改名广播或换网")
        con.send(num + "\n")
        # psk 网会问密码，open 网直接开始加入——两种都等着看结果
        pw_prompt = con.drain(3.0)
        if "Password for" in pw_prompt or "Password for" in con.buf:
            con.expect("Password for", timeout=5)
            con.send(psk + "\n")
        log("已提交，等待设备加入（em-wifi 会自验证）...")
        deadline = time.time() + 90
        result = ""
        while time.time() < deadline:
            chunk = con.drain(2.0)
            result += chunk
            joined = ("connected" in result.lower()
                      or "associated" in result.lower()
                      or "join" in result.lower())
            failed = ("fail" in result.lower() or "timeout" in result.lower()
                      or "error" in result.lower()
                      or re.search(r"^\[?emos?[^ ]* [#>] ", result, re.M))
            if joined:
                log("设备已加入 WiFi ✔")
                break
            if failed:
                die(f"em-wifi 报告失败，输出尾部: {result[-400:]!r}",
                    "核对密码；路由器必须 WPA2（emOS 驱动不支持 WPA3/WEP）")
        else:
            die(f"90s 未观察到加入结果，输出尾部: {result[-400:]!r}",
                "重跑 --wifi 重试；或手动 screen 上去跑 em-wifi 看现场")
        # 配网成功后立即复用同一串口做状态探测并打印最终汇总
        try:
            print_summary(run_probes(con))
        except SystemExit:
            raise
        except Exception as exc:  # 探测失败不影响配网成功结论
            log(f"配网成功，但状态汇总失败: {exc}")
        log("确认入网: `dns-sd -B _emcontroller._tcp` 应出现设备广播，"
            "server 日志流出现 echomuse register。")
    finally:
        con.close()




PROBES = [
    ("系统", "uname -a; uptime"),
    ("emOS 版本", "em-info 2>/dev/null; cat /etc/os-release /etc/version "
                  "/data/emos/version 2>/dev/null; ls /data/emos 2>/dev/null"),
    ("WiFi 配置", "ls -l /data/emos/wpa.conf 2>/dev/null "
                  "&& echo '(wpa.conf 存在=已配置)' || echo '(未配网)'; "
                  "grep '^ssid=' /data/emos/wpa.conf 2>/dev/null "
                  "|| echo '(无 ssid 行)'"),
    ("网络接口", "ip -o addr show 2>/dev/null || ifconfig -a 2>/dev/null"),
    ("无线状态", "cat /proc/net/wireless 2>/dev/null; "
                 "iw dev wlan0 link 2>/dev/null || iwconfig 2>/dev/null"),
    ("蓝牙", "hciconfig -a 2>/dev/null || ls /sys/class/bluetooth 2>/dev/null "
             "|| echo '(无蓝牙控制接口——emOS 正常现象)'"),
    ("进程", "ps | grep -v grep | grep -E 'wpa|emos' 2>/dev/null || true"),
]


def run_probes(con: "Console") -> dict[str, str]:
    """在已打开的控制台上跑全部探测命令，返回 标题→原始输出。

    设备端 shell 会回显输入，故 marker 用字符串拼接（__EO""C__）避免命中回显。
    """
    con.drain(1.0)
    con.send("\n")
    con.drain(0.5)
    con.buf = ""
    results: dict[str, str] = {}
    for title, cmd in PROBES:
        con.send(cmd + '; echo "__EO""C__"\n')
        results[title] = con.expect("__EOC__", timeout=20).strip()
    return results


def print_summary(results: dict[str, str]) -> None:
    """从探测输出提取关键字段并打印判定汇总。"""
    # 汇总判定（只输出精简结果；设备会回显命令，故取值时剔除回显行）
    def body_of(title: str) -> list[str]:
        # 回显行 = 原命令文本，包含探测命令特征词，剔除后才是真实输出
        # 剔除回显的命令行：设备会原样回显整条命令，其中含探测关键词与
        # 换行回绕碎片（如 "echo __EO C__<"），统一按特征词过滤
        return [ln for ln in results.get(title, "").splitlines()
                if ln.strip() and not re.search(
                    r"os-release|/proc/net|hciconfig|sys/class|grep -v grep|"
                    r"ip -o addr|ifconfig -a|iw dev|iwconfig|em-info|em-wifi|"
                    r"/data/emos|wpa\.conf|ssid=|__EO|__EC|__E.?O.?C|2>/dev/null|"
                    r"2>/dev/null|ls -l|grep \^|&&|echo|version|ls /data", ln)]

    all_out = "\n".join(results.values())
    ip = re.search(r"inet (\d+\.\d+\.\d+\.\d+)", all_out)
    mac = re.search(r"wlan0.*?link/ether ([0-9a-f:]{17})", all_out)
    host = re.search(r"Linux (\S+)", all_out)
    level = re.search(r"wlan0:\s*\d+\s*\d+\.\s*(\d+)\.", all_out)
    wifi_lines = body_of("WiFi 配置")
    ver_lines = body_of("emOS 版本")
    # SSID：只有真实取到值才显示（wpa.conf 路径/格式在不同 emOS 版本不一致，
    # 取不到就整个不打印该行，避免 "(未配置)" 与实际已联网状态矛盾）。
    # 从原始输出取：行首即 ssid= 且不含命令特征词（回显行含 grep/echo 等）
    ssid = next((m.group(1).strip().strip('"\'')
                 for ln in results.get("WiFi 配置", "").splitlines()
                 if (m := re.match(r'\s*ssid=(.+)$', ln))
                 and not re.search(r"grep|echo|wpa\.conf|2>/dev/null|\|\|", ln)),
                None)
    version = next((ln.strip() for ln in ver_lines
                    if re.search(r"v?\d+\.\d+|emos", ln, re.I)), "?")
    # 已配网判定以"拿到 IP"为准（wpa.conf 路径在不同 emOS 版本不一致，不可靠）
    wifi_cfg = bool(ip)
    print("\n══ 判定汇总 ══")
    print(f"主机名:      {host.group(1) if host else '?'}")
    print(f"系统版本:    {version}")
    print(f"IP (wlan0):  {ip.group(1) if ip else '未获取'}")
    print(f"MAC (wlan0): {mac.group(1) if mac else '?'}")
    print(f"信号 level:  {level.group(1) if level else '?'} (/proc/net/wireless，非负值，越小越好)")
    print(f"已配网:      {'是' if wifi_cfg else '否'}")
    if ssid:
        print(f"SSID:        {ssid}")


def step_probe(port: str) -> None:
    """串口探针：确认 emOS 运行状态、网络/IP/MAC、无线与蓝牙线索。"""
    con = Console(port or find_console_port())
    try:
        results = run_probes(con)
    finally:
        try:
            con.close()
        except OSError:
            pass
    print_summary(results)


def main() -> None:
    ap = argparse.ArgumentParser(description="Echo Dot 2 emOS 手动安装（断点续跑）")
    ap.add_argument("--workdir", default=str(Path.home() / "emos-install"))
    ap.add_argument("--status", action="store_true")
    ap.add_argument("--reset", choices=STEPS, help="作废该步及之后的所有断点")
    ap.add_argument("--probe", action="store_true",
                    help="只做串口状态探测：emOS 版本/网络/IP/MAC/蓝牙")
    ap.add_argument("--wifi", action="store_true",
                    help="不跑安装流水线，只做串口配网（询问 WiFi 配置）")
    ap.add_argument("--ssid", help="非交互指定 WiFi 名（跳过询问 SSID）")
    ap.add_argument("--wifi-password", help="非交互指定 WiFi 密码（跳过询问密码）")
    ap.add_argument("--port", help="串口设备，默认自动发现 /dev/tty.usbmodem*")
    args = ap.parse_args()

    work = Path(args.workdir).expanduser()
    work.mkdir(parents=True, exist_ok=True)

    if args.probe:
        step_probe(args.port)
        return

    if args.wifi:
        ssid = args.ssid or ""
        psk = args.wifi_password or ""
        if not ssid or not psk:
            prompt = prompt_wifi_config()
            ssid = ssid or prompt[0]
            psk = psk or prompt[1]
        log(f"配网目标 SSID: {ssid!r} port={args.port or '自动'}")
        step_wifi(work, ssid, psk, args.port)
        return

    state = State(work / "state.json")

    if args.reset:
        state.reset(args.reset)
        log(f"已重置 {args.reset} 及之后的断点")

    if args.status:
        for s in STEPS:
            m = state.meta(s)
            print(f"  {'✔' if state.done(s) else '·'} {s}"
                  + (f"  {json.dumps(m, ensure_ascii=False)}" if m else ""))
        return

    runners = {"prereq": step_prereq, "pull": step_pull, "build": step_build,
               "flash": step_flash, "boot": step_boot}
    for s in STEPS:
        if state.done(s):
            log(f"跳过 {s}（断点已完成）")
            continue
        log(f"== {s} ==")
        runners[s](work)
        state.mark(s, at=int(time.time()))
    log("全部完成。设备恢复：TWRP 中 dd 回 boot_a_x.img 约十秒；"
        "emOS 下可由控制台 /init recovery 进 TWRP。")


if __name__ == "__main__":
    main()
