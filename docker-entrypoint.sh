#!/bin/sh
# xiaozhi-server-rust 容器启动入口
#
# 职责：
#   1. 若启用自动下载且检测到关键模型文件缺失，则从 HuggingFace（csukuangfj 官方镜像仓库）
#      拉取并写入 $XIAOZHI_MODELS_DIR（默认 /models）。
#   2. exec 真正的服务器进程，把参数透传下去（保证能收到 SIGTERM 等信号）。
#
# 环境变量：
#   XIAOZHI_MODELS_DIR            模型根目录（默认 /models）
#   XIAOZHI_AUTO_DOWNLOAD_MODELS  missing(默认) | force | off
#                                 missing : 仅在关键文件缺失时下载
#                                 force   : 每次启动都重新下载
#                                 off     : 不下载（依赖挂载/预置模型）
#   SENSEVOICE_URL / KOKORO_URL / SILERO_VAD_URL  可覆盖默认 HuggingFace 仓库 ID
#                                 （形如 csukuangfj/xxx）；指向你的内网镜像仓库即可走内网。
#   ⚠️ 模型统一从 HuggingFace 直连下载，无需任何代理。
#
# 适用场景：首次启动容器且 ./models 为空时，自动拉取 SenseVoice/Kokoro/Silero 模型；
#          模型写入挂载目录后会持久化，后续启动检测到文件存在即跳过，避免重复下载。
#          下载为**显式清单 + 逐文件幂等**（已存在跳过、.part 断点续传），中断后重启只补缺失。
#
# 默认模型包（已核对 csukuangfj 官方 HuggingFace 镜像并完整下载验证，与 config.example.toml 路径一一对应）：
#   - SenseVoice INT8 : sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09（model.int8.onnx + tokens.txt）
#   - Kokoro INT8     : kokoro-int8-multi-lang-v1_1（中英双语；model.int8.onnx/voices.bin/
#                       tokens.txt/espeak-ng-data/lexicon-zh.txt/lexicon-us-en.txt/dict）
#   - Silero VAD      : silero_vad.onnx

set -eu

MODELS_DIR="${XIAOZHI_MODELS_DIR:-/models}"
AUTO="${XIAOZHI_AUTO_DOWNLOAD_MODELS:-missing}"

# 模型统一从 HuggingFace 直连（csukuangfj 官方镜像仓库），无需代理。
# 三个变量现为「HuggingFace 仓库 ID」，可被内网镜像仓库覆盖。
SENSEVOICE_URL="${SENSEVOICE_URL:-csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09}"
KOKORO_URL="${KOKORO_URL:-csukuangfj/kokoro-int8-multi-lang-v1_1}"
SILERO_VAD_URL="${SILERO_VAD_URL:-csukuangfj/vad}"

# ============ HuggingFace 直连下载（无需代理）============
# 模型统一从 huggingface.co 的 csukuangfj 官方镜像仓库拉取；环境需能访问 huggingface.co。
# 镜像站点/内网不可达时，把 SENSEVOICE_URL/KOKORO_URL/SILERO_VAD_URL 改成你的镜像仓库 ID 即可。
#
# ⚠️ 只下载**显式清单**内的文件（与 config.example.toml 路径一一对应），不做全仓库同步：
#    SenseVoice 仓库还含 fp32 model.onnx（约 900MB）与 test_wavs，全拉纯属浪费带宽。
#    每个文件幂等（已存在跳过）+ .part 断点续传：首次下载中途失败/重启时逐文件收敛，
#    不会"每次重启都全量重拉、永远凑不齐"。

# 列出仓库（可选子目录）内全部文件，每行一个 repo 相对路径。
# ⚠️ HF tree API 返回**单行紧凑 JSON**——必须先把对象拆行再按行过滤，
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

# 下载单个文件（幂等）：已存在跳过；先下 .part（curl -C - 断点续传），成功才 mv 到位。
# ⚠️ .part 的点只加在**文件名**上（dict/README.md → dict/.README.md.part）——
#    若加在整条相对路径开头会变成隐藏目录 .dict/，目录不存在导致 curl 写入失败（实测）。
hf_fetch() {
  local repo="$1" p="$2" dest="$3"
  local target="$dest/$p"
  local part="$dest/$(dirname "$p")/.$(basename "$p").part"
  if [ -s "$target" ]; then
    echo "[entrypoint]   [skip] $p（已存在）"
    return 0
  fi
  mkdir -p "$(dirname "$target")"
  echo "[entrypoint]   ↓ $p"
  curl $CURL_OPTS -C - -o "$part" "https://huggingface.co/$repo/resolve/main/$p" \
    || { rm -f "$part"; return 1; }
  mv "$part" "$target"
}

# 下载子目录内全部文件（espeak-ng-data / dict 这类目录树），保持相对路径。
hf_fetch_dir() {
  local repo="$1" sub="$2" dest="$3"
  local list="/tmp/hf_$(echo "$repo/$sub" | tr '/' '_')_files"
  hf_list "$repo" "$sub" > "$list"
  if [ ! -s "$list" ]; then
    echo "[entrypoint] 错误：无法从 HuggingFace 列出 $repo/$sub 的文件（仓库名是否正确？能否访问 huggingface.co？）" >&2
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

# curl 选项：连接 15s 超时（避免 134s 假死）+ 失败重试 3 次
CURL_OPTS="-fSL --connect-timeout 15 --retry 3 --retry-delay 2"

# 模型就绪“代表性”检查点：缺失任一即视为未下载。
# 注意：官方包内模型文件名为 model.int8.onnx（不是 model.onnx）；dict 目录为 [tts].dict_dir 所需。
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
$MODELS_DIR/Kokoro/dict
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
  echo "[entrypoint] 开始从 HuggingFace 下载模型到 $MODELS_DIR ..."
  mkdir -p "$MODELS_DIR/SenseVoiceSmall" "$MODELS_DIR/Kokoro"

  echo "[entrypoint] [1/3] Silero VAD"
  hf_fetch "$SILERO_VAD_URL" "silero_vad.onnx" "$MODELS_DIR" || return 1

  echo "[entrypoint] [2/3] SenseVoice INT8"
  hf_fetch "$SENSEVOICE_URL" "model.int8.onnx" "$MODELS_DIR/SenseVoiceSmall" || return 1
  hf_fetch "$SENSEVOICE_URL" "tokens.txt" "$MODELS_DIR/SenseVoiceSmall" || return 1

  echo "[entrypoint] [3/3] Kokoro INT8 多语种（en+zh）"
  for f in model.int8.onnx voices.bin tokens.txt lexicon-us-en.txt lexicon-zh.txt; do
    hf_fetch "$KOKORO_URL" "$f" "$MODELS_DIR/Kokoro" || return 1
  done
  # 目录树：espeak-ng-data（英文 g2p 数据）、dict（中文分词词典，[tts].dict_dir 需要）
  hf_fetch_dir "$KOKORO_URL" "espeak-ng-data" "$MODELS_DIR/Kokoro" || return 1
  hf_fetch_dir "$KOKORO_URL" "dict" "$MODELS_DIR/Kokoro" || return 1

  echo "[entrypoint] 校验关键文件："
  if verify_models; then
    echo "[entrypoint] 模型文件齐全。"
  else
    echo "[entrypoint] 错误：部分模型文件缺失（下载失败或仓库内容不符）。" >&2
    echo "[entrypoint] 请检查 SENSEVOICE_URL/KOKORO_URL/SILERO_VAD_URL 是否指向正确的 HuggingFace 仓库，" >&2
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
        echo "[entrypoint] 模型下载失败。无 mock 回退，中止启动。" >&2
        echo "[entrypoint] 请挂载已下载的模型，或检查网络/镜像后重试。" >&2
        exit 1
      fi
    fi
    ;;
esac

echo "[entrypoint] 启动 xiaozhi-server-rust ..."
exec /app/server "$@"
