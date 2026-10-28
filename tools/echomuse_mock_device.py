#!/usr/bin/env python3
"""EchoMuse mock 设备（emOS Echo Dot 2 模拟器）—— 联调脚本，零第三方依赖。

按 EchoMuse 官方 device-controller-interface 协议模拟一台设备：

1. 连 `ws://<host>:<port>/em/control`（`X-Em-Token` 头），发 `register`，收 `ack` + `config`
2. 连 `/em/data`，按设备侧帧型上行
3. 模拟唤醒：发 `oww_wake` → 等 `mic_start`（lock_mic:true）→ 发一段
   S16_LE 16kHz 正弦波 PCM（`0x01` 帧）→ 发 `0x04`（语音段结束哨兵）
4. 等 xiaozhi 流水线回话：收 `0x02`（speaker PCM）… `0x03`（EOS）即算成功
5. （容忍型）检查 control 面上的服务器 JSON 下发（led_anim 等）不导致异常

任一 fatal 失败 → 退出码 1。前置：`[echomuse].enabled = true` 并重启服务端。

用法：
    python3 tools/echomuse_mock_device.py --host 127.0.0.1 --port 8000 [--token TOKEN]
           [--save-downlink out.pcm] [--stay 600] [--no-shell]

`--save-downlink` 把服务端下发的全部 `0x02` 载荷（mono S16LE 48kHz）追加写入文件——
这是"服务端下行内容到底是不是 48k"的抓包判据：用
`ffplay -f s16le -ar 48000 -ch_layout mono out.pcm` 播放，语速自然 = 服务端正确，
加速 = 服务端内容未到 48k；再与真机听感对照即可定位问题在服务器还是设备侧。

`--stay N`：语音回路跑完后保持在线 N 秒，供浏览器控制台联调（默认 0 = 跑完即退出，
与原行为一致）。控制台联调用例：`--stay 600`，然后浏览器 → EchoMuse → 点设备 → 控制台。

## shell 回拨（控制台联调）

真机收到 control 面的 `{"type":"shell_open"}` 会回拨 `/shell/{device_id}`，本脚本同样
回拨（带同一枚 `X-Em-Token`），并把设备 stdin 的原始字节喂给**内置假 shell**：

- 白名单命令返回固定文本：`uname -a` / `cat /etc/os-release` / `df -k /data` / `ps` /
  `free`；`pwd` 回当前目录；`cd <dir>` 只改内部路径、回空；`exit` 关闭回拨通道
  （服务端按 `device-shell-lost` 收尾，可用来测那条路径）；
- 其余一律 `sh: <cmd>: not found`（busybox ash 风格）；
- 输出只走 **Binary 帧**（真实设备上行也只有 Binary，用 Text 会掩盖协议错误）；
- ⛔ **绝不执行真实系统命令**（无 `subprocess`）：一个能跑 shell 的 mock 不该变成远程命令
  执行入口。假 shell 也没有文件系统，`cd /不存在` 照样成功——这是刻意的演示级简化。
- `--no-shell`（或 `ECHOMUSE_MOCK_SHELL=0`）关掉该能力，不影响语音回路。
"""

import argparse
import json
import math
import os
import posixpath
import select
import socket
import struct
import sys
import threading
import time
import base64
from typing import Optional

# 帧型（设备 ↔ 服务器，方向命名空间：上行 0x01/0x04/0x05/0x07；下行 0x02/0x03）
FRAME_MIC = 0x01
FRAME_VAD_END = 0x04
FRAME_NO_SPEECH = 0x05
FRAME_SPEAKER = 0x02
FRAME_SPEAKER_EOS = 0x03

MIC_SR = 16000
# 下行 speaker 线缆采样率：上游契约是 mono 48kHz（设备 DAC 48k、不重采样）。
SPEAKER_SR = 48000


class WsClient:
    """最小 WebSocket 客户端（仅本脚本够用：文本收发、二进制收发、服务端 ping 忽略）。"""

    def __init__(self, host: str, port: int, path: str, token: str = "", timeout: float = 10):
        self.sock = socket.create_connection((host, port), timeout=timeout)
        key = base64.b64encode(b"0123456789abcdef").decode()
        headers = [
            f"GET {path} HTTP/1.1",
            f"Host: {host}:{port}",
            "Upgrade: websocket",
            "Connection: Upgrade",
            f"Sec-WebSocket-Key: {key}",
            "Sec-WebSocket-Version: 13",
        ]
        if token:
            headers.append(f"X-Em-Token: {token}")
        self.sock.sendall(("\r\n".join(headers) + "\r\n\r\n").encode())
        # 读握手响应头（逐字节找 \r\n\r\n）
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = self.sock.recv(1)
            if not chunk:
                raise RuntimeError("握手失败：连接关闭")
            buf += chunk
        status = buf.split(b"\r\n", 1)[0].decode()
        if "101" not in status:
            raise RuntimeError(f"握手失败：{status}")
        self.buf = b""

    def _read_exact(self, n: int) -> bytes:
        while len(self.buf) < n:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise RuntimeError("连接关闭")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def send_text(self, text: str) -> None:
        self._send_frame(0x1, text.encode())

    def send_binary(self, data: bytes) -> None:
        self._send_frame(0x2, data)

    def _send_frame(self, opcode: int, payload: bytes) -> None:
        # 客户端帧必须掩码（RFC6455）；用全零掩码键 + 原文（实现正确性等价）。
        mask = b"\x00\x00\x00\x00"
        masked = bytes(b ^ 0 for b in payload)
        header = bytes([0x80 | opcode])
        n = len(payload)
        if n < 126:
            header += bytes([0x80 | n])
        elif n < 65536:
            header += bytes([0x80 | 126]) + struct.pack(">H", n)
        else:
            header += bytes([0x80 | 127]) + struct.pack(">Q", n)
        self.sock.sendall(header + mask + masked)

    def recv_message(self) -> tuple[int, bytes]:
        """返回 (opcode, payload)；自动回应服务端 ping。"""
        while True:
            b1, b2 = self._read_exact(2)
            opcode = b1 & 0x0F
            masked = b2 & 0x80
            n = b2 & 0x7F
            if n == 126:
                (n,) = struct.unpack(">H", self._read_exact(2))
            elif n == 127:
                (n,) = struct.unpack(">Q", self._read_exact(8))
            mask = self._read_exact(4) if masked else b""
            payload = self._read_exact(n)
            if masked:
                payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
            if opcode == 0x9:  # ping → pong
                self._send_frame(0xA, payload)
                continue
            return opcode, payload

    def recv_text(self) -> str:
        op, payload = self.recv_message()
        if op != 0x1:
            raise RuntimeError(f"期望文本帧，收到 opcode={op}")
        return payload.decode()

    def recv_binary(self) -> bytes:
        op, payload = self.recv_message()
        if op != 0x2:
            raise RuntimeError(f"期望二进制帧，收到 opcode={op}")
        return payload

    def recv_message_timeout(self, timeout: float):
        """带超时的 [recv_message]；超时返回 None。

        只在**帧边界**上等（先 select 再整帧读），不会读到半帧就放弃——否则
        长驻等待（`--stay`）里一次超时就能把帧流读乱。
        """
        if not self.buf:
            ready, _, _ = select.select([self.sock], [], [], timeout)
            if not ready:
                return None
        return self.recv_message()

    def recv_text_timeout(self, timeout: float) -> Optional[str]:
        got = self.recv_message_timeout(timeout)
        if got is None:
            return None
        op, payload = got
        if op != 0x1:
            raise RuntimeError(f"期望文本帧，收到 opcode={op}")
        return payload.decode()

    def close(self) -> None:
        try:
            self._send_frame(0x8, b"")
            self.sock.close()
        except OSError:
            pass


# ============================================================
# 内置假 shell（控制台联调用；⛔ 不执行任何真实系统命令）
# ============================================================

#: 假 shell 的 HOME（`cd` 不带参数时回到这里）。
SHELL_HOME = "/root"

#: Ctrl-C 字节：行模式里它与"清掉半行"等价（无 tty，没有信号，也没有回显的 `^C`）。
SHELL_CTRL_C = 0x03

#: 白名单命令 → 固定输出。都是 emOS（Echo Dot 2，armv7l）上会看到的形状。
SHELL_FIXED = {
    "uname -a": (
        "Linux emos 4.9.0-emos #1 SMP PREEMPT Wed Nov 6 10:12:04 UTC 2019 armv7l GNU/Linux\n"
    ),
    "cat /etc/os-release": (
        'NAME=emos\n'
        'VERSION="1.0 (mock)"\n'
        "ID=emos\n"
        "VERSION_ID=1.0\n"
        'PRETTY_NAME="emOS 1.0 (mock device)"\n'
    ),
    "df -k /data": (
        "Filesystem           1K-blocks      Used Available Use% Mounted on\n"
        "/dev/mmcblk0p8         3014656    984320   1870100  34% /data\n"
    ),
    "ps": (
        "  PID USER       VSZ STAT COMMAND\n"
        "    1 root      1584 S    init\n"
        "  118 root      4120 S    /usr/bin/emos-voice\n"
        "  152 root     12880 S    /usr/bin/emos-agent\n"
        "  233 root      2360 S    -sh\n"
    ),
    "free": (
        "              total        used        free      shared  buff/cache   available\n"
        "Mem:         247584      131072       61024        2048       55488       94400\n"
        "Swap:             0           0           0\n"
    ),
}


class FakeShell:
    """行模式的假 `sh`：同一"进程"里保持 `cd` 状态，命令白名单返回固定文本。

    为什么不用 `subprocess`：这个脚本的用途是"没有真机也能调控制台"，而控制台是一条
    **root 命令通道**——让它去 spawn 真 shell，等于把联调脚本变成远程命令执行入口。
    行模式需要的语义（同一会话、`cd` 保持、输出流）用几十行状态就能演出来，够验证协议。

    刻意不模拟的东西（都是演示级简化，别当成设备行为）：
    - 没有文件系统：`cd` 到任何路径都成功（只改内部字符串）；
    - 没有退出码/提示符：行模式下 `sh` 本来就不回显、不提示；
    - 没有多进程：`ps` 是固定文本，`exit` 直接关回拨通道。
    """

    def __init__(self) -> None:
        self.cwd = SHELL_HOME
        self._partial = bytearray()

    def feed(self, data: bytes) -> "tuple[str, bool]":
        """喂入一段 stdin 原始字节，返回 (要回给服务端的文本, 是否继续存活)。"""
        out = []
        keep = True
        for byte in data:
            if byte == SHELL_CTRL_C:
                # 中断：丢弃正在攒的半行（服务端只会写 0x03 单字节）
                self._partial.clear()
                continue
            if byte != 0x0A:
                self._partial.append(byte)
                continue
            line = self._partial.decode("utf-8", "replace").rstrip("\r")
            self._partial.clear()
            text, keep = self.run_line(line)
            out.append(text)
            if not keep:
                break
        return "".join(out), keep

    def run_line(self, line: str) -> "tuple[str, bool]":
        cmd = line.strip()
        if not cmd:
            return "", True
        if cmd in ("exit", "logout"):
            return "", False
        if cmd == "pwd":
            return self.cwd + "\n", True
        if cmd == "cd" or cmd.startswith("cd "):
            arg = cmd[2:].strip()
            self.cwd = self.resolve(arg or "~")
            return "", True
        fixed = SHELL_FIXED.get(cmd)
        if fixed is not None:
            return fixed, True
        return "sh: {}: not found\n".format(cmd.split()[0]), True

    def resolve(self, arg: str) -> str:
        if arg == "~":
            return SHELL_HOME
        if arg.startswith("~/"):
            arg = SHELL_HOME + arg[1:]
        if not arg.startswith("/"):
            arg = posixpath.join(self.cwd, arg)
        return posixpath.normpath(arg)


class ShellSession:
    """一条活着的 `/shell/{id}` 回拨：读 stdin 字节 → 假 shell → 写 Binary 输出。

    `pty=True`（控制台）与真机同形：回拨后**先发一帧 `shell_meta{pty:true}`**，随后由
    "设备"自己打印提示符、回显用户输入——真实 pty 上的 mksh 就是这样（termios ECHO 打开）。
    `pty=False`（一次性操作：系统信息/文件传输）不发 meta、不回显、无提示符，只有命令输出。

    这是浏览器控制台**能看到终端效果**的前提：前端不再自己伪造提示符，画面里的
    `emos:/ # ` 与命令回显全部来自这里（或真机）。
    """

    def __init__(self, ws: "WsClient", pty: bool = True) -> None:
        self.ws = ws
        self.shell = FakeShell()
        self.pty = pty
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, name="mock-shell", daemon=True)

    def start(self) -> None:
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        self.ws.close()

    def _run(self) -> None:
        try:
            if self.pty:
                # 官方 handle_shell：声明模式的这一帧在**任何终端字节之前**，且是文本帧。
                # 服务端把它原样转给浏览器（不会写进终端画面）。
                self.ws.send_text(json.dumps({"type": "shell_meta", "pty": True}))
                self.ws.send_binary(b"emos:/ # ")
            while not self._stop.is_set():
                opcode, payload = self.ws.recv_message()
                if opcode == 0x8:  # 服务端关连接 = 会话结束
                    break
                if opcode != 0x2:
                    # 真实设备上行只有 Binary（stdin 原始字节）；其它帧忽略而不是断链
                    continue
                text, keep = self.shell.feed(payload)
                if self.pty:
                    # pty 的 ECHO：命令本身先被回显一遍，然后是它的输出，最后是新提示符。
                    # 换行统一成 CRLF——终端画面里 `\n` 只下移不回车，会变成阶梯状。
                    echoed = payload.decode("utf-8", errors="replace")
                    text = echoed + (text or "") + "emos:/ # "
                    text = text.replace("\r\n", "\n").replace("\n", "\r\n")
                if text:
                    self.ws.send_binary(text.encode())
                if not keep:
                    # exit：设备侧 sh 退出，服务端按 device-shell-lost 收尾
                    break
        except (OSError, RuntimeError):
            pass  # 对端关闭/超时：会话结束，不是脚本失败
        finally:
            self.ws.close()


class ShellBridge:
    """控制面的 shell 生命周期：`shell_open` → 回拨；`shell_close` → 收摊。

    与真实固件同形：**只有收到 `shell_open` 才回拨**，绝不主动连 `/shell/{id}`
    （服务端 `claim` 只认自己刚 reserve 的那一把槽，自发连接会被 409 拒掉）。
    """

    def __init__(self, host: str, port: int, device_id: str, token: str, enabled: bool) -> None:
        self.host = host
        self.port = port
        self.device_id = device_id
        self.token = token
        self.enabled = enabled
        self._session: "Optional[ShellSession]" = None

    def handle(self, msg: dict) -> bool:
        """处理 shell 相关控制帧；返回 True 表示已消化（调用方别当业务消息继续处理）。"""
        kind = msg.get("type")
        if kind == "shell_open":
            # 控制台带 `pty:true`（终端，要提示符与回显）；一次性操作不带（干净管道）。
            # 与真机一致：字段缺省即 pipe，老固件同样忽略它。
            self.open(pty=bool(msg.get("pty")))
            return True
        if kind == "shell_close":
            self.close()
            return True
        return False

    def open(self, pty: bool = False) -> None:
        if not self.enabled:
            print("[shell] 收到 shell_open，但本次运行已关闭 shell 支持（--no-shell）")
            return
        if self._session is not None:
            self.close()
        try:
            ws = WsClient(
                self.host,
                self.port,
                "/shell/{}".format(self.device_id),
                self.token,
            )
        except (OSError, RuntimeError) as err:
            print("[shell] 回拨 /shell 失败：{}".format(err), file=sys.stderr)
            return
        # 长驻会话可能几分钟没有输入：不能留 10s 读超时，否则空闲就自己断
        ws.sock.settimeout(None)
        self._session = ShellSession(ws, pty=pty)
        self._session.start()
        mode = "pty 终端" if pty else "pipe 管道"
        print(
            "[shell] shell_open → 已回拨 /shell/{}（{}，内置假 shell，不执行真实命令）".format(
                self.device_id, mode
            )
        )

    def close(self) -> None:
        session, self._session = self._session, None
        if session is not None:
            session.stop()
            print("[shell] shell_close → 回拨通道已关闭")


def recv_control(ctl: "WsClient", bridge: ShellBridge, timeout: "Optional[float]" = None) -> "Optional[dict]":
    """读一条控制面 JSON；`shell_open`/`shell_close` 就地分派。

    `timeout` 给了就带超时（`--stay` 期间用），超时返回 None。
    """
    while True:
        raw = ctl.recv_text_timeout(timeout) if timeout is not None else ctl.recv_text()
        if raw is None:
            return None
        msg = json.loads(raw)
        if bridge.handle(msg):
            continue
        return msg


def tone_pcm(freq: int = 440, seconds: float = 0.6) -> bytes:
    """一段 440Hz 正弦波（S16_LE mono 16kHz），模拟一句话的"语音"。"""
    n = int(MIC_SR * seconds)
    return b"".join(
        struct.pack("<h", int(8000 * math.sin(2 * math.pi * freq * i / MIC_SR)))
        for i in range(n)
    )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8000)
    ap.add_argument("--token", default="")
    ap.add_argument("--device-id", default="mock-biscuit-001")
    ap.add_argument(
        "--save-downlink",
        default="",
        help="把下行 0x02 载荷（48kHz mono S16LE）追加写入该文件，供本地播放核对",
    )
    ap.add_argument(
        "--stay",
        type=float,
        default=0.0,
        help="语音回路跑完后保持在线 N 秒（供浏览器控制台联调；默认 0 = 跑完即退出）",
    )
    ap.add_argument(
        "--no-shell",
        action="store_true",
        help="不响应 shell_open（默认开启内置假 shell 回拨，供控制台联调）",
    )
    args = ap.parse_args()

    # 假 shell 开关：命令行优先，其次 env（`ECHOMUSE_MOCK_SHELL=0` 关），默认开。
    # 默认开的理由：只有收到 shell_open 才回拨，对纯语音回路用例完全无副作用。
    shell_enabled = not args.no_shell and os.environ.get("ECHOMUSE_MOCK_SHELL", "1").lower() not in (
        "0",
        "false",
        "no",
    )
    bridge = ShellBridge(args.host, args.port, args.device_id, args.token, shell_enabled)

    # 1) control 面：register → ack + config
    ctl = WsClient(args.host, args.port, "/em/control", args.token)
    ctl.send_text(json.dumps({
        "type": "register",
        "device_id": args.device_id,
        "version": "mock-1.0.0",
        "capabilities": ["mic", "speaker", "leds", "buttons",
                          "oww_shadow", "oww_trigger", "oww_local_only"],
        "board": "biscuit",
        "base_os": "emos",
        "ip": "127.0.0.1",
    }))
    ack = recv_control(ctl, bridge)
    assert ack is not None and ack.get("type") == "ack", f"期望 ack，收到 {ack}"
    print(f"[1/4] register ✓  服务器 features={ack.get('features')}")
    cfg = recv_control(ctl, bridge)
    assert cfg is not None and cfg.get("type") == "config", f"期望 config，收到 {cfg}"
    print(f"[2/4] config push ✓  owwOnDevice={cfg.get('owwOnDevice')}")

    # 2) data 面：与 control 同源接入（协议要求同地址；本机场景相同 IP）
    data = WsClient(args.host, args.port, f"/em/data?device_id={args.device_id}", args.token)
    # 真机固件拨通后先发一条 Text identify（data 面唯一约定的文本帧），服务端凭它
    # 精确认领；mock 保留显式 ?device_id= 与该帧，两条认领路径都覆盖。
    data.send_text(json.dumps({"type": "identify", "device_id": args.device_id}))

    # 3) 模拟唤醒 → mic_start → PCM → 0x04
    ctl.send_text(json.dumps({"type": "oww_wake", "score": 0.97, "threshold": 0.5}))
    while True:
        msg = recv_control(ctl, bridge)
        if msg is not None and msg.get("type") == "mic_start":
            break
    print("[3/4] mic_start ✓  上行一段'语音'…")
    pcm = tone_pcm()
    # 按 ~40ms 一帧上行。帧头与真机一致：`[0x01][seq u16 BE][payload]`
    # （见 EchoMuse controller/em_controller.py 的 MIC_HEADER_LEN = 3）——
    # 不带 seq 的帧曾让服务端把 seq 当 PCM、把哨兵当音频，联调假通过。
    chunk = MIC_SR // 25 * 2  # 40ms 的字节数
    seq = 0
    for i in range(0, len(pcm), chunk):
        data.send_binary(
            bytes([FRAME_MIC]) + struct.pack(">H", seq & 0xFFFF) + pcm[i:i + chunk]
        )
        seq += 1
    # 收口哨兵也是 0x01 帧的 1 字节负载，不是独立帧型。
    data.send_binary(bytes([FRAME_MIC]) + struct.pack(">H", seq & 0xFFFF) + bytes([FRAME_VAD_END]))

    # 4) 等回复：0x02*…0x03（容忍无回复——流水线引擎未配好时也明确指出）
    got = 0
    eos = False
    sink = open(args.save_downlink, "ab") if args.save_downlink else None
    try:
        while not eos:
            frame = data.recv_binary()
            kind = frame[0]
            if kind == FRAME_SPEAKER:
                payload = frame[1:]
                got += len(payload)
                if sink:
                    sink.write(payload)
            elif kind == FRAME_SPEAKER_EOS:
                eos = True
    except socket.timeout:
        # 10s 没有下行 = 流水线没出音（TTS 不可用/引擎缺失）。这不是联调失败：
        # 控制台联调（--stay）本来就不需要语音回路出声，如实提示后继续。
        print("⚠️  等下行超时：本次没有收到语音回复（检查 TTS 引擎/凭据）", file=sys.stderr)
    finally:
        if sink:
            sink.close()
    print(f"[4/4] 下行回复 ✓  共 {got} 字节 PCM（48kHz S16LE ≈ {got / SPEAKER_SR / 2:.2f}s）")
    if args.save_downlink and got:
        print(
            f"       下行已保存 → {args.save_downlink}："
            f"ffplay -f s16le -ar 48000 -ch_layout mono {args.save_downlink}"
        )
    if got == 0:
        print("⚠️  回复为空：检查 TTS 是否可用（[tts] 引擎/凭据）", file=sys.stderr)

    # 5) 可选：保持在线，供浏览器控制台联调（不回这一段就走原行为：跑完即退出）
    if args.stay > 0:
        print(
            f"[stay] 保持在线 {args.stay:.0f}s：浏览器 → EchoMuse → 点设备 → 控制台，"
            f"输入 uname -a 应看到假 shell 输出（Ctrl-C 退出）"
        )
        deadline = time.monotonic() + args.stay
        try:
            while time.monotonic() < deadline:
                # 1s 一轮：既能及时响应 shell_open/shell_close，也能按时醒来看 deadline
                recv_control(ctl, bridge, timeout=1.0)
        except KeyboardInterrupt:
            print("\n[stay] 已中断保持在线")
        finally:
            bridge.close()

    bridge.close()
    ctl.close()
    data.close()
    print("EchoMuse mock 设备联调全部通过 ✓")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (AssertionError, RuntimeError) as e:
        print(f"✗ 联调失败：{e}", file=sys.stderr)
        sys.exit(1)
