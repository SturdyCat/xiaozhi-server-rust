#!/usr/bin/env bash
# 下载 xiaozhi-server-rust 所需的本地模型：SenseVoice INT8（ASR）、Kokoro INT8（TTS）、Silero VAD。
# 用法：./scripts/download_models.sh [目标目录，默认 ./models]
#
# 容器内也可由 docker-entrypoint.sh 自动完成同样的事情（XIAOZHI_AUTO_DOWNLOAD_MODELS=missing）。
# 本脚本用于「先在宿主机预置、再挂载」的场景，或离线环境手动搬运。
#
# 模型来源：k2-fsa/sherpa-onnx 官方 release（URL 以官方发布页为准）。
#   - SenseVoice INT8 : sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09
#   - Kokoro INT8     : kokoro-int8-en-v0_19（含 model/voices/tokens/espeak-ng-data/双 lexicon）
#   - Silero VAD      : silero_vad.onnx

set -euo pipefail

MODELS_DIR="${1:-./models}"
SENSEVOICE_URL="${SENSEVOICE_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09.tar.bz2}"
KOKORO_URL="${KOKORO_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-int8-en-v0_19.tar.bz2}"
SILERO_VAD_URL="${SILERO_VAD_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx}"

mkdir -p "$MODELS_DIR"/SenseVoiceSmall "$MODELS_DIR"/Kokoro

echo "==> [1/3] Silero VAD"
curl -fSL -o "$MODELS_DIR/silero_vad.onnx" "$SILERO_VAD_URL"

echo "==> [2/3] SenseVoice INT8"
curl -fSL -o /tmp/sensevoice.tar.bz2 "$SENSEVOICE_URL"
tar -xjf /tmp/sensevoice.tar.bz2 -C "$MODELS_DIR"/SenseVoiceSmall --strip-components=1
rm -f /tmp/sensevoice.tar.bz2

echo "==> [3/3] Kokoro INT8 多语种（en+zh）"
curl -fSL -o /tmp/kokoro.tar.bz2 "$KOKORO_URL"
tar -xjf /tmp/kokoro.tar.bz2 -C "$MODELS_DIR"/Kokoro --strip-components=1
rm -f /tmp/kokoro.tar.bz2

echo "==> 完成。核对关键文件："
for f in "$MODELS_DIR/silero_vad.onnx" "$MODELS_DIR/SenseVoiceSmall/tokens.txt" "$MODELS_DIR/SenseVoiceSmall/model.onnx" "$MODELS_DIR/Kokoro/model.onnx" "$MODELS_DIR/Kokoro/voices.bin" "$MODELS_DIR/Kokoro/tokens.txt" "$MODELS_DIR/Kokoro/espeak-ng-data" "$MODELS_DIR/Kokoro/lexicon-us-en.txt" "$MODELS_DIR/Kokoro/lexicon-zh.txt"; do
  if [ -e "$f" ]; then echo "  [ok]      $f"; else echo "  [MISSING] $f"; fi
done
echo "请确保以上路径与 config.example.toml 的 [asr]/[vad]/[tts] 配置一致。"
