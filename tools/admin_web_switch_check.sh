#!/usr/bin/env bash
# W15-1 / 生产 admin_dir 切换验收脚本（研究 2/2 §3 断言的机器化实现）。
#
# 前置条件（缺一不可）：
#   1) 已构建 client/admin-web/dist：pnpm --dir client/admin-web build
#      （build 内含 scripts/sync-em.mjs，会把 em 白名单 4 文件放进 dist/em/）；
#   2) 真 Rust server 以切换后的 admin_dir 在 127.0.0.1:8000 运行：./dev-server.sh
#      （或 XIAOZHI_ADMIN_DIR=/abs/path/to/client/admin-web/dist 启动 release server）。
# ⚠️ 本机无 GUI / 真机 / 运行中的 server 时**不代跑**，结论标「待验」；本脚本只做
#    HTTP/WS 层断言，不启动 server、不构建、不写仓库文件（临时文件落 mktemp 并清理）。
# ⚠️ 口径勘误：研究 2/2 §3 把 `id="root"` 列为 SPA 独占的负断言，但官方页
#    client/apps/h5App/em/index.html:571 自身就有 `<div id="root"></div>`（React UMD
#    挂载点），按原文做负断言会恒 FAIL。故 SPA 特征改用 `<title>小智管理台</title>`、
#    `/assets/index-` 与 `<script type="module"`（dist/index.html 的 Vite 模块入口）三者。
# ⚠️ 深链是 HashRouter 形态 `/#/config`、`/#/settings`；裸 /config、/settings 不是有效
#    深链，不写进断言（hash 不发给服务端，curl 只能验 `/`）。
#
# 用法：BASE=http://127.0.0.1:8000 ./tools/admin_web_switch_check.sh
#       ./tools/admin_web_switch_check.sh http://127.0.0.1:8000
#       CHECK_WS=1 BASE=http://127.0.0.1:8000 ./tools/admin_web_switch_check.sh  # 含可选 WS 升级握手
# 退出码：全部 PASS → 0；任一 FAIL → 1（server 不可达也算 FAIL）。
set -uo pipefail

# 本机若注入了 HTTP_PROXY，loopback 也可能被代理拦截（实测 curl 127.0.0.1 直接 502），
# 故下面所有请求显式 --noproxy '*'；若 BASE 是必须经代理才能访问的远端，去掉该参数。
BASE="${1:-${BASE:-http://127.0.0.1:8000}}"
BASE="${BASE%/}"
CHECK_WS="${CHECK_WS:-0}"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

PASS_COUNT=0
FAIL_COUNT=0
SKIP_COUNT=0

pass() {
  printf 'PASS: %s\n' "$1"
  PASS_COUNT=$((PASS_COUNT + 1))
}

fail() {
  printf 'FAIL: %s\n' "$1" >&2
  FAIL_COUNT=$((FAIL_COUNT + 1))
}

skip() {
  printf 'SKIP: %s\n' "$1"
  SKIP_COUNT=$((SKIP_COUNT + 1))
}

# 前置：server 必须活着且 `/` 可下载；否则后续断言没有意义，直接给出诚实提示。
ROOT_HTML="$TMP_DIR/root.html"
if ! curl --noproxy '*' -fsS --max-time 10 -D "$TMP_DIR/root.headers" -o "$ROOT_HTML" "$BASE/" \
     2>"$TMP_DIR/root.curl.err"; then
  printf 'FAIL: %s/ 不可达——需先构建 dist 并启动真 server（./dev-server.sh）。curl: %s\n' \
    "$BASE" "$(tr '\n' ' ' <"$TMP_DIR/root.curl.err")" >&2
  printf 'RESULT: FAIL\n'
  exit 1
fi

# ---- 1) `/` 必须返回 admin-web/dist 的 SPA index.html ----
if grep -qF '<title>小智管理台</title>' "$ROOT_HTML" \
   && grep -qF '/assets/index-' "$ROOT_HTML" \
   && grep -qF '<div id="root">' "$ROOT_HTML"; then
  pass "/ 返回 dist/index.html（title + /assets/index- + #root）"
else
  fail "/ 未返回 dist/index.html 特征（admin_dir 可能没切，或 dist 未构建）"
fi
if grep -qF '<title>XiaoZhi Admin</title>' "$ROOT_HTML" \
   || grep -qF 'nativevue2.js' "$ROOT_HTML" \
   || grep -qF 'h5App.js' "$ROOT_HTML"; then
  fail "/ 仍是旧端 h5App/web 的 Kuikly 页（XiaoZhi Admin / 双 bundle）"
else
  pass "/ 不含旧端 Kuikly 页特征"
fi
if grep -qF '管理页面未找到' "$ROOT_HTML"; then
  fail "/ 命中 server 的 admin_missing 中文提示（admin_dir 目录不存在）"
else
  pass "/ 未命中 admin_missing 提示"
fi

# ---- 2) `/em/`（尾斜杠必须）必须返回官方 EchoMuse 页，而不是 SPA fallback ----
EM_HTML="$TMP_DIR/em.html"
if ! curl --noproxy '*' -fsS --max-time 10 -o "$EM_HTML" "$BASE/em/" 2>"$TMP_DIR/em.curl.err"; then
  fail "/em/ 非 2xx：$(tr '\n' ' ' <"$TMP_DIR/em.curl.err")"
else
  if grep -qF '<title>EchoMuse 设备管理</title>' "$EM_HTML" \
     && grep -qF 'static/adapter.js' "$EM_HTML" \
     && grep -qF 'static/dashboard.js' "$EM_HTML" \
     && grep -qF 'static/host.js' "$EM_HTML"; then
    pass "/em/ 是官方页（EchoMuse title + 三个 static 脚本引用）"
  else
    fail "/em/ 缺少官方页 title 或三个 static 脚本引用（em 白名单拷贝可能没进 dist/em）"
  fi
  # 负断言：不得落回 SPA index.html（研究 §3 的「二次挂载」核心症状）。
  if grep -qF '<title>小智管理台</title>' "$EM_HTML" \
     || grep -qF '/assets/index-' "$EM_HTML" \
     || grep -qF '<script type="module"' "$EM_HTML"; then
    fail "/em/ 落到了 SPA index.html（ServeDir fallback / dist/em 缺失）"
  else
    pass "/em/ 未落到 SPA index.html"
  fi
fi

# ---- 3) 三个相对脚本必须真是 JS（首字节非 HTML），否则 iframe 内会 SyntaxError ----
for js_file in adapter.js dashboard.js host.js; do
  JS_OUT="$TMP_DIR/$js_file"
  if ! curl --noproxy '*' -fsS --max-time 10 -o "$JS_OUT" "$BASE/em/static/$js_file" \
       2>"$TMP_DIR/$js_file.curl.err"; then
    fail "/em/static/$js_file 不可达：$(tr '\n' ' ' <"$TMP_DIR/$js_file.curl.err")"
    continue
  fi
  if [[ ! -s "$JS_OUT" ]]; then
    fail "/em/static/$js_file 是空文件"
    continue
  fi
  FIRST_BYTE="$(head -c 120 "$JS_OUT" | tr -d '\r\n' | sed 's/^[[:space:]]*//' | head -c 1)"
  if [[ "$FIRST_BYTE" == '<' ]]; then
    fail "/em/static/$js_file 返回了 HTML（首字节 '<'）"
  else
    pass "/em/static/$js_file 首字节非 HTML（JS 资源）"
  fi
done

# ---- 4) /api/config 必须是 JSON，不能被 SPA fallback 吞成 HTML ----
API_OUT="$TMP_DIR/api-config"
if ! curl --noproxy '*' -fsS --max-time 10 -D "$TMP_DIR/api-config.headers" -o "$API_OUT" \
     "$BASE/api/config" 2>"$TMP_DIR/api-config.curl.err"; then
  fail "/api/config 不可达：$(tr '\n' ' ' <"$TMP_DIR/api-config.curl.err")"
else
  API_CONTENT_TYPE="$(awk 'tolower($1) == "content-type:" { print tolower($2); exit }' \
    "$TMP_DIR/api-config.headers" | tr -d '\r')"
  API_FIRST_BYTE="$(head -c 120 "$API_OUT" | tr -d '\r\n' | sed 's/^[[:space:]]*//' | head -c 1)"
  if [[ "$API_FIRST_BYTE" == '<' || "$API_CONTENT_TYPE" == *html* ]]; then
    fail "/api/config 返回了 HTML（Content-Type=$API_CONTENT_TYPE，首字节='$API_FIRST_BYTE'）"
  else
    pass "/api/config 非 HTML（Content-Type=$API_CONTENT_TYPE）"
  fi
fi

# ---- 5) 可选：/api/admin/ws 升级握手（101 Switching Protocols）----
if [[ "$CHECK_WS" == '1' ]]; then
  WS_HEADERS="$TMP_DIR/ws.headers"
  curl --noproxy '*' -sS --http1.1 --max-time 5 \
    -H 'Connection: Upgrade' -H 'Upgrade: websocket' \
    -H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
    -D "$WS_HEADERS" -o /dev/null "$BASE/api/admin/ws" >/dev/null 2>&1 || true
  if grep -qi '101 Switching Protocols' "$WS_HEADERS" 2>/dev/null; then
    pass '/api/admin/ws 完成 101 WebSocket 升级'
  else
    fail '/api/admin/ws 未见 101 Switching Protocols（可能被 fallback 抢或路由未注册）'
  fi
else
  skip '/api/admin/ws 升级握手（可选，CHECK_WS=1 启用）'
fi

printf 'INFO: 深链需浏览器验证（HashRouter）：%s/#/config 、 %s/#/settings\n' "$BASE" "$BASE"
printf 'INFO: 裸 /config、/settings 不是有效深链，未纳入断言。\n'
printf '\n=== W15-1 验收汇总 ===\nPASS=%d FAIL=%d SKIP=%d BASE=%s\n' \
  "$PASS_COUNT" "$FAIL_COUNT" "$SKIP_COUNT" "$BASE"
if ((FAIL_COUNT > 0)); then
  printf 'RESULT: FAIL\n'
  exit 1
fi
printf 'RESULT: PASS\n'
exit 0
