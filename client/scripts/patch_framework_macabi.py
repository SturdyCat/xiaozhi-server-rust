#!/usr/bin/env python3
"""把 macOS Mach-O 的 LC_BUILD_VERSION 平台改写为 Mac Catalyst（macabi）。

背景：shared.framework 由 Kotlin/Native macosArm64 目标产出（静态框架，
Versions/A/shared 是 ar 归档，内含 shared.framework.o 等目标文件成员），
平台标记是 macOS(1)；macosApp 是 Mac Catalyst（macabi）目标，ld 拒绝平台不匹配：
    ld: building for 'macCatalyst', but linking in object file ... built for 'macOS'
Kotlin/Native 没有 macabi 目标，本机 vtool 也不认识 macabi 平台名（实测），
故直接改写加载命令字段（社区处理 Kotlin 框架上 Catalyst 的标准做法）：
    platform  macOS(1) → MacCatalyst(6)
    minos     保持 ≥12.0（Catalyst 最低支持版本；低于 12.0 一并抬高）

支持两种输入：单个 64 位 Mach-O（原地改写）；ar 归档（ar x 抽出 .o 成员 →
逐个补丁 → ar cr 重建归档 → ranlib 重建符号表）。补丁幂等：全部成员已是
macabi 时不重写归档。

用法：python3 patch_framework_macabi.py <framework二进制路径>
"""
import os
import struct
import subprocess
import sys
import tempfile

LC_BUILD_VERSION = 0x32
PLATFORM_MACCATALYST = 6
MINOS_12_0 = (12 << 16)  # X.Y.Z 打包为 (X<<16)|(Y<<8)|Z
MAGIC_MH_MAGIC_64 = b"\xcf\xfa\xed\xfe"  # 0xFEEDFACF 小端字节序
AR_MAGIC = b"!<arch>\n"


def minos_str(v: int) -> str:
    return f"{v >> 16}.{(v >> 8) & 0xFF}.{v & 0xFF}"


def patch_macho_bytes(data: bytearray, base: int, label: str) -> bool:
    """在 data[base:] 处改写单个 Mach-O 的 LC_BUILD_VERSION。有改动返回 True。"""
    if bytes(data[base : base + 4]) != MAGIC_MH_MAGIC_64:
        return False
    ncmds = struct.unpack_from("<I", data, base + 16)[0]
    offset = base + 32  # mach_header_64 固定 32 字节
    for _ in range(ncmds):
        cmd, cmdsize = struct.unpack_from("<II", data, offset)
        if cmd == LC_BUILD_VERSION:
            platform, minos = struct.unpack_from("<II", data, offset + 8)
            if platform == PLATFORM_MACCATALYST:
                print(f"  {label}: 已是 macabi（minos {minos_str(minos)}），跳过")
                return False
            if minos < MINOS_12_0:
                struct.pack_into("<I", data, offset + 12, MINOS_12_0)
                print(f"  {label}: minos {minos_str(minos)} → 12.0")
            struct.pack_into("<I", data, offset + 8, PLATFORM_MACCATALYST)
            print(f"  {label}: platform 1 (macOS) → 6 (MacCatalyst)")
            return True
        offset += cmdsize
    print(f"  {label}: 无 LC_BUILD_VERSION，跳过")
    return False


def patch_single_file(path: str) -> bool:
    with open(path, "rb") as f:
        data = bytearray(f.read())
    changed = patch_macho_bytes(data, 0, os.path.basename(path))
    if changed:
        with open(path, "wb") as f:
            f.write(data)
    return changed


def patch_archive(path: str) -> None:
    """ar x 抽出 .o 成员 → 逐个补丁 → ar cr 重建 → ranlib 重建符号表。"""
    members = subprocess.run(
        ["ar", "t", path], capture_output=True, text=True, check=True
    ).stdout.split()
    objects = [m for m in members if m.endswith(".o")]
    with tempfile.TemporaryDirectory() as td:
        subprocess.run(["ar", "x", os.path.abspath(path)], cwd=td, check=True)
        changed = False
        for name in objects:
            member = os.path.join(td, name)
            if patch_single_file(member):
                changed = True
        if not changed:
            print("所有成员均已是 macabi，归档保持不变")
            return
        subprocess.run(["ar", "cr", path] + objects, cwd=td, check=True)
        subprocess.run(["ranlib", path], check=True)
        print("归档已重建（成员替换 + 符号表重建）")


def main(path: str) -> None:
    with open(path, "rb") as f:
        head = f.read(8)

    if head == AR_MAGIC:
        print(f"ar 归档：{path}")
        patch_archive(path)
    elif head[:4] == MAGIC_MH_MAGIC_64:
        print(f"Mach-O：{path}")
        patch_single_file(path)
    else:
        sys.exit(f"既不是 Mach-O 也不是 ar 归档：{path}")
    print("补丁完成")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
