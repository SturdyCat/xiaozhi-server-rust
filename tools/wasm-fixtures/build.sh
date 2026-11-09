#!/usr/bin/env bash
# 构建 WASM 测试夹具组件：cargo(wasm32-unknown-unknown) → wasm-tools component new
# 产物：server/tests/fixtures/wasm/{hello,greeter-user,trap,tuyasign}.wasm（测试读此路径，产物不入库）
# 依赖：rustup target add wasm32-unknown-unknown；brew install wasm-tools
set -euo pipefail
cd "$(dirname "$0")"

cargo build --release --target wasm32-unknown-unknown

OUT=../../server/tests/fixtures/wasm
mkdir -p "$OUT"
for crate_name in hello greeter-user trap tuyasign; do
  # cargo 把 crate 名里的连字符转成下划线作为产物名
  src="target/wasm32-unknown-unknown/release/${crate_name//-/_}.wasm"
  wasm-tools component new "$src" -o "$OUT/${crate_name}.wasm"
  echo "built $OUT/${crate_name}.wasm ($(wc -c < "$OUT/${crate_name}.wasm") bytes)"
done
