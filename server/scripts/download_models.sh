#!/usr/bin/env bash
# 下载 xiaozhi-server-rust 所需的本地模型：SenseVoice INT8（ASR）、Kokoro INT8（TTS）、Silero VAD。
# 用法：./scripts/download_models.sh [目标目录，默认 ./models]
#
# 容器内也可由 docker-entrypoint.sh 自动完成同样的事情（XIAOZHI_AUTO_DOWNLOAD_MODELS=missing）。
# 本脚本用于「先在宿主机预置、再挂载」的场景，或离线环境手动搬运。
#
# 模型来源：HuggingFace 的 csukuangfj 官方镜像仓库（无需代理，环境需能访问 huggingface.co）。
#   - SenseVoice INT8 : sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09（model.int8.onnx + tokens.txt）
#   - Kokoro INT8     : kokoro-int8-multi-lang-v1_1（中英双语，含 lexicon-zh/dict/双 lexicon/espeak-ng-data）
#   - Silero VAD      : silero_vad.onnx
# 镜像站点/内网不可达时，把 SENSEVOICE_URL/KOKORO_URL/SILERO_VAD_URL 改成你的镜像仓库 ID 即可。

set -euo pipefail

MODELS_DIR="${1:-./models}"
SENSEVOICE_URL="${SENSEVOICE_URL:-csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09}"
KOKORO_URL="${KOKORO_URL:-csukuangfj/kokoro-int8-multi-lang-v1_1}"
SILERO_VAD_URL="${SILERO_VAD_URL:-csukuangfj/vad}"

# ============ HuggingFace 直连下载（无需代理）============

hf_list() {
  curl -fsSL "https://huggingface.co/api/models/$1/tree/main?recursive=true" \
    | grep '"type": *"file"' | grep '"path":' \
    | sed 's/.*"path":[[:space:]]*"\([^"]*\)".*/\1/'
}

hf_sync() {
  local repo="$1" dest="$2" pat="${3:-}"   # $3 可选：仅下载文件名精确匹配 pat 的条目
  echo "==> 从 HuggingFace 同步 $repo → $dest${pat:+"（仅 $pat）"}"
  local list="/tmp/hf_$(echo "$repo" | tr '/' '_')_files"
  hf_list "$repo" > "$list"
  if [ ! -s "$list" ]; then
    echo "错误：无法从 HuggingFace 列出 $repo 的文件（仓库名是否正确？能否访问 huggingface.co？）" >&2
    rm -f "$list"
    return 1
  fi
  while IFS= read -r p; do
    [ -z "$p" ] && continue
    [ -n "$pat" ] && [ "$p" != "$pat" ] && continue   # 仅取指定文件（如 silero_vad.onnx），跳过仓库内无关文件
    local target="$dest/$p"
    mkdir -p "$(dirname "$target")"
    echo "    ↓ $p"
    curl $CURL_OPTS -o "$target" "https://huggingface.co/$repo/resolve/main/$p" || return 1
  done < "$list"
  rm -f "$list"
}

CURL_OPTS="-fSL --connect-timeout 15 --retry 3 --retry-delay 2"

mkdir -p "$MODELS_DIR"/SenseVoiceSmall "$MODELS_DIR"/Kokoro

echo "==> [1/3] Silero VAD"
hf_sync "$SILERO_VAD_URL" "$MODELS_DIR" "silero_vad.onnx" || exit 1

echo "==> [2/3] SenseVoice INT8"
hf_sync "$SENSEVOICE_URL" "$MODELS_DIR"/SenseVoiceSmall || exit 1

echo "==> [3/3] Kokoro INT8 多语种（en+zh）"
hf_sync "$KOKORO_URL" "$MODELS_DIR"/Kokoro || exit 1

echo "==> 完成。核对关键文件："
for f in "$MODELS_DIR/silero_vad.onnx" "$MODELS_DIR/SenseVoiceSmall/model.int8.onnx" "$MODELS_DIR/SenseVoiceSmall/tokens.txt" "$MODELS_DIR/Kokoro/model.int8.onnx" "$MODELS_DIR/Kokoro/voices.bin" "$MODELS_DIR/Kokoro/tokens.txt" "$MODELS_DIR/Kokoro/espeak-ng-data" "$MODELS_DIR/Kokoro/lexicon-us-en.txt" "$MODELS_DIR/Kokoro/lexicon-zh.txt"; do
  if [ -e "$f" ]; then echo "  [ok]      $f"; else echo "  [MISSING] $f"; fi
done
echo "请确保以上路径与 config.example.toml 的 [asr]/[vad]/[tts] 配置一致。"
