#!/bin/sh
# xiaozhi-server-rust 容器启动入口
#
# 职责：
#   1. 若启用自动下载且检测到关键模型文件缺失，则从 k2-fsa/sherpa-onnx 官方
#      release 拉取并解压到 $XIAOZHI_MODELS_DIR（默认 /models）。
#   2. exec 真正的服务器进程，把参数透传下去（保证能收到 SIGTERM 等信号）。
#
# 环境变量：
#   XIAOZHI_MODELS_DIR            模型根目录（默认 /models）
#   XIAOZHI_AUTO_DOWNLOAD_MODELS  missing(默认) | force | off
#                                   missing : 仅在关键文件缺失时下载
#                                   force   : 每次启动都重新下载
#                                   off     : 不下载（依赖挂载/预置模型）
#   XIAOZHI_ALLOW_MISSING_MODELS  非空时，即便下载失败也仍尝试启动（仅 mock 模式有意义）
#   SENSEVOICE_URL / KOKORO_URL / SILERO_VAD_URL  可覆盖默认下载地址（便于内网镜像）
#   GITHUB_PROXY                  GitHub 代理前缀（默认 https://tvv.tw/）。
#                                 仅对 github.com 直链自动套用；设为 off 或空则直连。
#                                 显式覆盖为内网/代理地址时不会被二次套用。
#
# 适用场景：首次启动容器且 ./models 为空时，自动拉取 SenseVoice/Kokoro/Silero 模型；
#          模型写入挂载目录后会持久化，后续启动检测到文件存在即跳过，避免重复下载。
#
# 默认模型包（已核对官方 release 并完整下载验证，与 config.example.toml 路径一一对应）：
#   - SenseVoice INT8 : sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09（model.int8.onnx）
#   - Kokoro INT8     : kokoro-int8-multi-lang-v1_1（中英双语；model.int8.onnx/voices.bin/
#                       tokens.txt/espeak-ng-data/lexicon-zh.txt/lexicon-us-en.txt/dict）
#   - Silero VAD      : silero_vad.onnx

set -eu

MODELS_DIR="${XIAOZHI_MODELS_DIR:-/models}"
AUTO="${XIAOZHI_AUTO_DOWNLOAD_MODELS:-missing}"

SENSEVOICE_URL="${SENSEVOICE_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09.tar.bz2}"
KOKORO_URL="${KOKORO_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-int8-multi-lang-v1_1.tar.bz2}"
SILERO_VAD_URL="${SILERO_VAD_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx}"

# GitHub 直链代理：部署环境常无法直连 github.com / release-assets.githubusercontent.com
# （表现为 curl 连接 134s 超时）。默认走 tvv.tw 代理；GITHUB_PROXY=off 可关闭。
# 仅对 github.com 开头的 URL 套前缀，用户覆盖为内网镜像/代理地址时不受影响。
GITHUB_PROXY="${GITHUB_PROXY:-https://tvv.tw/}"
case "$GITHUB_PROXY" in
  off|OFF|"") GITHUB_PROXY="" ;;
esac

apply_proxy() {
  case "$1" in
    http://github.com/*|https://github.com/*)
      [ -n "$GITHUB_PROXY" ] && printf '%s' "$GITHUB_PROXY$1" || printf '%s' "$1"
      ;;
    *) printf '%s' "$1" ;;
  esac
}

# curl 选项：连接 15s 超时（避免 134s 假死）+ 失败重试 3 次
CURL_OPTS="-fSL --connect-timeout 15 --retry 3 --retry-delay 2"

# 模型就绪“代表性”检查点：缺失任一即视为未下载。
# 注意：官方包内模型文件名为 model.int8.onnx（不是 model.onnx）。
CHECKPOINTS="
$MODELS_DIR/silero_vad.onnx
$MODELS_DIR/SenseVoiceSmall/model.int8.onnx
$MODELS_DIR/SenseVoiceSmall/tokens.txt
$MODELS_DIR/Kokoro/model.int8.onnx
$MODELS_DIR/Kokoro/voices.bin
$MODELS_DIR/Kokoro/tokens.txt
$MODELS_DIR/Kokoro/espeak-ng-data
$MODELS_DIR/Kokoro/lexicon-us-en.txt
$MODELS_DIR/Kokoro/lexicon-zh.txt
"

echo "[entrypoint] 模型目录=$MODELS_DIR  自动下载模式=$AUTO"

all_present() {
  for f in $CHECKPOINTS; do
    [ -e "$f" ] || return 1
  done
  return 0
}

verify_models() {
  missing=0
  for f in $CHECKPOINTS; do
    if [ -e "$f" ]; then
      echo "  [ok]      $f"
    else
      echo "  [MISSING] $f"
      missing=1
    fi
  done
  return $missing
}

download_models() {
  echo "[entrypoint] 开始下载模型到 $MODELS_DIR ..."
  if [ -n "$GITHUB_PROXY" ]; then
    echo "[entrypoint] GitHub 代理=$GITHUB_PROXY （GITHUB_PROXY=off 可关闭）"
  fi
  mkdir -p "$MODELS_DIR/SenseVoiceSmall" "$MODELS_DIR/Kokoro"

  echo "[entrypoint] [1/3] Silero VAD"
  curl $CURL_OPTS -o "$MODELS_DIR/silero_vad.onnx" "$(apply_proxy "$SILERO_VAD_URL")" || return 1

  echo "[entrypoint] [2/3] SenseVoice INT8"
  curl $CURL_OPTS -o /tmp/sensevoice.tar.bz2 "$(apply_proxy "$SENSEVOICE_URL")" || return 1
  tar -xjf /tmp/sensevoice.tar.bz2 -C "$MODELS_DIR/SenseVoiceSmall" --strip-components=1 || return 1
  rm -f /tmp/sensevoice.tar.bz2

  echo "[entrypoint] [3/3] Kokoro INT8 多语种（en+zh）"
  curl $CURL_OPTS -o /tmp/kokoro.tar.bz2 "$(apply_proxy "$KOKORO_URL")" || return 1
  tar -xjf /tmp/kokoro.tar.bz2 -C "$MODELS_DIR/Kokoro" --strip-components=1 || return 1
  rm -f /tmp/kokoro.tar.bz2

  echo "[entrypoint] 校验关键文件："
  if verify_models; then
    echo "[entrypoint] 模型文件齐全。"
  else
    echo "[entrypoint] 错误：部分模型文件缺失（下载或解压失败）。" >&2
    echo "[entrypoint] 请检查 KOKORO_URL/SENSEVOICE_URL 是否指向了正确的模型包，" >&2
    echo "[entrypoint] 或比对 config.example.toml 的 [asr]/[vad]/[tts] 路径。" >&2
    return 1
  fi
}

case "$AUTO" in
  off)
    echo "[entrypoint] 自动下载已关闭（XIAOZHI_AUTO_DOWNLOAD_MODELS=off）"
    ;;
  force)
    download_models
    ;;
  *)
    if all_present; then
      echo "[entrypoint] 模型已就绪，跳过下载"
    else
      if download_models; then
        :
      else
        echo "[entrypoint] 模型下载失败。" >&2
        if [ -n "${XIAOZHI_ALLOW_MISSING_MODELS:-}" ]; then
          echo "[entrypoint] XIAOZHI_ALLOW_MISSING_MODELS 已设置，仍尝试启动（仅 mock 模式有意义）。"
        else
          echo "[entrypoint] 请挂载已下载的模型，或检查网络/镜像后重试。中止启动。" >&2
          exit 1
        fi
      fi
    fi
    ;;
esac

echo "[entrypoint] 启动 xiaozhi-server-rust ..."
exec /app/server "$@"
