#!/usr/bin/env bash
# 构建 tts.official 组件：cargo(wasm32-unknown-unknown) → wasm-tools component new
# 产物不入 git；全量分发请用仓库根的 tools/build-system-plugins.sh
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown
OUT=../../server/tests/fixtures/wasm
mkdir -p "$OUT"
src="target/wasm32-unknown-unknown/release/tts_official.wasm"
wasm-tools component new "$src" -o "$OUT/tts.wasm"
echo "built $OUT/tts.wasm ($(wc -c < "$OUT/tts.wasm") bytes)"
