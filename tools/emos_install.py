#!/usr/bin/env python3
"""Echo Dot 2 手动安装 emOS（不依赖 EchoMuse 官方控制器/docker）。

按 EchoMuse emos/README 的 "Build and install" + "Flashing safely" 实现，
每步落盘状态（state JSON），失败后重跑自动从断点继续。

用法:
  python3 tools/emos_install.py                 # 从断点继续到完成
  python3 tools/emos_install.py --status        # 只看当前进度
  python3 tools/emos_install.py --reset pull    # 作废某步及之后的断点
  python3 tools/emos_install.py --workdir DIR   # 镜像/状态目录，默认 ~/emos-install

前置: Dot 已用 firebreak 解锁并在 TWRP（adb devices 可见，无授权弹窗）。
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
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

def step_prereq(work: Path) -> None:
    if not shutil.which("adb"):
        die("未找到 adb", "brew install --cask android-platform-tools")
    r = adb("getprop ro.twrp.version")
    if r.returncode != 0 or not r.stdout.strip():
        die("设备不在 TWRP", "先跑 firebreak 解锁，解锁完成后设备停在 TWRP；确认 adb devices 授权")
    serial = adb("getprop ro.serialno").stdout.strip()
    if not serial:
        die("读不到序列号", "adb 未授权或设备异常，重插 USB 后检查弹窗")
    (work / "serial.txt").write_text(serial)
    log(f"设备 TWRP v{r.stdout.strip()} serial={serial}")


def step_pull(work: Path) -> None:
    """读出 boot 分区：既是构建输入，也是 10 秒恢复镜像。"""
    img = work / "boot_a_x.img"
    if img.exists():
        img.rename(img.with_suffix(".img.bak"))
    with open(img, "wb") as f:
        p = subprocess.run(["adb", "exec-out", f"dd if={BOOT_PART} bs=4096"], stdout=f)
    if p.returncode != 0 or img.stat().st_size != BOOT_SIZE:
        die(f"读取 boot 分区失败（{img.stat().st_size if img.exists() else 0} 字节）",
            "重跑即可；多次失败换线/换口，确认是数据线不是充电线")
    # 回读校验：两次读取哈希一致才可信
    h1 = md5_file(img)
    check = work / "boot_readback.img"
    with open(check, "wb") as f:
        subprocess.run(["adb", "exec-out", f"dd if={BOOT_PART} bs=4096"], stdout=f)
    h2 = md5_file(check)
    if h1 != h2 or check.stat().st_size != BOOT_SIZE:
        die("两次读取 boot 分区不一致，eMMC 读取不可靠，先排查线缆再继续")
    check.unlink()
    log(f"boot 备份完成 md5={h1}（此文件即恢复镜像，务必另外复制一份保存）")


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
    out = work / "emos-boot.img"
    out.unlink(missing_ok=True)
    # 官方 build.sh 可能需要 NDK/Docker，失败时输出其错误供排查
    r = run(["./build.sh", str(img), str(out)], cwd=str(repo))
    if r.returncode != 0 or not out.exists():
        (work / "build.log").write_text(r.stdout + "\n" + r.stderr)
        die("build.sh 构建失败，详见 build.log",
            "多数是缺 NDK/Docker，按 EchoMuse 仓库 CONTRIBUTING 装好构建链后重跑（会从此步续跑）")
    log(f"构建完成: {out} ({out.stat().st_size} 字节)")


def step_flash(work: Path) -> None:
    """安全写入：busybox dd conv=fsync + drop_caches 后回读 md5 比对。"""
    img = work / "emos-boot.img"
    if not img.exists():
        die("emos-boot.img 不存在", "断点被破坏，--reset build 重跑构建")
    local_md5 = md5_file(img)
    img_confirmed = work / "boot_a_x.img"
    if not img_confirmed.exists():
        die("缺少 boot_a_x.img 恢复镜像备份", "绝对不能在没有备份的情况下刷写；--reset pull 重读")

    push = run(["adb", "push", str(img), "/tmp/emos-boot.img"])
    if push.returncode != 0:
        die("推送镜像到设备失败")
    # Toolbox dd 拒绝 conv=fsync，优先 busybox
    w = adb("sh -c 'busybox dd if=/tmp/emos-boot.img of=" + BOOT_PART +
            " bs=2048 conv=fsync || dd if=/tmp/emos-boot.img of=" + BOOT_PART +
            " bs=2048 conv=fsync'")
    if "error" in (w.stderr or "").lower() or w.returncode != 0:
        die(f"写入失败: {w.stderr.strip()[:300]}")
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
    log("若未连上 WiFi：USB 连电脑，以 raw 模式开串口"
        "（screen 不可靠，用 `stty -f /dev/tty.usb* raw -echo` + cat，或 pyserial ser.setraw()），"
        "在控制台运行 em-wifi 选网配网；拒绝 WPA3/WEP。")


# ---------- main ----------

def main() -> None:
    ap = argparse.ArgumentParser(description="Echo Dot 2 emOS 手动安装（断点续跑）")
    ap.add_argument("--workdir", default=str(Path.home() / "emos-install"))
    ap.add_argument("--status", action="store_true")
    ap.add_argument("--reset", choices=STEPS, help="作废该步及之后的所有断点")
    args = ap.parse_args()

    work = Path(args.workdir).expanduser()
    work.mkdir(parents=True, exist_ok=True)
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
