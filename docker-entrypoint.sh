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
#
# 适用场景：首次启动容器且 ./models 为空时，自动拉取 SenseVoice/Kokoro/Silero 模型；
#          模型写入挂载目录后会持久化，后续启动检测到文件存在即跳过，避免重复下载。
#
# 默认模型包（已核对官方 release，与 config.example.toml 路径一一对应）：
#   - SenseVoice INT8 : sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09
#   - Kokoro INT8     : kokoro-int8-en-v0_19（含 model/voices/tokens/espeak-ng-data/双 lexicon）
#   - Silero VAD      : silero_vad.onnx

set -eu

MODELS_DIR="${XIAOZHI_MODELS_DIR:-/models}"
AUTO="${XIAOZHI_AUTO_DOWNLOAD_MODELS:-missing}"

SENSEVOICE_URL="${SENSEVOICE_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09.tar.bz2}"
KOKORO_URL="${KOKORO_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-int8-en-v0_19.tar.bz2}"
SILERO_VAD_URL="${SILERO_VAD_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx}"

# 模型就绪“代表性”检查点：缺失任一即视为未下载。
CHECKPOINTS="
$MODELS_DIR/silero_vad.onnx
$MODELS_DIR/SenseVoiceSmall/tokens.txt
$MODELS_DIR/SenseVoiceSmall/model.onnx
$MODELS_DIR/Kokoro/model.onnx
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
  mkdir -p "$MODELS_DIR/SenseVoiceSmall" "$MODELS_DIR/Kokoro"

  echo "[entrypoint] [1/3] Silero VAD"
  curl -fSL -o "$MODELS_DIR/silero_vad.onnx" "$SILERO_VAD_URL"

  echo "[entrypoint] [2/3] SenseVoice INT8"
  curl -fSL -o /tmp/sensevoice.tar.bz2 "$SENSEVOICE_URL"
  tar -xjf /tmp/sensevoice.tar.bz2 -C "$MODELS_DIR/SenseVoiceSmall" --strip-components=1
  rm -f /tmp/sensevoice.tar.bz2

  echo "[entrypoint] [3/3] Kokoro INT8 多语种（en+zh）"
  curl -fSL -o /tmp/kokoro.tar.bz2 "$KOKORO_URL"
  tar -xjf /tmp/kokoro.tar.bz2 -C "$MODELS_DIR/Kokoro" --strip-components=1
  rm -f /tmp/kokoro.tar.bz2

  echo "[entrypoint] 校验关键文件："
  if verify_models; then
    echo "[entrypoint] 模型文件齐全。"
  else
    echo "[entrypoint] 警告：部分模型文件缺失，服务器启动后相关引擎会报错。" >&2
    echo "[entrypoint] 请检查 KOKORO_URL/SENSEVOICE_URL 是否指向了正确的模型包，" >&2
    echo "[entrypoint] 或比对 config.example.toml 的 [asr]/[vad]/[tts] 路径。" >&2
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
