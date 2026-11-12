#!/usr/bin/env bash
# Real Rust admin/API server, explicitly without inference or model dependencies.
# W15-1：管理台静态目录默认切到 client/admin-web/dist（React SPA + /em/ 白名单 4 文件）。
# 软下线第一波：旧端 Kotlin/Kuikly web 产物保留在仓库，但本脚本不再触发 Gradle publishWeb。
set -euo pipefail
ROOT="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
DEFAULT_WEB="$ROOT/client/admin-web/dist"
if [[ "${1:-}" == "--build-web" ]]; then
  # 兼容旧调用：不再触发 Gradle publishWeb；构建链已改为 pnpm（见下方提示）。
  shift
  printf 'Note: --build-web is deprecated and no longer builds anything (legacy UI soft-offline).\n' >&2
  printf 'Build the React admin instead: pnpm --dir client/admin-web build\n' >&2
fi
WEB="${XIAOZHI_ADMIN_DIR:-$DEFAULT_WEB}"
# 新检查条件：dist/index.html（SPA 入口）+ dist/em/index.html（官方页尾斜杠契约）。
# assets/ 由 Vite 内容哈希生成、文件名不固定，不作硬性存在条件；三个 static js
# 由 tools/admin_web_switch_check.sh 在真 server 上断言。
if [[ ! -f "$WEB/index.html" || ! -f "$WEB/em/index.html" ]]; then
  if [[ "$WEB" != "$DEFAULT_WEB" ]]; then
    printf 'Custom XIAOZHI_ADMIN_DIR has missing assets (need index.html + em/index.html): %s\n' "$WEB" >&2
    exit 1
  fi
  printf 'admin-web dist is missing or lacks /em/ assets; build it first:\n' >&2
  printf '  pnpm --dir client/admin-web build\n' >&2
  exit 1
fi
export XIAOZHI_ADMIN_DIR="$WEB"
CARGO="${CARGO:-cargo}"
if ! command -v "$CARGO" >/dev/null 2>&1; then
  CARGO="$HOME/.cargo/bin/cargo"
fi
if [[ ! -x "$(command -v "$CARGO" 2>/dev/null || true)" ]]; then
  printf 'Rust cargo is required; install the Rust toolchain first.\n' >&2
  exit 1
fi
cd "$ROOT"
printf 'Starting real Rust admin server (no models, no sherpa/libopus required).\n'
printf 'Default UI: http://127.0.0.1:8000/ ; Ctrl+C stops the server.\n'
printf 'After UI edits: pnpm --dir client/admin-web build ; custom port: --port 8001\n'
exec "$CARGO" run --manifest-path "$ROOT/server/Cargo.toml" --no-default-features -- --dev "$@"
