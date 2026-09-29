# agents.md — 给 AI 编码助手的项目指引

> 本文件供 CodeBuddy / Claude Code / Cursor / Codex 等 AI 编码助手在本仓库工作时阅读。
> 它聚焦**本仓库特有的环境坑与协议约束**，通用 Rust 知识不在此赘述。

## 1. 项目是什么

`xiaozhi-server-rust` 是一个用 Rust 实现的 **小智（xiaozhi-esp32）语音终端服务端**：

- 本地 ASR：SenseVoice INT8（离线，`sherpa-onnx`）
- 本地 TTS：Kokoro INT8（离线，`sherpa-onnx`）
- VAD：Silero VAD
- 远程 LLM：OpenAI 兼容的 `chat/completions` HTTP 接口
- 传输：WebSocket，协议**严格对齐本地 `xiaozhi-esp32` 固件**（文本消息 + 二进制 Opus 音频帧，支持 v1/v2/v3 二进制协议版本）

设计目标：在 Intel Celeron N5105（x86-64）这类低功耗主机上用 Docker 跑一个 ASR/TTS 离线的小智服务端。

权威协议来源（相对路径，与固件仓库并列时）：

- `../xiaozhi-esp32/docs/websocket_zh.md`
- `../xiaozhi-esp32/main/protocols/websocket_protocol.cc`

## 2. 技术栈与关键依赖

| 依赖 | 版本 | 说明 |
|------|------|------|
| `sherpa-onnx` | **1.13.8** | 本地 ASR/TTS；`optional`，仅 `sherpa` feature 引入 |
| `axum` | 0.8 | 内置 `ws`，**不**用 `tokio-tungstenite` |
| `tokio` | 1.44 | `features = ["full"]` |
| `reqwest` | 0.12 | `default-features = false, features = ["json", "rustls-tls"]` |
| `audiopus` | 0.2 | Opus 编解码（`sherpa` feature 下启用，需系统 `libopus`） |
| `rubato` | 0.15 | 重采样（`sherpa` feature 下启用） |
| `serde` / `serde_json` / `toml` / `anyhow` / `thiserror` / `tracing` / `uuid` | 见 `Cargo.toml` | — |

> `sherpa-onnx` 版本务必锁定 **1.13.8**（最新稳定版，2026-09 发布）。最初 spec 写的 `0.1` 是错的。API 已对照 1.13.8 rustdoc 核对：`OfflineSenseVoiceModelConfig`、`OfflineTtsKokoroModelConfig`、`VoiceActivityDetector` 字段与方法签名与实现一致。

## 3. 构建与测试（⚠️ 环境相关命令）

本机的 `cargo` 不在 PATH，且全局 cargo 配置把 USTC 镜像指向 git 协议（返回 404）。**所有 cargo 命令必须**：

```bash
# 1) 用本地 cargo（PATH 未预置）
export PATH="$HOME/.cargo/bin:$PATH"      # 或直接使用绝对路径 ~/.cargo/bin/cargo

# 2) 每次 cargo 调用追加 USTC sparse 镜像覆盖，否则报 404：
--config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'
```

完整示例：

```bash
# 编译（默认 mock 特性，零依赖即可过）
~/.cargo/bin/cargo build \
  --config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'

# 运行（mock 模式，无需模型与 libopus）
cd server && ~/.cargo/bin/cargo run \
  --config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'

# 单元测试
~/.cargo/bin/cargo test \
  --config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'

# 真实引擎（需联网下载 sherpa 原生库 + 系统 libopus + 模型文件）
cd server && ~/.cargo/bin/cargo run --features sherpa -- --config config.toml
```

## 4. Feature 门控（重要）

`Cargo.toml` 中：

- `default = []`：**纯 mock**。无需模型文件、无需系统 `libopus`，即可编译运行并做协议联调。
- `sherpa = ["dep:sherpa-onnx", "dep:audiopus", "dep:rubato"]`：引入真实引擎。

后端选择（见 `config.rs` / `config.example.toml`）：

| 配置项 | 取值 | 行为 |
|--------|------|------|
| `asr.backend` | `mock` / `sherpa` | mock 回显固定文本；sherpa 用 SenseVoice |
| `tts.backend` | `mock` / `sherpa` | mock 生成 0.5s 静音；sherpa 用 Kokoro |
| `llm.backend` | `mock` / `http` | **默认 `mock`**（本地回显，零配置可跑通）；`http` 调真实 OpenAI 兼容接口 |

> 默认（无配置文件）`asr`/`tts`/`llm` 全是 `mock`，因此**裸 `cargo run` 即可端到端跑通整个协议链路**，无需任何外部服务。

## 5. 🚨 关键约束与陷阱（AI 最容易踩）

### 5.1 协议 `type` 标签必须是**小写**

`xiaozhi-esp32` 固件发送/期望**小写** `type` 标签：`hello` / `listen` / `abort` / `mcp`（设备→服务器）与 `hello` / `stt` / `llm` / `tts` / `system` / `custom` / `mcp`（服务器→设备）。

`serde` 的 `#[serde(tag = "type")]` 默认用 Rust 变体**大写**名（`Hello`/`Stt`/`Llm`…）做匹配，会在运行时直接报：

```
unknown variant `hello`, expected one of `Hello`, `Listen`, `Abort`, `Mcp`
```

**必须**加：

```rust
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClientMessage { Hello(ClientHello), Listen { .. }, Abort { .. }, Mcp { .. } }

#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerMessage { Hello { .. }, Stt { .. }, Llm { .. }, Tts { .. }, System { .. }, Custom { .. }, Mcp { .. } }
```

> 这一 bug **`cargo check` 完全无法发现**，只有真实 WebSocket 客户端（`tests/mock_client.py`）才能暴露。任何修改协议枚举的 PR 都必须重跑 mock 联调。

### 5.2 LLM 默认是 mock，不是真网

需要真实对话时，把 `llm.backend` 改为 `"http"` 并填 `api_base` / `api_key`。若误以为默认会真网调用而联调失败，先确认是 mock。

### 5.3 二进制协议版本 = 设备 hello 的 `version`

- `version` 字段 = **二进制协议版本（1/2/3）**，不是握手协议号。
- 服务器**下行**二进制帧必须使用与设备相同的版本（`wrap_downlink`）。
- 上行按版本剥离头部（`unwrap_uplink`）。v1 裸 Opus；v2 16 字节头；v3 4 字节头。
- 建议先用 v1 真机验证，再切 v2/v3。

### 5.4 服务器 hello 的 `audio_params` = 下行解码参数

设备读取服务器 hello 的 `audio_params.sample_rate` / `frame_duration` 来解码下行 TTS 音频。上行仍按设备自己的 16k。下行采样率由 `audio.downlink_sample_rate`（默认 24000）决定。

### 5.5 构建镜像的 Rust 版本必须与生成 `Cargo.lock` 的 cargo 对齐（当前 1.90）

`Cargo.lock` 由本机 cargo **1.90** 生成，锁定的传递依赖需要较新 Rust：`idna_adapter`（`reqwest → url → idna`）要求 `edition2024`（Cargo ≥ 1.85 才能解析）；`time` / `icu_*` 要求 `rustc 1.88`。`Dockerfile` 构建阶段用 **`rust:1.90-bookworm`**（与本机 cargo 同版本，保证 lock 一定可编译）。

**不要降级镜像 Rust**；也不要在无 lock 限定的情况下 `cargo update` 导致依赖需要更高 Rust。若升级了本机 cargo，记得同步抬高此处的镜像版本。

### 5.6 本地（macOS）编 `--features sherpa` 需要 Homebrew 的 `opus`（绕开 autoreconf）

`audiopus_sys` 优先用 `pkg-config` 找系统的 `libopus`；找不到就回退去**从自带 Opus 源码 autotools 编译**，而本机只装了 `autoconf`/`autoreconf`/`glibtoolize`，**缺 `automake`（无 `aclocal`）**，于是 `autoreconf` 直接失败：

```
Can't exec "aclocal": No such file or directory
autoreconf: error: aclocal failed with exit status: 2
```

**最省事的修法（macOS 本地验证用）**：

```bash
# 1) 把 Homebrew 工具链与 pkg-config 路径加入 PATH（非交互 shell 默认不带 /opt/homebrew/bin）
export PATH="/opt/homebrew/bin:$PATH"
# 2) 装 opus，让 audiopus_sys 走 pkg-config 动态链接，彻底跳过源码编译
brew install opus
export PKG_CONFIG_PATH="/opt/homebrew/lib/pkgconfig:$PKG_CONFIG_PATH"
# 3) 此时再编 sherpa 特性即可（动态链到 /opt/homebrew/lib/libopus.dylib）
~/.cargo/bin/cargo check --features sherpa \
  --config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'
```

- 注意：这样编出的二进制**动态依赖 Homebrew 的 opus**，仅用于本地验证代码能否编译，**与 Docker 部署无关**（Docker 走 Debian `libopus0` + 静态/系统链接，CI 已验证可过 `audopus_sys`）。
- 若想严格复现源码编译路径，则需再 `brew install automake libtool`（并把 `libtoolize` 链到 `glibtoolize`），但 pkg-config 路径更省事，推荐。

### 5.7 容器启动会自动检测并下载缺失模型（`docker-entrypoint.sh`）

报错 `tokens.txt does not exist` / `创建 SenseVoice 识别器失败` 的根因是 `/models` 里没有模型文件。Docker 入口脚本在 `exec` 服务器**之前**会自检关键文件（`silero_vad.onnx`、`SenseVoiceSmall/model.int8.onnx`、`Kokoro/model.int8.onnx` 等，**官方包内文件名是 `model.int8.onnx`，不是 `model.onnx`**）：

- **缺失 → 自动从 k2-fsa/sherpa-onnx 官方 release 下载**到挂载的 `/models`（默认行为），下载后持久化，后续启动检测到即跳过。
- **默认走 GitHub 代理 `https://tvv.tw/`**（`GITHUB_PROXY` 覆盖，`off` 直连）：部署环境直连 `github.com`/`release-assets.githubusercontent.com` 常超时（curl 卡 134s）；代理仅对 github.com 直链套用，内网镜像 URL 不受影响。
- 下载地址可被 `SENSEVOICE_URL` / `KOKORO_URL` / `SILERO_VAD_URL` 覆盖（内网镜像）。
- curl 带 `--connect-timeout 15 --retry 3`（避免连接假死 134s）；下载/解压失败会让入口**中止启动**（未设 `XIAOZHI_ALLOW_MISSING_MODELS` 时），避免带着缺模型崩溃重启循环。
- 行为开关 `XIAOZHI_AUTO_DOWNLOAD_MODELS`：`missing`（默认）/`force`（每次重下）/`off`（不下载）。
- 运行期镜像需装 `curl` + `bzip2`（`tar` 自带）用于下载与解包；`docker-compose.yml` 的 `./models` 挂载**必须可写**（不要 `:ro`）。
- 离线/内网：先 `./server/scripts/download_models.sh /host/models` 预置再挂载，或设 `XIAOZHI_AUTO_DOWNLOAD_MODELS=off`。

默认模型包（已完整下载验证，与 config.example.toml 路径一一对应）：`sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09`（SenseVoice INT8，`model.int8.onnx`）、`kokoro-int8-multi-lang-v1_1`（Kokoro INT8 **中英双语**，含 model.int8.onnx/voices.bin/tokens.txt/espeak-ng-data/lexicon-zh.txt/lexicon-us-en.txt/dict（jieba）/date-zh.fst）、`silero_vad.onnx`（Silero VAD）。⚠️ 此前误判 `kokoro-int8-multi-lang-v1_1` 不是完整包（流式列清单被管道截断所致），实际 144MB/417 文件完整；`kokoro-int8-en-v0_19` 仅英文、无 lexicon，中文场景勿用。

## 6. 项目结构速查（monorepo）

本仓库为 monorepo：`server/` 是 Rust 服务端，`client/` 是 Kuikly 多端工程（管理后台 web）。

```
server/                        # Rust 服务端
  src/
    main.rs       入口：加载配置 → 初始化引擎 → 启动 Axum
    config.rs     TOML 配置 + mock 默认值（含 GET/POST /api/config 读写）
    protocol.rs   消息枚举 + 二进制版本封装（v1/v2/v3）—— 改这里必看 §5.1
    ws.rs         WebSocket 网关：握手 / 协商 / 鉴权 + 静态托管管理页（/）
    session.rs    每连接会话状态机 + 语音流水线（支持 abort）
    asr.rs / vad.rs / tts.rs / llm.rs / engine.rs / audio/
  config.toml / config.example.toml
  scripts/download_models.sh
  tests/mock_client.py         零依赖 WebSocket 联调客户端
client/                        # Kuikly 多端工程（管理后台 web + macOS 测试台）
  settings.gradle.kts         rootProject.name = "client"
  shared/
    commonMain/               公共代码：@Page("config") 管理后台（跨端）+ 配置表单 + NetworkModule
    macosArm64Main/           仅 macOS(arm64) 编译：@Page("test") ASR/TTS 测试台 + XiaoZhiModule
  apps/
    h5App/                    Web(H5) 宿主：Main.kt + index.html（仅管理后台，无测试功能）
    androidApp/ iosApp/ ohosApp/   各原生宿主
    macosApp/                 macOS 宿主（原生，Mac Catalyst）：ASR/TTS 测试台，复用 iOS 渲染器
```

> server 在 `/` 静态托管 `client/apps/h5App` 构建产物（由 `[server].admin_dir` 指定目录），
> 并通过 `GET/POST /api/config` 让管理页读写 `config.toml`。

## 7. 运行与联调

```bash
# 启动 mock 服务（后台）
~/.cargo/bin/cargo run --config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"' &
# 健康检查
curl http://127.0.0.1:8000/api/health   # => xiaozhi-server-rust ok

# 协议联调（零第三方依赖，纯标准库）
python3 server/tests/mock_client.py
# 期望输出含：握手成功 → 服务器 hello（downlink sr=24000）→
# 文本消息序列 ['stt','llm','tts:start','tts:sentence_start','tts:stop']
# → 收到若干下行二进制帧 → PASS
# 可选参数：--host 127.0.0.1 --port 8000 --token <bearer>（expected_token 非空时）
```

联调客户端断言顺序：**hello → listen start → stt → llm → tts:start → tts:sentence_start → 二进制帧 → tts:stop**。任一缺失即 FAIL。

## 8. 配置加载优先级

`XIAOZHI_CONFIG` 环境变量 → `--config <path>` 参数 → 内置默认（全 mock）。

## 9. 已知限制 / 未实现

- **激活流程（OTA/activate）本期未实现**：接受任意设备，`expected_token` 为空则跳过鉴权。
- **VAD 模型加载**：每次会话 `VoiceActivityDetector::create` 会加载模型；连接数大时建议池化（待优化）。
- **ASR 流式**：SenseVoice 为离线逐段识别；如需逐字流式可后续换 Zipformer `OnlineRecognizer`。
- **mock 模式下行音频为空帧**：`encode_opus_frame` 在 mock 下返回空，仅用于验证协议回包。
- 残留编译 warning（预留未用字段/变体），不影响功能。

## 10. 提交规范（重要）

本项目约定：**不要自动提交代码**。完成改动并编译通过后停在工作区，由用户决定何时、以什么范围提交。不要自行 `git add` / `git commit`。

## 11. 更多上下文

- 设计取舍、分阶段实施、协议对齐要点见 [`PLAN.md`](./PLAN.md)。
- 用户向文档、快速开始、Docker、协议要点见 [`README.md`](./README.md)。
