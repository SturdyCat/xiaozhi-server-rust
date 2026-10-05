#!/usr/bin/env python3
"""xiaozhi-server-rust 协议联调客户端（零第三方依赖，纯标准库实现 RFC 6455）。

模拟 macApp 管理端测试台：hello 带 `test:true`，逐项验证三个**独立服务请求-响应**
（服务端无 mock，直调真实引擎；ESP 设备正式流程由 VAD 切句驱动，不经过这些端点）。

流程：
  1. 建立 WebSocket 连接到 /api/ws
  2. 发送测试台 hello（version=1，test=true，上行 16k opus）
  3. 断言服务器 hello（含 downlink audio_params）
  4. asr_test start → stop（不上行音频）→ 断言 stt 回包（空缓冲占位文案）
  5. tts_test（"你好"）→ 断言 tts start → sentence_start → 二进制帧 → tts stop
  6. llm_test → 断言 llm_test 回包到达（state=ok|error 均可，取决于 [llm] 配置）

用法：
  python3 tests/mock_client.py [--host 127.0.0.1] [--port 8000] [--token <bearer>]
"""

import argparse
import base64
import json
import os
import socket
import struct
import sys
import uuid


def ws_connect(host: str, port: int, path: str, token: str | None) -> socket.socket:
    """完成 WebSocket 握手，返回已升级的 TCP socket。"""
    sock = socket.create_connection((host, port), timeout=10)
    key = base64.b64encode(os.urandom(16)).decode()
    req = [
        f"GET {path} HTTP/1.1",
        f"Host: {host}:{port}",
        "Upgrade: websocket",
        "Connection: Upgrade",
        f"Sec-WebSocket-Key: {key}",
        "Sec-WebSocket-Version: 13",
    ]
    if token:
        req.append(f"Authorization: Bearer {token}")
    sock.sendall(("\r\n".join(req) + "\r\n\r\n").encode())

    # 读取并校验 101 响应
    buf = b""
    while b"\r\n\r\n" not in buf:
        chunk = sock.recv(4096)
        if not chunk:
            raise RuntimeError("握手失败：连接关闭")
        buf += chunk
    head, _, _ = buf.partition(b"\r\n\r\n")
    status_line = head.split(b"\r\n", 1)[0].decode()
    if "101" not in status_line:
        raise RuntimeError(f"握手失败，状态码：{status_line}")
    return sock


def _recv_exact(sock: socket.socket, n: int) -> bytes:
    data = b""
    while len(data) < n:
        chunk = sock.recv(n - len(data))
        if not chunk:
            raise RuntimeError("连接关闭，读取不完整")
        data += chunk
    return data


def ws_send_text(sock: socket.socket, text: str) -> None:
    """发送带掩码的客户端文本帧（RFC 要求客户端帧必须掩码）。"""
    payload = text.encode("utf-8")
    mask = os.urandom(4)
    masked = bytes(payload[i] ^ mask[i % 4] for i in range(len(payload)))
    header = bytes([0x81])  # FIN + text opcode
    length = len(payload)
    if length < 126:
        header += bytes([0x80 | length])
    elif length < 65536:
        header += bytes([0x80 | 126]) + struct.pack(">H", length)
    else:
        header += bytes([0x80 | 127]) + struct.pack(">Q", length)
    sock.sendall(header + mask + masked)


def ws_recv(sock: socket.socket) -> tuple[int, bytes]:
    """读取一帧，返回 (opcode, payload)。自动处理 ping/pong/close。"""
    b0, b1 = _recv_exact(sock, 2)
    opcode = b0 & 0x0F
    masked = bool(b1 & 0x80)
    length = b1 & 0x7F
    if length == 126:
        (length,) = struct.unpack(">H", _recv_exact(sock, 2))
    elif length == 127:
        (length,) = struct.unpack(">Q", _recv_exact(sock, 8))
    if masked:
        _recv_exact(sock, 4)  # 服务端帧通常不带掩码，这里仅防御性消费
    payload = _recv_exact(sock, length)

    if opcode == 0x8:  # close
        try:
            sock.sendall(bytes([0x88, 0x00]))
        except OSError:
            pass
        raise RuntimeError("服务端发送 close")
    if opcode == 0x9:  # ping -> pong
        sock.sendall(bytes([0x8A, length]) + payload)
        return ws_recv(sock)
    if opcode == 0xA:  # pong
        return ws_recv(sock)
    return opcode, payload


def recv_text_until(sock: socket.socket, mtype: str, max_frames: int = 2000) -> dict | None:
    """持续读帧直到出现指定 type 的文本消息（跳过二进制/心跳），超限返回 None。"""
    for _ in range(max_frames):
        opcode, payload = ws_recv(sock)
        if opcode != 0x1:
            continue
        msg = json.loads(payload.decode())
        if msg.get("type") == mtype:
            return msg
    return None


def main() -> int:
    ap = argparse.ArgumentParser(description="xiaozhi-server-rust 协议联调客户端")
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8000)
    ap.add_argument("--token", default=None, help="Bearer token（当 expected_token 非空时必填）")
    args = ap.parse_args()

    print(f"[*] 连接 ws://{args.host}:{args.port}/api/ws")
    sock = ws_connect(args.host, args.port, "/api/ws", args.token)
    print("[+] 握手成功")

    # 1) 测试台 hello（test=true：受理 asr_test/tts_test/llm_test 独立服务请求）
    hello = {
        "type": "hello",
        "version": 1,
        "test": True,
        "audio_params": {
            "format": "opus",
            "sample_rate": 16000,
            "channels": 1,
            "frame_duration": 60,
        },
        "features": {"mcp": False, "aec": False},
    }
    ws_send_text(sock, json.dumps(hello))
    print("[>] 已发送 hello（test=true）")

    # 2) 服务器 hello
    opcode, payload = ws_recv(sock)
    assert opcode == 0x1, f"期望文本帧，实际 opcode={opcode}"
    srv_hello = json.loads(payload.decode())
    assert srv_hello.get("type") == "hello", f"非 hello 消息：{srv_hello}"
    ap_params = srv_hello.get("audio_params") or {}
    print(
        f"[<] 服务器 hello：session_id={srv_hello.get('session_id')}, "
        f"downlink sr={ap_params.get('sample_rate')}, "
        f"frame={ap_params.get('frame_duration')}"
    )

    # 3) ASR 测试（独立服务请求-响应）：start → 不上行音频 → stop → stt 回包
    #    （空缓冲返回占位文案，用于验证端点受理与回包链路；真实识别需上行有效 Opus 音频）
    ws_send_text(sock, json.dumps({"type": "asr_test", "action": "start"}))
    ws_send_text(sock, json.dumps({"type": "asr_test", "action": "stop"}))
    print("[>] 已发送 asr_test start/stop")
    errors: list[str] = []
    msg = recv_text_until(sock, "stt")
    if msg is None:
        errors.append("缺少 stt（asr_test stop 无回包）")
    else:
        print(f"[<] stt：{msg.get('text')}")

    # 4) TTS 测试（独立服务请求-响应）：文本直接合成下行
    ws_send_text(sock, json.dumps({"type": "tts_test", "text": "你好"}))
    print("[>] 已发送 tts_test")
    got_types: list[str] = []
    binary_frames = 0
    received_stop = False
    for _ in range(2000):  # 上限保护，避免无限阻塞
        opcode, payload = ws_recv(sock)
        if opcode == 0x2:  # 下行二进制（TTS Opus 帧）
            binary_frames += 1
            continue
        if opcode != 0x1:
            continue
        msg = json.loads(payload.decode())
        mtype = msg.get("type")
        if mtype == "tts":
            got_types.append(f"tts:{msg.get('state')}")
            if msg.get("state") == "stop":
                received_stop = True
                break
        else:
            got_types.append(mtype)
    print(f"[<] 文本消息序列：{got_types}")
    if binary_frames:
        print(f"[<] 收到下行二进制帧（TTS Opus）：{binary_frames} 个")
    if "tts:start" not in got_types:
        errors.append("缺少 tts start")
    if "tts:sentence_start" not in got_types:
        errors.append("缺少 tts sentence_start")
    if not received_stop:
        errors.append("缺少 tts stop")
    if not binary_frames:
        errors.append("缺少下行二进制音频帧")

    # 5) LLM 测试（独立服务请求-响应）：服务端按磁盘最新 [llm] 配置直调 LLM，
    #    单轮无历史；state=ok|error 均算端点可达（error 取决于 api_key 配置）。
    ws_send_text(sock, json.dumps({"type": "llm_test", "text": "你好"}))
    print("[>] 已发送 llm_test")
    msg = recv_text_until(sock, "llm_test")
    if msg is None:
        errors.append("缺少 llm_test 回包")
    else:
        print(f"[<] llm_test：state={msg.get('state')} text={str(msg.get('text'))[:60]!r}")

    try:
        sock.sendall(bytes([0x88, 0x00]))  # 发送 close
    except OSError:
        pass

    if errors:
        print("[!] FAIL: " + "; ".join(errors))
        return 1
    print("[+] PASS: 握手/协商 + asr_test/tts_test/llm_test 三个独立服务端点均符合预期")
    return 0


if __name__ == "__main__":
    sys.exit(main())
