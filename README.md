# xiaozhi-server-rust

基于 [`sherpa-onnx`](https://github.com/k2-fsa/sherpa-onnx) 的 **小智（xiaozhi-esp32）** 语音服务端，纯 Rust 实现：

- **本地 ASR**：SenseVoice INT8（离线，CPU 推理）。
- **本地 TTS**：Kokoro INT8（离线，CPU 推理）。
- **VAD**：Silero VAD，负责上行音频的语音段切分。
- **远程 LLM**：通过 OpenAI 兼容的 Chat Completions HTTP 接口接入。
- **传输**：WebSocket 协议，严格对齐 `xiaozhi-esp32` 固件（文本消息 + 二进制 Opus 音频帧，支持 v1/v2/v3 二进制协议版本）。

> 设计目标：在 Intel Celeron N5105（x86-64，Jasper Lake）这类低功耗主机上，用 Docker 跑起一个完全本地化（ASR/TTS 离线）的小智服务端，LLM 走本地或云端的 HTTP 接口。
>
> 📐 项目架构详见 [docs/architecture.md](./docs/architecture.md)（模块划分、会话流水线、协议设计、部署架构与扩展指引）。

---

## 特性

- 协议层严格对齐 `xiaozhi-esp32` 固件：设备 hello `version` 即二进制协议版本；服务器 hello 的 `audio_params` 作为**下行（TTS）解码参数**。
- 上行采样率跟随设备（默认 16k），下行采样率由服务器决定（默认 24k），二者独立协商。
- **Feature 门控**：`default`（纯 mock，无需模型文件与系统 Opus 库即可编译运行）；`sherpa`（引入 `sherpa-onnx` / `audiopus` / `rubato`，启用真实引擎）。
- 内置 mock 后端（ASR 回显固定文本、TTS 生成静音、LLM 本地回显、VAD 跳过），可在**没有任何模型文件与 libopus** 的环境下完成编译与端到端协议联调。
- 可选 Bearer Token 鉴权（配置 `server.expected_token`）。

---

## 当前依赖版本（已核对官方文档）

- `sherpa-onnx` **1.13.8**（截至 2026-09 的最新稳定版；官方 Rust API 文档：<https://k2-fsa.github.io/sherpa/onnx/rust-api/index.html>）。
- `axum` 0.8（内置 `ws`，无需 `tokio-tungstenite`），`tokio` 1.44，`reqwest` 0.12（rustls）。

API 已对照 1.13.8 rustdoc 校验：`OfflineSenseVoiceModelConfig`、`OfflineTtsKokoroModelConfig`、`VoiceActivityDetector` 的字段与方法签名与实现一致。

---

## 快速开始（mock 模式，零依赖）

无需模型、无需 `libopus`，开箱编译运行：

```bash
# 使用本地 cargo（本仓库在 macOS 上用 ~/.cargo/bin/cargo）
cd server && ~/.cargo/bin/cargo run
```

默认监听 `0.0.0.0:8000`，使用内置 mock 配置（`asr.backend=mock`、`tts.backend=mock`、`expected_token=""`）。

健康检查：

```bash
curl http://127.0.0.1:8000/api/health
# => xiaozhi-server-rust ok
```

随后可运行 Python mock 客户端做协议联调（见下文 [测试](#测试)）。

---

## 配置

配置加载优先级：**`XIAOZHI_CONFIG` 环境变量** → **`--config <path>` 参数** → **内置默认（mock）**。

```bash
# 方式一：环境变量
export XIAOZHI_CONFIG=$PWD/server/config.toml
cd server && ~/.cargo/bin/cargo run

# 方式二：命令行参数
cd server && ~/.cargo/bin/cargo run -- --config config.toml
```

配置字段说明见 [`server/config.example.toml`](./server/config.example.toml)。关键项：

- `server.listen`：监听地址，默认 `0.0.0.0:8000`。
- `server.expected_token`：Bearer token；为空表示不校验 `Authorization` 头。
- `audio.downlink_sample_rate` / `downlink_frame_duration_ms`：下行（TTS）采样率与帧长，写入服务器 hello。
- `audio.binary_protocol_version`：下行二进制协议版本（1/2/3）。**建议先用 1 真机验证，再切 2/3。**
- `asr.backend` / `tts.backend`：`sherpa` 或 `mock`。
- `llm.backend`：`mock`（本地回显，零配置联调，默认）或 `http`（真实 OpenAI 兼容接口）。
- `llm.*`：`http` 模式下的 `api_base` / `api_key` / `model` / `system_prompt`。

---

## 真实引擎（`sherpa` feature）

真实引擎需要：

1. 系统 `libopus`（audiopus 编译/链接依赖，且 `sherpa-onnx` 部分路径需要）。
2. 网络（构建脚本/原生库下载）。
3. 本地模型文件（SenseVoice / Kokoro / Silero VAD）。

### 1. 下载模型

```bash
./server/scripts/download_models.sh /host/models
```

脚本会拉取 Silero VAD、SenseVoice INT8、Kokoro INT8（默认取官方 release 的 `*-int8-*` 量化包），并提示核对文件路径。下载后按 `config.example.toml` 的 `[asr]` / `[vad]` / `[tts]` 路径对齐 `model` / `tokens` / `voices` 等。

> 用 Docker 部署时**无需手动下载**：容器入口会自动检测并下载（见上文 [模型自动下载](#模型自动下载)），除非你处于离线/内网环境。

### 2. 编译运行

```bash
cd server && ~/.cargo/bin/cargo run --features sherpa -- --config config.toml
```

> 构建 `sherpa-onnx` 原生库需要联网；首次 `cargo build --features sherpa` 可能耗时数分钟。

---

## Docker

多阶段构建。`Dockerfile` 默认以 `--features sherpa` 构建（运行期需 `libopus0` 与 `curl`/`bzip2` 供入口脚本按需下载模型）：

```bash
docker build -t xiaozhi-server-rust:latest .
docker run -d --name xiaozhi \
  -p 8000:8000 \
  -v /host/models:/models \
  -v $PWD/config.toml:/app/config.toml \
  -e XIAOZHI_CONFIG=/app/config.toml \
  xiaozhi-server-rust:latest
```

### 模型自动下载

容器入口（`docker-entrypoint.sh`）会在启动时检测关键模型文件（`silero_vad.onnx`、`SenseVoiceSmall/tokens.txt`、`Kokoro/model.onnx`）：

- **缺失则自动下载**到挂载的 `/models`（默认行为，首次启动拉取后持久化，后续跳过）。
- **默认走 GitHub 代理 `https://tvv.tw/`**（`GITHUB_PROXY` 可换其他代理，`off` 直连）——部署环境直连 `github.com` / `release-assets.githubusercontent.com` 常超时不可达；代理仅对 github.com 直链套用，覆盖为内网镜像地址时不受影响。
- 下载地址可用环境变量覆盖：`SENSEVOICE_URL` / `KOKORO_URL` / `SILERO_VAD_URL`（便于内网镜像）。
- 行为开关 `XIAOZHI_AUTO_DOWNLOAD_MODELS`：`missing`（默认，缺失才下）/ `force`（每次重下）/ `off`（不下载，依赖挂载或预置）。

> 因此**不手动预置模型也能直接 `docker compose up` 跑起来**；直连不可达时靠 `GITHUB_PROXY` 代理兜底。
> 离线/内网环境：先 `GITHUB_PROXY=off ./scripts/download_models.sh /host/models` 预置（内网镜像则设对应 URL），再挂载，或设 `XIAOZHI_AUTO_DOWNLOAD_MODELS=off`。

### 部署机指令集

构建期固定 `RUSTFLAGS="-C target-cpu=x86-64-v2"`（见 `Dockerfile`），以兼容部署机 N5105（Tremont，无 AVX）。**不要用 `native`**——CI 构建机 CPU 与部署机不同，`native` 会嵌入部署机不支持的 AVX/AVX2/AVX-512 指令，导致运行时 `SIGILL` 崩溃。

---

## 协议要点

- **设备 hello**：`{"type":"hello","version":1,"audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60},"features":{...}}`。`version` = 二进制协议版本（1/2/3）。
- **服务器 hello**：回 `transport`、`session_id`、`audio_params`（下行 TTS 解码参数）。
- **一次对话**：`listen start` → `stt` → `llm`（首包）→ `tts start` → `tts sentence_start`（带文本）→ 若干**下行二进制 Opus 帧** → `tts stop`。
- **上行音频**：设备以二进制帧（按协商版本封装）发送 Opus；服务器解码 → VAD → ASR。mock 模式忽略上行音频，由 `listen start` 直接触发模拟流水。
- **abort**：任何阶段客户端可发 `{"type":"abort"}` 中断当前 TTS 下行。

二进制帧封装见 `src/protocol.rs`：`wrap_downlink` / `unwrap_uplink`（v1 裸 Opus；v2/v3 带版本头）。

---

## 测试

### 单元自测（Rust）

协议层与音频层含 `#[cfg(test)]` 用例（二进制版本往返、VAD/ASR 桩等）：

```bash
cd server && ~/.cargo/bin/cargo test
```

### 协议联调（Python mock 客户端）

`tests/mock_client.py` 是一个**零第三方依赖**的 WebSocket 客户端（纯标准库实现 RFC 6455 帧），验证握手、协商与一次完整对话回包：

```bash
# 先启动服务（mock 模式）
cd server && ~/.cargo/bin/cargo run &

# 运行联调客户端
python3 server/tests/mock_client.py
# => 依次断言 server hello / stt / llm / tts start / tts sentence_start / 二进制帧 / tts stop
# => 输出 PASS 或 FAIL
```

可用参数：`--host 127.0.0.1 --port 8000 --token <bearer>`（当 `expected_token` 非空时传 `--token`）。

### 管理后台（Web 配置页）

管理后台是一个 **Kuikly 多端工程**（`client/`），配置页（`@Page("config")`）位于 `client/shared` 公共层，跨端复用（web / Android / iOS / OHOS）。
其 web 构建产物由 server 在 `/` 静态托管。它通过 `GET` / `POST`（或 `PUT`）`/api/config` 读写 `config.toml` 的**全部参数**，无需手改文件；auth 沿用 `server.expected_token` 语义，不另造简化鉴权。

- **访问**：启动 server（带 `--config config.toml`）后浏览器打开 `http://<host>/?page_name=config`，
  页面自动拉取当前配置并渲染表单；修改后点「保存配置」写回 `config.toml`（原 TOML 注释会被覆盖）。
- **API**：
  - `GET /api/config` → 返回当前配置 JSON（启动时指定了文件则实时读盘）。
  - `POST /api/config`（亦接受 `PUT`）→ 接收完整配置 JSON 并写回启动加载的配置文件；**引擎相关参数（ASR/TTS/LLM）需重启 server 才生效**，`[server]` 部分下次启动生效。
- **注意**：以内置默认（mock）配置启动（未指定文件）时保存无目标文件，会返回 400；请用 `--config` 指定 `config.toml` 后重启再保存。

```bash
# 构建 web 管理页（产物目录含 index.html 与 nativevue2.js，由 [server].admin_dir 指向）
cd client && ./gradlew :apps:h5App:build

# 启动 server（admin_dir 默认 ../client/apps/h5App/dist，按实际构建输出调整）
cd server && ~/.cargo/bin/cargo run -- --config config.toml
# 浏览器访问 http://127.0.0.1:8000/?page_name=config
```

### ASR/TTS 测试台（macOS App）

`web` 受浏览器麦克风 / WebSocket 二进制等限制，不适合做语音链路测试。因此另起 **macOS App**（`apps/macosApp`，Mac Catalyst，复用 iOS 渲染器 `OpenKuiklyIOSRender`）承载 ASR/TTS 测试，替代原 web 测试台：

- **测试页 `@Page("test")` 仅放在 `client/shared/src/macosArm64Main`**：只编译进 macOS 框架，**web/Android/iOS/OHOS 的包都不含测试代码**，web 自然无测试功能。
- 管理后台 `@Page("config")` 仍在 `commonMain`，**跨端复用**；测试页提供「管理后台」入口跳过去。
- 测试逻辑复用 server 的专用测试协议（见 `src/protocol.rs` / `src/session.rs`）：
  - `asr_test {action: start|stop}`：整段录音缓冲后一次性识别，回 `stt`；
  - `tts_test {text, speaker, lang, speed}`：合成并流式下发 Opus 音频。
- 原生桥接在 `apps/macosApp/XiaoZhiModule.m`（Kuikly 自定义 Module `XiaoZhiModule`）：负责 WS 建连、麦克风采集、音频播放。**Opus 编解码目前为占位 TODO**（需接入 `libopus` / OpusKit），是联调前唯一待补的原生环节。

```bash
# ① 编出 KMP 业务框架 shared.framework（含 config 管理页 + test 测试页），仅 Apple Silicon
cd client && ./gradlew :shared:linkReleaseFrameworkMacosArm64
#    产物在 client/shared/build/bin/macosArm64/releaseShared/shared.framework
#    （注：Kotlin/Native 没有 assembleMacosArm64 任务，正确任务名是 linkReleaseFrameworkMacosArm64）

# ② 生成 Xcode 工程（需 brew install xcodegen）
cd client/apps/macosApp && xcodegen generate        # 生成 macosApp.xcodeproj，并 embed 上面的 shared.framework

# ③ 安装渲染器依赖（需 CocoaPods：sudo gem install cocoapods 或 brew install cocoapods）
pod install                                        # 拉取 OpenKuiklyIOSRender，生成 macosApp.xcworkspace

# ④ 用 Xcode 打开 workspace，选「My Mac (Mac Catalyst)」运行
open macosApp.xcworkspace
```

> 启动顺序不可省：必须先 ① 编出 `shared.framework`，② 的 xcodegen 才能把框架正确 embed 进 App；
> 否则链接阶段报 `shared.framework not found`。Opus 编解码（`XiaoZhiModule.m`）仍为 TODO，联调前需补。

> 协议要点与 `mock_client.py` 共用同一套 `hello / asr_test / tts_test` 语义，便于回归。

---

## 项目结构（monorepo）

本仓库为 monorepo：`server/` 是 Rust 服务端，`client/` 是 Kuikly 多端工程（管理后台 web）。

```
.
├── Dockerfile / docker-compose.yml / docker-entrypoint.sh   # 部署编排（仓库根）
├── server/                      # Rust 服务端
│   ├── Cargo.toml / Cargo.lock
│   ├── src/
│   │   ├── main.rs             入口：加载配置、初始化引擎、启动 Axum
│   │   ├── config.rs           TOML 配置与 mock 默认值（含 GET/POST /api/config 读写）
│   │   ├── protocol.rs         消息枚举 + 二进制版本封装（v1/v2/v3）
│   │   ├── ws.rs               WebSocket 网关 + 静态托管管理页（/）
│   │   ├── session.rs          每连接会话状态机 + 语音流水线
│   │   ├── asr.rs / vad.rs / tts.rs / llm.rs / engine.rs / audio/
│   ├── config.toml / config.example.toml
│   ├── scripts/download_models.sh
│   └── tests/mock_client.py
└── client/                      # Kuikly 多端工程（管理后台）
    ├── settings.gradle.kts      rootProject.name = "client"
    ├── shared/
    │   ├── commonMain/         公共代码：@Page("config") 管理后台（跨端）+ 配置表单 + NetworkModule
    │   └── macosArm64Main/     仅 macOS(arm64) 编译：@Page("test") ASR/TTS 测试台 + XiaoZhiModule
    └── apps/
        ├── h5App/               Web(H5) 宿主：Main.kt + index.html（仅管理后台，无测试功能）
        ├── androidApp/          Android 宿主（仅 arm64-v8a，ndk.abiFilters 限定）
        ├── iosApp/              iOS 宿主（原生；仅 arm64：iosArm64 + Apple Silicon 模拟器，Podfile 排除 x86_64）
        ├── macosApp/            macOS 宿主（原生，Mac Catalyst）：ASR/TTS 测试台（仅 arm64）
        └── ohosApp/             HarmonyOS 宿主（原生）
```

> server 在 `/` 静态托管 `client/apps/h5App` 的构建产物（由 `[server].admin_dir` 指定目录），
> 并通过 `GET/POST /api/config` 让管理页读写 `config.toml`。

---

## 状态与已知限制

- **本期未实现激活流程（OTA/activate）**：标准 xiaozhi-server 含激活/OTP，本期接受任意设备（可选 token 校验）。如需对接官方激活另开任务。
- **VAD 模型加载**：每次会话 `VoiceActivityDetector::create` 会加载模型；连接数大时建议池化（待优化）。
- **ASR 流式**：本期 SenseVoice 为离线逐段识别；如需逐字流式可后续换 Zipformer `OnlineRecognizer`。
- **mock 模式下行音频为空帧**：`encode_opus_frame` 在 mock 下返回空，仅用于验证协议回包；真实音频需 `sherpa` feature + libopus。

---

## 许可证

MIT（见 [`LICENSE`](./LICENSE)）。
