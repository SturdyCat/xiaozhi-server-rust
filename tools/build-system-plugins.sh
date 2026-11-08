#!/usr/bin/env bash
# 构建 plugins-src/manifest.toml 声明的官方系统级 WASM 插件。
# 产物不入 git；默认写入 build/system-plugins/<id>.wasm，交给镜像/部署流水线导入。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${SYSTEM_PLUGIN_OUT:-$ROOT/build/system-plugins}"
MANIFEST="$ROOT/plugins-src/manifest.toml"
mkdir -p "$OUT"

while IFS=$'\t' read -r id source_dir package; do
    [[ -n "$id" ]] || continue
    source_dir="${source_dir:-plugins-src/$id}"
    package="${package:-${id##*.}}"
    source="$ROOT/$source_dir"
    manifest="$source/Cargo.toml"
    [[ -f "$manifest" ]] || {
        echo "缺少系统插件 Cargo.toml：$manifest" >&2
        exit 1
    }

    echo "[system-plugin] build $id from $source"
    cargo build --manifest-path "$manifest" --release --target wasm32-unknown-unknown
    target_dir="$(cargo metadata --manifest-path "$manifest" --no-deps --format-version 1 \
        | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
    wasm="$target_dir/wasm32-unknown-unknown/release/${package//-/_}.wasm"
    [[ -f "$wasm" ]] || {
        echo "找不到系统插件 wasm 产物：$wasm" >&2
        exit 1
    }
    wasm-tools component new "$wasm" -o "$OUT/$id.wasm"
    echo "[system-plugin] wrote $OUT/$id.wasm ($(wc -c < "$OUT/$id.wasm") bytes)"
done < <(
    python3 - "$MANIFEST" <<'PY'
import sys, tomllib
with open(sys.argv[1], "rb") as f:
    doc = tomllib.load(f)
for p in doc.get("plugins", []):
    source = p.get("source_dir") or ("plugins-src/" + p.get("dir", ""))
    print("\t".join((p["id"], source, p.get("package", ""))))
PY
)

echo "[system-plugin] output: $OUT"
