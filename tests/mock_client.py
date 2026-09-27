#!/usr/bin/env python3
"""xiaozhi-server-rust 协议联调客户端（零第三方依赖，纯标准库实现 RFC 6455）。

流程：
  1. 建立 WebSocket 连接到 /ws
  2. 发送设备 hello（version=1，上行 16k opus）
  3. 断言服务器 hello（含 downlink audio_params）
  4. 发送 listen start
  5. 断言完整对话回包：stt -> llm -> tts start -> tts sentence_start -> 二进制帧 -> tts stop

mock 模式下：
  - ASR 回显固定文本，LLM 由本地/远程接口返回（默认 api_base 为占位，需配置真实 key 才有文本）。
  - 下行二进制帧为**空帧**（mock 下 encode_opus_frame 返回空），仅用于验证协议链路。

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


def main() -> int:
    ap = argparse.ArgumentParser(description="xiaozhi-server-rust 协议联调客户端")
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8000)
    ap.add_argument("--token", default=None, help="Bearer token（当 expected_token 非空时必填）")
    args = ap.parse_args()

    print(f"[*] 连接 ws://{args.host}:{args.port}/ws")
    sock = ws_connect(args.host, args.port, "/ws", args.token)
    print("[+] 握手成功")

    # 1) 设备 hello
    hello = {
        "type": "hello",
        "version": 1,
        "audio_params": {
            "format": "opus",
            "sample_rate": 16000,
            "channels": 1,
            "frame_duration": 60,
        },
        "features": {"mcp": False, "aec": False},
    }
    ws_send_text(sock, json.dumps(hello))
    print("[>] 已发送 hello")

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

    # 3) listen start
    ws_send_text(sock, json.dumps({"type": "listen", "state": "start", "mode": "manual"}))
    print("[>] 已发送 listen start")

    # 4) 断言对话回包顺序
    got_types: list[str] = []
    binary_frames = 0
    received_stop = False

    for _ in range(200):  # 上限保护，避免无限阻塞
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

    # 校验关键节点
    errors: list[str] = []
    if not got_types or got_types[0] != "stt":
        errors.append("缺少 stt")
    if "llm" not in got_types:
        errors.append("缺少 llm")
    if "tts:start" not in got_types:
        errors.append("缺少 tts start")
    if "tts:sentence_start" not in got_types:
        errors.append("缺少 tts sentence_start")
    if not received_stop:
        errors.append("缺少 tts stop")

    try:
        sock.sendall(bytes([0x88, 0x00]))  # 发送 close
    except OSError:
        pass

    if errors:
        print("[!] FAIL: " + "; ".join(errors))
        return 1
    print("[+] PASS: 握手/协商/完整对话回包均符合预期")
    return 0


if __name__ == "__main__":
    sys.exit(main())
