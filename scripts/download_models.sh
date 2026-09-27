#!/usr/bin/env bash
# 下载 xiaozhi-server-rust 所需的本地模型：SenseVoice INT8（ASR）、Kokoro（TTS）、Silero VAD。
# 用法：./scripts/download_models.sh [目标目录，默认 /host/models]
#
# 注意：具体 release 文件名以 k2-fsa/sherpa-onnx 官方发布页为准（https://github.com/k2-fsa/sherpa-onnx/releases）。
# 下面给出常见命名示例，请按需替换实际版本号与文件名。

set -euo pipefail

MODELS_DIR="${1:-/host/models}"
mkdir -p "$MODELS_DIR"/SenseVoiceSmall "$MODELS_DIR"/Kokoro

echo "==> 下载 Silero VAD"
curl -L -o "$MODELS_DIR/silero_vad.onnx" \
  "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx"

echo "==> 下载 SenseVoice INT8（中文/英/日/韩/粤）"
# 示例：sherpa-onnx-sense-voice-zh-en-ja-ko-2025-02-16（int8）
curl -L -o /tmp/sensevoice.zip \
  "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-2025-02-16.tar.bz2"
tar -xjf /tmp/sensevoice.zip -C "$MODELS_DIR"/SenseVoiceSmall
# 解压后确认 model.onnx / tokens.txt 路径与 config.example.toml 的 [asr] 一致。

echo "==> 下载 Kokoro 多语种 INT8（含 voices.bin / tokens / espeak-ng-data / dict / lexicon）"
# 示例：kokoro-multi-lang-v1_0
curl -L -o /tmp/kokoro.zip \
  "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-multi-lang-v1_0.tar.bz2"
tar -xjf /tmp/kokoro.zip -C "$MODELS_DIR"/Kokoro

echo "==> 完成。模型目录："
ls -R "$MODELS_DIR" | head -40
echo "请按 config.example.toml 中的 [asr]/[tts] 路径核对 model/tokens/voices 等文件位置。"
