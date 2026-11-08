#!/usr/bin/env bash
# 构建 nlu.official 组件：cargo(wasm32-unknown-unknown) → wasm-tools component new
# 产物不入 git；全量分发请用仓库根的 tools/build-system-plugins.sh
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown
OUT=../../server/tests/fixtures/wasm
mkdir -p "$OUT"
src="target/wasm32-unknown-unknown/release/nlu_official.wasm"
wasm-tools component new "$src" -o "$OUT/nlu.wasm"
echo "built $OUT/nlu.wasm ($(wc -c < "$OUT/nlu.wasm") bytes)"
