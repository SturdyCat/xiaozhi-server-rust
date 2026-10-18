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
"""

import argparse
import json
import math
import socket
import struct
import sys
import base64
from typing import Optional

# 帧型（设备 ↔ 服务器，方向命名空间：上行 0x01/0x04/0x05/0x07；下行 0x02/0x03）
FRAME_MIC = 0x01
FRAME_VAD_END = 0x04
FRAME_NO_SPEECH = 0x05
FRAME_SPEAKER = 0x02
FRAME_SPEAKER_EOS = 0x03

MIC_SR = 16000


class WsClient:
    """最小 WebSocket 客户端（仅本脚本够用：文本收发、二进制收发、服务端 ping 忽略）。"""

    def __init__(self, host: str, port: int, path: str, token: str = ""):
        self.sock = socket.create_connection((host, port), timeout=10)
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

    def close(self) -> None:
        try:
            self._send_frame(0x8, b"")
            self.sock.close()
        except OSError:
            pass


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
    args = ap.parse_args()

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
    ack = json.loads(ctl.recv_text())
    assert ack.get("type") == "ack", f"期望 ack，收到 {ack}"
    print(f"[1/4] register ✓  服务器 features={ack.get('features')}")
    cfg = json.loads(ctl.recv_text())
    assert cfg.get("type") == "config", f"期望 config，收到 {cfg}"
    print(f"[2/4] config push ✓  owwOnDevice={cfg.get('owwOnDevice')}")

    # 2) data 面：与 control 同源接入（协议要求同地址；本机场景相同 IP）
    data = WsClient(args.host, args.port, f"/em/data?device_id={args.device_id}", args.token)

    # 3) 模拟唤醒 → mic_start → PCM → 0x04
    ctl.send_text(json.dumps({"type": "oww_wake", "score": 0.97, "threshold": 0.5}))
    while True:
        msg = json.loads(ctl.recv_text())
        if msg.get("type") == "mic_start":
            break
    print("[3/4] mic_start ✓  上行一段'语音'…")
    pcm = tone_pcm()
    # 按 ~40ms 一帧上行
    chunk = MIC_SR // 25 * 2  # 40ms 的字节数
    for i in range(0, len(pcm), chunk):
        data.send_binary(bytes([FRAME_MIC]) + pcm[i:i + chunk])
    data.send_binary(bytes([FRAME_VAD_END]))

    # 4) 等回复：0x02*…0x03（容忍无回复——流水线引擎未配好时也明确指出）
    got = 0
    eos = False
    while not eos:
        frame = data.recv_binary()
        kind = frame[0]
        if kind == FRAME_SPEAKER:
            got += len(frame) - 1
        elif kind == FRAME_SPEAKER_EOS:
            eos = True
    print(f"[4/4] 下行回复 ✓  共 {got} 字节 PCM（16kHz S16LE ≈ {got / MIC_SR / 2:.2f}s）")
    if got == 0:
        print("⚠️  回复为空：检查 TTS 是否可用（[tts] 引擎/凭据）", file=sys.stderr)

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
