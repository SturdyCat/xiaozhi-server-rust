#!/usr/bin/env bash
# Real Rust admin/API server, explicitly without inference or model dependencies.
set -euo pipefail
ROOT="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
BUILD_WEB=0
if [[ "${1:-}" == "--build-web" ]]; then
  BUILD_WEB=1
  shift
fi
WEB="${XIAOZHI_ADMIN_DIR:-$ROOT/client/apps/h5App/web}"
if [[ "$BUILD_WEB" == 1 || ! -f "$WEB/index.html" || ! -f "$WEB/nativevue2.js" || ! -f "$WEB/h5App.js" ]]; then
  if [[ "$WEB" != "$ROOT/client/apps/h5App/web" ]]; then
    printf 'Custom XIAOZHI_ADMIN_DIR has missing assets; build that directory first: %s\n' "$WEB" >&2
    exit 1
  fi
  (cd "$ROOT/client" && ./gradlew :apps:h5App:publishWeb -PwebSourceMap=false)
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
printf 'After UI edits: ./dev-server.sh --build-web ; custom port: --port 8001\n'
exec "$CARGO" run --manifest-path "$ROOT/server/Cargo.toml" --no-default-features -- --dev "$@"
