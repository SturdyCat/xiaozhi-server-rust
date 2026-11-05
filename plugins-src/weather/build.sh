#!/usr/bin/env bash
# 构建 weather.official 组件：cargo(wasm32-unknown-unknown) → wasm-tools component new
# 产物：server/tests/fixtures/wasm/weather.wasm（宿主回放测试读此路径，产物入库）
# 依赖：rustup target add wasm32-unknown-unknown；brew install wasm-tools
set -euo pipefail
cd "$(dirname "$0")"

cargo build --release --target wasm32-unknown-unknown

OUT=../../server/tests/fixtures/wasm
mkdir -p "$OUT"
src="target/wasm32-unknown-unknown/release/weather_official.wasm"
wasm-tools component new "$src" -o "$OUT/weather.wasm"
echo "built $OUT/weather.wasm ($(wc -c < "$OUT/weather.wasm") bytes)"
