#!/bin/sh
# xiaozhi-server-rust 容器启动入口
#
# 职责：
#   1. 若启用自动下载且检测到关键模型文件缺失，则从 k2-fsa/sherpa-onnx 官方
#      GitHub Release 拉取**整包 tar.bz2** 并解压到 $XIAOZHI_MODELS_DIR（默认 /data/models）
#      ——单请求拿全（Kokoro 含 espeak-ng-data/dict 共 365 个文件），不做逐文件下载。
#   2. exec 真正的服务器进程，把参数透传下去（保证能收到 SIGTERM 等信号）。
#
# 环境变量：
#   XIAOZHI_MODELS_DIR            模型根目录（默认 /data/models）
#   XIAOZHI_AUTO_DOWNLOAD_MODELS  missing(默认) | force | off
#                                 missing : 仅在关键文件缺失时下载（按模型粒度幂等跳过）
#                                 force   : 忽略已就绪检查，强制重新下载
#                                 off     : 不下载（依赖挂载/预置模型）
#   SENSEVOICE_URL / KOKORO_URL / SILERO_VAD_URL  可覆盖默认下载地址（完整直链，内网镜像用；
#                                 显式覆盖不会被二次套代理）
#   GITHUB_PROXY                  GitHub 代理前缀（**默认空 = 直连原始地址**）。
#                                 是否走代理由 docker compose 决定：配置了 GITHUB_PROXY 才套用
#                                 （部署环境无法直连 github.com 时按需配置）；off 亦表示直连。
#
# 幂等与续传：压缩包先下 .part（curl -C - 断点续传）成功才 mv 到位、按模型校验通过即跳过
# ——中断/重启后只补缺失的模型包，不会全量重拉。
#
# 默认模型包（已核对官方 release 并验证下载解压，与 config.example.toml 路径一一对应）：
#   - SenseVoice INT8 : sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09（model.int8.onnx + tokens.txt）
#   - Kokoro INT8     : kokoro-int8-multi-lang-v1_1（中英双语；model.int8.onnx/voices.bin/
#                       tokens.txt/espeak-ng-data/lexicon-zh.txt/lexicon-us-en.txt/dict）
#   - Silero VAD      : silero_vad.onnx

set -eu

MODELS_DIR="${XIAOZHI_MODELS_DIR:-/data/models}"
AUTO="${XIAOZHI_AUTO_DOWNLOAD_MODELS:-missing}"

SENSEVOICE_URL="${SENSEVOICE_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09.tar.bz2}"
KOKORO_URL="${KOKORO_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-int8-multi-lang-v1_1.tar.bz2}"
SILERO_VAD_URL="${SILERO_VAD_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx}"

# GitHub 直链代理：**默认空 = 直连原始地址（github.com）**；是否走代理由部署方决定——
# docker compose 配置了 GITHUB_PROXY 才套用前缀（直连不可达时表现为 curl 134s 假死）。
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

# 下载单个文件（.part 断点续传，成功才 mv 到位；已存在直接跳过）。
fetch_file() {  # $1=url（未套代理） $2=目标路径
  local part="$2.part"
  [ -s "$2" ] && return 0
  curl $CURL_OPTS -C - -o "$part" "$(apply_proxy "$1")" || { rm -f "$part"; return 1; }
  mv "$part" "$2"
}

# 下载整包 tar.bz2 并解压**覆盖**到目标目录（--strip-components=1 去掉包内顶层目录）。
# 已落盘的压缩包只跳过「下载」，解压仍要执行（防上次"下载成功解压失败"的残留）。
fetch_archive() {  # $1=url $2=解压目标目录 $3=包名（/tmp 下的落盘名）
  if [ ! -s "/tmp/$3" ]; then
    local part="/tmp/$3.part"
    curl $CURL_OPTS -C - -o "$part" "$(apply_proxy "$1")" || { rm -f "$part"; return 1; }
    mv "$part" "/tmp/$3"
  fi
  tar -xjf "/tmp/$3" -C "$2" --strip-components=1 || return 1
  rm -f "/tmp/$3"
}

download_models() {
  echo "[entrypoint] 模型缺失，开始从 GitHub 下载到 $MODELS_DIR（解压覆盖）..."
  if [ -n "$GITHUB_PROXY" ]; then
    echo "[entrypoint] GitHub 代理=$GITHUB_PROXY （GITHUB_PROXY=off 可关闭）"
  fi
  mkdir -p "$MODELS_DIR/SenseVoiceSmall" "$MODELS_DIR/Kokoro"

  echo "[entrypoint] [1/3] Silero VAD"
  fetch_file "$SILERO_VAD_URL" "$MODELS_DIR/silero_vad.onnx" || return 1

  echo "[entrypoint] [2/3] SenseVoice INT8"
  fetch_archive "$SENSEVOICE_URL" "$MODELS_DIR/SenseVoiceSmall" sensevoice.tar.bz2 || return 1

  echo "[entrypoint] [3/3] Kokoro INT8 多语种（en+zh）"
  fetch_archive "$KOKORO_URL" "$MODELS_DIR/Kokoro" kokoro.tar.bz2 || return 1

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
      download_models || {
        echo "[entrypoint] 模型下载失败。无 mock 回退，中止启动。" >&2
        echo "[entrypoint] 请挂载已下载的模型，或检查网络/代理（GITHUB_PROXY）后重试。" >&2
        exit 1
      }
    fi
    ;;
esac

# 配置文件首次引导：容器内无配置时用内置示例生成（全新部署开箱即用）——
# 不生成的话 server 以内置默认启动（config_path=None），PUT /api/config 无法持久化。
CFG="${XIAOZHI_CONFIG:-}"
if [ -n "$CFG" ] && [ ! -e "$CFG" ] && [ -f /app/config.example.toml ]; then
  mkdir -p "$(dirname "$CFG")"
  cp /app/config.example.toml "$CFG"
  echo "[entrypoint] 已生成默认配置 $CFG（可经管理页修改保存）"
fi

# 配置文件可写自检（健壮性）：管理页「保存配置」走 PUT /api/config 回写此文件，
# 只读/属主不对会在保存时报 500——启动即暴露，而不是等用户保存失败。
CFG="${XIAOZHI_CONFIG:-}"
if [ -n "$CFG" ] && [ -e "$CFG" ] && [ ! -w "$CFG" ]; then
  echo "[entrypoint] ⚠️ 配置文件 $CFG 不可写：管理页「保存配置」将失败（500）。" >&2
  echo "[entrypoint]    请检查宿主挂载（勿加 :ro）与文件属主/权限后重启容器。" >&2
fi

echo "[entrypoint] 启动 xiaozhi-server-rust ..."
exec /app/server "$@"
