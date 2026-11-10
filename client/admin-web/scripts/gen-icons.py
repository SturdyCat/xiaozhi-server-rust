#!/usr/bin/env python3
"""从仓库根 logo.png 生成 Tauri 图标与 Web 品牌图。

唯一视觉来源：<repo>/logo.png（最终稿 · AI 聊天机器人 AppIcon，蓝紫渐变 + 白色螺旋
对话气泡；设计稿记录见 <repo>/xiaozhi-logo.miora）。禁止再用纯色占位图标。

用法（在仓库任意目录执行）：
    python3 -S client/admin-web/scripts/gen-icons.py

依赖：macOS 自带 sips / iconutil；其余只用 Python 标准库，不引入第三方包。
生成物：src-tauri/icons/* 与 public/logo.png，均入库；中间尺寸缓存在
client/admin-web/node_modules/.cache/（已被 .gitignore 忽略）。
"""

from __future__ import annotations

import pathlib
import struct
import subprocess
import zlib

REPO = pathlib.Path(__file__).resolve().parents[3]
APP = REPO / "client" / "admin-web"
LOGO = REPO / "logo.png"
ICONS = APP / "src-tauri" / "icons"
PUBLIC = APP / "public"
CACHE = APP / "node_modules" / ".cache"
ICONSET = CACHE / "tauri-iconset.iconset"
SIZED_DIR = CACHE / "logo-sizes"

PNG_SIZES = (16, 32, 48, 64, 128, 256, 512, 1024)
ICO_SIZES = (16, 32, 48, 64, 128, 256)

# iconutil 要求的 iconset 文件名 -> 实际像素尺寸
ICONSET_ENTRIES = {
    "icon_16x16.png": 16,
    "icon_16x16@2x.png": 32,
    "icon_32x32.png": 32,
    "icon_32x32@2x.png": 64,
    "icon_128x128.png": 128,
    "icon_128x128@2x.png": 256,
    "icon_256x256.png": 256,
    "icon_256x256@2x.png": 512,
    "icon_512x512.png": 512,
    "icon_512x512@2x.png": 1024,
}


def run(*args: str) -> None:
    subprocess.run(list(args), check=True, capture_output=True, text=True)


def _paeth(a: int, b: int, c: int) -> int:
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    return b if pb <= pc else c


def _chunk(kind: bytes, data: bytes) -> bytes:
    return (
        struct.pack(">I", len(data))
        + kind
        + data
        + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    )


def to_rgba_png(png: bytes) -> bytes:
    """把 sips 输出的 8-bit RGB PNG 转成 RGBA（alpha 全 255）。

    Tauri 的 `generate_context!` 要求 `bundle.icon` 的 PNG 是 RGBA，否则报
    "icon ... is not RGBA" 并编译失败；logo.png 本身无 alpha，sips 缩放后
    仍是 RGB。这里用标准库解 filter 后按 filter=0 重编码，避免引入 Pillow。
    """
    if png[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("不是 PNG")
    pos, ihdr, idat = 8, None, bytearray()
    while pos < len(png):
        length = struct.unpack(">I", png[pos : pos + 4])[0]
        kind = png[pos + 4 : pos + 8]
        data = png[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if kind == b"IHDR":
            ihdr = struct.unpack(">IIBBBBB", data)
        elif kind == b"IDAT":
            idat.extend(data)
        elif kind == b"IEND":
            break
    if ihdr is None:
        raise ValueError("缺少 IHDR")
    width, height, depth, color, _, _, interlace = ihdr
    if depth != 8 or interlace != 0:
        raise ValueError(f"仅支持 8-bit 非隔行 PNG: {ihdr}")
    if color == 6:  # 已是 RGBA
        return png
    if color != 2:
        raise ValueError(f"仅支持 RGB/RGBA PNG，实际 color type={color}")

    raw = zlib.decompress(bytes(idat))
    bpp, stride = 3, width * 3
    previous = bytearray(stride)
    rows: list[bytes] = []
    offset = 0
    for _ in range(height):
        filter_type = raw[offset]
        offset += 1
        line = bytearray(raw[offset : offset + stride])
        offset += stride
        if filter_type:
            for i in range(stride):
                a = line[i - bpp] if i >= bpp else 0
                b = previous[i]
                c = previous[i - bpp] if i >= bpp else 0
                if filter_type == 1:
                    line[i] = (line[i] + a) & 0xFF
                elif filter_type == 2:
                    line[i] = (line[i] + b) & 0xFF
                elif filter_type == 3:
                    line[i] = (line[i] + ((a + b) // 2)) & 0xFF
                elif filter_type == 4:
                    line[i] = (line[i] + _paeth(a, b, c)) & 0xFF
                else:
                    raise ValueError(f"未知 PNG filter: {filter_type}")
        rgba = bytearray(width * 4)
        for x in range(width):
            rgba[x * 4 : x * 4 + 3] = line[x * 3 : x * 3 + 3]
            rgba[x * 4 + 3] = 255
        rows.append(bytes(rgba))
        previous = line

    new_raw = b"".join(b"\x00" + row for row in rows)
    new_ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + _chunk(b"IHDR", new_ihdr)
        + _chunk(b"IDAT", zlib.compress(new_raw, 6))
        + _chunk(b"IEND", b"")
    )


def make_sized_pngs() -> dict[int, pathlib.Path]:
    SIZED_DIR.mkdir(parents=True, exist_ok=True)
    result: dict[int, pathlib.Path] = {}
    for size in PNG_SIZES:
        out = SIZED_DIR / f"logo_{size}.png"
        run("sips", "-z", str(size), str(size), str(LOGO), "--out", str(out))
        out.write_bytes(to_rgba_png(out.read_bytes()))
        result[size] = out
    return result


def build_icns(sized: dict[int, pathlib.Path]) -> None:
    ICONSET.mkdir(parents=True, exist_ok=True)
    for name, size in ICONSET_ENTRIES.items():
        (ICONSET / name).write_bytes(sized[size].read_bytes())
    run("iconutil", "-c", "icns", str(ICONSET), "-o", str(ICONS / "icon.icns"))


def build_ico(sized: dict[int, pathlib.Path]) -> None:
    """Vista+ 的 ICO 允许直接内嵌 PNG 数据，无需 libpng/ImageMagick。"""
    images = [(size, sized[size].read_bytes()) for size in ICO_SIZES]
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    directory = bytearray()
    payload = bytearray()
    for size, data in images:
        # ICO 宽高为单字节，256 用 0 表示
        dimension = 0 if size >= 256 else size
        directory.extend(
            struct.pack(
                "<BBBBHHII",
                dimension,
                dimension,
                0,
                0,
                1,
                32,
                len(data),
                offset,
            )
        )
        payload.extend(data)
        offset += len(data)
    (ICONS / "icon.ico").write_bytes(header + bytes(directory) + bytes(payload))


def main() -> None:
    if not LOGO.is_file():
        raise SystemExit(f"缺少 logo 源文件: {LOGO}")
    ICONS.mkdir(parents=True, exist_ok=True)
    PUBLIC.mkdir(parents=True, exist_ok=True)

    sized = make_sized_pngs()

    # tauri.conf.json 的 bundle.icon 引用的 PNG + 模板惯例的 icon.png
    for size, name in (
        (32, "32x32.png"),
        (128, "128x128.png"),
        (256, "128x128@2x.png"),
        (512, "icon.png"),
    ):
        (ICONS / name).write_bytes(sized[size].read_bytes())

    # Web 端 favicon 与侧边栏品牌图，由 Vite public/ 原样复制到 dist/
    (PUBLIC / "logo.png").write_bytes(sized[512].read_bytes())

    build_icns(sized)
    build_ico(sized)

    print("来源:", LOGO)
    for path in sorted(ICONS.iterdir()):
        print(f"  {path.name:20s} {path.stat().st_size:>9,d} B")
    favicon = PUBLIC / "logo.png"
    print(f"  {'public/logo.png':20s} {favicon.stat().st_size:>9,d} B")


if __name__ == "__main__":
    main()
