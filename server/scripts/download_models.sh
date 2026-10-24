#!/usr/bin/env bash
# 下载 xiaozhi-server-rust 所需的本地模型：SenseVoice INT8（ASR）、Kokoro INT8（TTS）、
# Silero VAD，以及**下发到 EchoMuse 设备**的语音门模型（oww/silero_vad.onnx）。
# 用法：./scripts/download_models.sh [目标目录，默认 ./models]
#
# 从 k2-fsa/sherpa-onnx 官方 GitHub Release 下载**整包 tar.bz2**并本地解压覆盖
# （单请求拿全，Kokoro 含 espeak-ng-data/dict 共 365 个文件，不做逐文件下载）。
# 容器内也可由 docker-entrypoint.sh 自动完成同样的事情（XIAOZHI_AUTO_DOWNLOAD_MODELS=missing）。
# 本脚本用于「先在宿主机预置、再挂载」的场景，或离线环境手动搬运。
#
# 环境变量：
#   GITHUB_PROXY   GitHub 代理前缀（**默认空 = 直连原始地址**；需要时设为如 https://tvv.tw/）
#   SENSEVOICE_URL / KOKORO_URL / SILERO_VAD_URL / OWW_VAD_URL  可覆盖下载地址（内网镜像用）
#
# 默认模型包（已核对官方 release 并验证下载解压，与 config.example.toml 路径一一对应）：
#   - SenseVoice INT8 : sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09（model.int8.onnx + tokens.txt）
#   - Kokoro INT8     : kokoro-int8-multi-lang-v1_1（中英双语；model.int8.onnx/voices.bin/
#                       tokens.txt/espeak-ng-data/lexicon-zh.txt/lexicon-us-en.txt/dict）
#   - Silero VAD      : silero_vad.onnx（服务端 ASR 切段用）
#   - OWW VAD         : oww/silero_vad.onnx（设备语音门，Silero v5；服务端读取时转 typed 布局）

set -euo pipefail

MODELS_DIR="${1:-./models}"
SENSEVOICE_URL="${SENSEVOICE_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09.tar.bz2}"
KOKORO_URL="${KOKORO_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-int8-multi-lang-v1_1.tar.bz2}"
SILERO_VAD_URL="${SILERO_VAD_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx}"
# 设备语音门要的是 Silero v5（input+sr+h/c），与上面 sherpa 那份用途不同、路径也分开。
# 来源与上游 controller/Dockerfile 的 silero 阶段一致（openWakeWord v0.5.1 release）。
OWW_VAD_URL="${OWW_VAD_URL:-https://github.com/dscripka/openWakeWord/releases/download/v0.5.1/silero_vad.onnx}"
OWW_VAD_SHA256="a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28"

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

# 与 $4 的 sha256 比对；不符就删掉（宁可重下也不留错文件）。宿主机没有 sha256sum
# 时（macOS 默认没有）退回 shasum，两者都没有则跳过校验并提示。
fetch_file_verified() {  # $1=url $2=目标路径 $3=用于提示的名字 $4=期望 sha256
  fetch_file "$1" "$2" || return 1
  local got=""
  if command -v sha256sum >/dev/null 2>&1; then
    got="$(sha256sum "$2" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    got="$(shasum -a 256 "$2" | awk '{print $1}')"
  else
    echo "  [WARN] 找不到 sha256sum/shasum，跳过 $3 摘要校验"
    return 0
  fi
  if [ "$got" != "$4" ]; then
    echo "  [FAIL] $3 摘要不符：期望 $4 实得 $got（已删除，请检查来源/代理）" >&2
    rm -f "$2"
    return 1
  fi
  echo "  [ok] $3 sha256=$got"
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
mkdir -p "$MODELS_DIR"/SenseVoiceSmall "$MODELS_DIR"/Kokoro "$MODELS_DIR"/oww

echo "==> [1/4] Silero VAD（服务端切段）"
fetch_file "$SILERO_VAD_URL" "$MODELS_DIR/silero_vad.onnx"

echo "==> [2/4] SenseVoice INT8"
fetch_archive "$SENSEVOICE_URL" "$MODELS_DIR"/SenseVoiceSmall sensevoice.tar.bz2

echo "==> [3/4] Kokoro INT8 多语种（en+zh）"
fetch_archive "$KOKORO_URL" "$MODELS_DIR"/Kokoro kokoro.tar.bz2

echo "==> [4/4] OWW VAD（下发到 EchoMuse 设备的语音门，可选）"
# 缺了不影响服务端 ASR/TTS，只是「安装 VAD」接口没料可发，所以失败不中止。
# 服务端也会在 [echomuse].enabled=true 时自己下（同一来源、同一摘要），这里只是
# 供「先预置模型卷再挂载」或离线搬运。
fetch_file_verified "$OWW_VAD_URL" "$MODELS_DIR/oww/silero_vad.onnx" "oww/silero_vad.onnx" "$OWW_VAD_SHA256" \
  || echo "  [WARN] OWW VAD 下载/校验失败：设备语音门将退化为 RMS 门限（可用 OWW_VAD_URL 换源）"

echo "==> 完成。核对关键文件："
for f in "$MODELS_DIR/silero_vad.onnx" "$MODELS_DIR/SenseVoiceSmall/model.int8.onnx" "$MODELS_DIR/SenseVoiceSmall/tokens.txt" "$MODELS_DIR/Kokoro/model.int8.onnx" "$MODELS_DIR/Kokoro/voices.bin" "$MODELS_DIR/Kokoro/tokens.txt" "$MODELS_DIR/Kokoro/espeak-ng-data" "$MODELS_DIR/Kokoro/lexicon-us-en.txt" "$MODELS_DIR/Kokoro/lexicon-zh.txt" "$MODELS_DIR/Kokoro/dict" "$MODELS_DIR/oww/silero_vad.onnx"; do
  if [ -e "$f" ]; then echo "  [ok]      $f"; else echo "  [MISSING] $f"; fi
done
echo "请确保以上路径与 config.example.toml 的 [asr]/[vad]/[tts] 配置一致。"
