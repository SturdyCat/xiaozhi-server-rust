#!/usr/bin/env bash
# 下载 xiaozhi-server-rust 所需的本地模型：SenseVoice INT8（ASR）、Kokoro INT8（TTS）、Silero VAD。
# 用法：./scripts/download_models.sh [目标目录，默认 ./models]
#
# 从 k2-fsa/sherpa-onnx 官方 GitHub Release 下载**整包 tar.bz2**并本地解压覆盖
# （单请求拿全，Kokoro 含 espeak-ng-data/dict 共 365 个文件，不做逐文件下载）。
# 容器内也可由 docker-entrypoint.sh 自动完成同样的事情（XIAOZHI_AUTO_DOWNLOAD_MODELS=missing）。
# 本脚本用于「先在宿主机预置、再挂载」的场景，或离线环境手动搬运。
#
# 环境变量：
#   GITHUB_PROXY   GitHub 代理前缀（**默认空 = 直连原始地址**；需要时设为如 https://tvv.tw/）
#   SENSEVOICE_URL / KOKORO_URL / SILERO_VAD_URL  可覆盖默认下载地址（完整直链，内网镜像用）
#
# 默认模型包（已核对官方 release 并验证下载解压，与 config.example.toml 路径一一对应）：
#   - SenseVoice INT8 : sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09（model.int8.onnx + tokens.txt）
#   - Kokoro INT8     : kokoro-int8-multi-lang-v1_1（中英双语；model.int8.onnx/voices.bin/
#                       tokens.txt/espeak-ng-data/lexicon-zh.txt/lexicon-us-en.txt/dict）
#   - Silero VAD      : silero_vad.onnx

set -euo pipefail

MODELS_DIR="${1:-./models}"
SENSEVOICE_URL="${SENSEVOICE_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09.tar.bz2}"
KOKORO_URL="${KOKORO_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-int8-multi-lang-v1_1.tar.bz2}"
SILERO_VAD_URL="${SILERO_VAD_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx}"

# GitHub 直链代理：**默认空 = 直连原始地址**；需要时设置（如 https://tvv.tw/）。
GITHUB_PROXY="${GITHUB_PROXY:-}"
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

CURL_OPTS="-fSL --connect-timeout 15 --retry 3 --retry-delay 2"

fetch_file() {  # $1=url（未套代理） $2=目标路径
  local part="$2.part"
  [ -s "$2" ] && return 0
  curl $CURL_OPTS -C - -o "$part" "$(apply_proxy "$1")" || { rm -f "$part"; return 1; }
  mv "$part" "$2"
}

fetch_archive() {  # $1=url $2=解压目标目录 $3=包名（落盘名）
  # 已落盘的压缩包只跳过「下载」，解压仍要执行（防上次"下载成功解压失败"的残留）
  if [ ! -s "/tmp/$3" ]; then
    local part="/tmp/$3.part"
    curl $CURL_OPTS -C - -o "$part" "$(apply_proxy "$1")" || { rm -f "$part"; return 1; }
    mv "$part" "/tmp/$3"
  fi
  tar -xjf "/tmp/$3" -C "$2" --strip-components=1 || return 1
  rm -f "/tmp/$3"
}

if [ -n "$GITHUB_PROXY" ]; then
  echo "==> GitHub 代理=$GITHUB_PROXY"
fi
mkdir -p "$MODELS_DIR"/SenseVoiceSmall "$MODELS_DIR"/Kokoro

echo "==> [1/3] Silero VAD"
fetch_file "$SILERO_VAD_URL" "$MODELS_DIR/silero_vad.onnx"

echo "==> [2/3] SenseVoice INT8"
fetch_archive "$SENSEVOICE_URL" "$MODELS_DIR"/SenseVoiceSmall sensevoice.tar.bz2

echo "==> [3/3] Kokoro INT8 多语种（en+zh）"
fetch_archive "$KOKORO_URL" "$MODELS_DIR"/Kokoro kokoro.tar.bz2

echo "==> 完成。核对关键文件："
for f in "$MODELS_DIR/silero_vad.onnx" "$MODELS_DIR/SenseVoiceSmall/model.int8.onnx" "$MODELS_DIR/SenseVoiceSmall/tokens.txt" "$MODELS_DIR/Kokoro/model.int8.onnx" "$MODELS_DIR/Kokoro/voices.bin" "$MODELS_DIR/Kokoro/tokens.txt" "$MODELS_DIR/Kokoro/espeak-ng-data" "$MODELS_DIR/Kokoro/lexicon-us-en.txt" "$MODELS_DIR/Kokoro/lexicon-zh.txt" "$MODELS_DIR/Kokoro/dict"; do
  if [ -e "$f" ]; then echo "  [ok]      $f"; else echo "  [MISSING] $f"; fi
done
echo "请确保以上路径与 config.example.toml 的 [asr]/[vad]/[tts] 配置一致。"
