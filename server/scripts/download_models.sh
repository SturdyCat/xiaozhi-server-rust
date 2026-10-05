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
# ⚠️ 只下载**显式清单**内的文件（与 config.example.toml 路径一一对应），不做全仓库同步：
#    SenseVoice 仓库还含 fp32 model.onnx（约 900MB）与 test_wavs，全拉纯属浪费带宽。
#    每个文件幂等（已存在跳过）+ .part 断点续传，重跑脚本只补缺失文件。
# ⚠️ HF tree API 返回**单行紧凑 JSON**——hf_list 必须先把对象拆行再按行过滤，
#    否则贪婪 sed 只取到整个数组最后一个 path（实测每仓库只下一个文件的根因）。
#    换行用「反斜杠+真实换行」写法，BSD/GNU/busybox sed 通用（\n 转义 BSD 不认）。

hf_list() {
  local repo="$1" sub="${2:-}"
  local url="https://huggingface.co/api/models/$repo/tree/main"
  [ -n "$sub" ] && url="$url/$sub"
  curl -fsSL --connect-timeout 15 "$url?recursive=true" \
    | sed 's/},{/}\
{/g' \
    | grep '"type": *"file"' \
    | sed 's/.*"path":[[:space:]]*"\([^"]*\)".*/\1/'
}

hf_fetch() {
  local repo="$1" p="$2" dest="$3"
  local target="$dest/$p"
  local part="$dest/$(dirname "$p")/.$(basename "$p").part"
  if [ -s "$target" ]; then
    echo "    [skip] $p（已存在）"
    return 0
  fi
  mkdir -p "$(dirname "$target")"
  echo "    ↓ $p"
  curl $CURL_OPTS -C - -o "$part" "https://huggingface.co/$repo/resolve/main/$p" \
    || { rm -f "$part"; return 1; }
  mv "$part" "$target"
}

hf_fetch_dir() {
  local repo="$1" sub="$2" dest="$3"
  local list="/tmp/hf_$(echo "$repo/$sub" | tr '/' '_')_files"
  # || true：本脚本开 pipefail，curl/grep 失败会让 hf_list 返回非零——
  # 让下面的空清单检查给出友好报错，而不是 set -e 直接退场
  hf_list "$repo" "$sub" > "$list" || true
  if [ ! -s "$list" ]; then
    echo "错误：无法从 HuggingFace 列出 $repo/$sub 的文件（仓库名是否正确？能否访问 huggingface.co？）" >&2
    rm -f "$list"
    return 1
  fi
  local p
  while IFS= read -r p; do
    [ -z "$p" ] && continue
    hf_fetch "$repo" "$p" "$dest" || { rm -f "$list"; return 1; }
  done < "$list"
  rm -f "$list"
}

CURL_OPTS="-fSL --connect-timeout 15 --retry 3 --retry-delay 2"

mkdir -p "$MODELS_DIR"/SenseVoiceSmall "$MODELS_DIR"/Kokoro

echo "==> [1/3] Silero VAD"
hf_fetch "$SILERO_VAD_URL" "silero_vad.onnx" "$MODELS_DIR" || exit 1

echo "==> [2/3] SenseVoice INT8"
hf_fetch "$SENSEVOICE_URL" "model.int8.onnx" "$MODELS_DIR"/SenseVoiceSmall || exit 1
hf_fetch "$SENSEVOICE_URL" "tokens.txt" "$MODELS_DIR"/SenseVoiceSmall || exit 1

echo "==> [3/3] Kokoro INT8 多语种（en+zh）"
for f in model.int8.onnx voices.bin tokens.txt lexicon-us-en.txt lexicon-zh.txt; do
  hf_fetch "$KOKORO_URL" "$f" "$MODELS_DIR"/Kokoro || exit 1
done
hf_fetch_dir "$KOKORO_URL" "espeak-ng-data" "$MODELS_DIR"/Kokoro || exit 1
hf_fetch_dir "$KOKORO_URL" "dict" "$MODELS_DIR"/Kokoro || exit 1

echo "==> 完成。核对关键文件："
for f in "$MODELS_DIR/silero_vad.onnx" "$MODELS_DIR/SenseVoiceSmall/model.int8.onnx" "$MODELS_DIR/SenseVoiceSmall/tokens.txt" "$MODELS_DIR/Kokoro/model.int8.onnx" "$MODELS_DIR/Kokoro/voices.bin" "$MODELS_DIR/Kokoro/tokens.txt" "$MODELS_DIR/Kokoro/espeak-ng-data" "$MODELS_DIR/Kokoro/lexicon-us-en.txt" "$MODELS_DIR/Kokoro/lexicon-zh.txt" "$MODELS_DIR/Kokoro/dict"; do
  if [ -e "$f" ]; then echo "  [ok]      $f"; else echo "  [MISSING] $f"; fi
done
echo "请确保以上路径与 config.example.toml 的 [asr]/[vad]/[tts] 配置一致。"
