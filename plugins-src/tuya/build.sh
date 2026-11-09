#!/usr/bin/env bash
# 构建 tuya.official 组件：cargo(wasm32-unknown-unknown) → wasm-tools component new
# 产物不入 git；全量分发请用仓库根的 tools/build-system-plugins.sh
set -euo pipefail
cd "$(dirname "$0")"
# 规避本机（macOS 沙盒/TCC 环境）rustc 对仓库内 target 目录的间歇 EPERM：
# 编译缓存默认放 /tmp，可用 CARGO_TARGET_DIR 覆盖（CI 无此问题）。
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/xiaozhi-guest-target}"
cargo build --release --target wasm32-unknown-unknown
OUT=../../server/tests/fixtures/wasm
mkdir -p "$OUT"
src="$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/tuya_official.wasm"
wasm-tools component new "$src" -o "$OUT/tuya.wasm"
echo "built $OUT/tuya.wasm ($(wc -c < "$OUT/tuya.wasm") bytes)"
