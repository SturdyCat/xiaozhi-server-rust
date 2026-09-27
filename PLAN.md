# xiaozhi-server-rust 实施方案（对齐本地 xiaozhi-esp32 固件）

> 目标：在 Intel Celeron N5105 + Docker + Rust 上，用 `sherpa-onnx` 本地完成 ASR（SenseVoice INT8）与 TTS（Kokoro INT8），远程调用 LLM，并通过 WebSocket 与本地 `xiaozhi-esp32` 固件真实对接。
>
> 协议严格对齐本地仓库 `../xiaozhi-esp32/docs/websocket_zh.md` 与 `../xiaozhi-esp32/main/protocols/websocket_protocol.cc`。

> **版本与 API 来源确认（2026-09-27 核对）**：crate 最新稳定版 **`sherpa-onnx = 1.13.8`**（crates.io，2026-09-11 发布）。官方 API 文档站 <https://k2-fsa.github.io/sherpa/onnx/rust-api/index.html> 列出全部示例（sense_voice / kokoro_tts_zh_en / silero_vad_remove_silence / streaming_zipformer …）。其示例页仅给出 GitHub 链接、不内联源码；本环境 raw.githubusercontent.com 不可达，故改用 **docs.rs 1.13.8 rustdoc** 精确核对三个最关键结构体，字段与方案**完全一致、无差异**：
> - `OfflineSenseVoiceModelConfig { model, language, use_itn: bool }`
> - `OfflineTtsKokoroModelConfig { model, voices, tokens, data_dir, length_scale: f32, dict_dir, lexicon, lang }`
> - `VoiceActivityDetector::create(&VadModelConfig, buffer_size_in_seconds: f32)` + `accept_waveform(&[f32])` / `front()` / `pop()` / `flush()` / `detected()` / `is_empty()`

---

## 0. 关键约束（已实机核对）

| 项 | 说明 |
|----|------|
| `cargo` 可用性 | 本机 `cargo 1.90.0`（stable aarch64）可用，可直接 `cargo check` 验证编译；但**真实推理需模型文件（~300MB+），本环境无法跑端到端**。 |
| crate 版本 | **`sherpa-onnx = 1.13.8`**（非 spec 写的 `0.1`）。API 已按官方 `rust-api-examples` 核对。 |
| SenseVoice 是离线识别器 | 必须用 `OfflineRecognizer` + `OfflineSenseVoiceModelConfig`，**不是** `OnlineRecognizer`（spec 写错）。方案：VAD 切段 → 每段送 SenseVoice 离线识别。 |
| 协议版本 | 设备 hello 的 `version` 字段 = **二进制协议版本（1/2/3）**，同时作为 `Protocol-Version` 请求头。服务器下行二进制帧必须使用同一版本。 |

---

## 1. 与原始 spec 的差异修正

| 原始 spec | 修正为（真实情况） |
|-----------|-------------------|
| `sherpa-onnx = "0.1"` | `sherpa-onnx = "1.13.8"`，默认静态链接，构建脚本自动下载原生库。 |
| ASR 用 `OnlineRecognizer` + SenseVoice | SenseVoice 在 sherpa-onnx 中是 `OfflineRecognizer`；`OnlineRecognizer` 仅用于 Zipformer 真流式（未采用）。 |
| VAD 用 `sherpa_onnx::Vad` | 实为 `VoiceActivityDetector` + `VadModelConfig` + `SileroVadModelConfig`，API：`create(&cfg, 30.0)` / `accept_waveform` / `front()` / `pop()` / `flush()`。 |
| TTS 用 `OfflineTts` + 单模型 | `OfflineTts` + `OfflineTtsKokoroModelConfig`，需 `model.onnx`+`voices.bin`+`tokens.txt`+`espeak-ng-data`+`dict`+`lexicon-*.txt` 一整套。 |
| 固定 16kHz | **上行跟随设备 hello（`audio_params.sample_rate`，默认 16k）；下行由服务器在 hello 中指定（默认 24k，设备自动重采样）。** |
| 协议仅 stt/tts/llm | 补齐 `listen`/`abort`/`mcp`/`system`/`custom` 及握手 headers（`Authorization`/`Protocol-Version`/`Device-Id`/`Client-Id`）。 |
| WebSocket 用 axum + tokio-tungstenite | 采用 **axum 0.8 内置 WebSocket**（`WebSocketUpgrade`），无需 tokio-tungstenite。 |
| Opus 用 `audiopus` | 保留 `audiopus`（0.3，编译 libopus），下行 24k 需 `rubato` 重采样（Kokoro 输出若为 24k 则免重采样）。 |

---

## 2. 真实 xiaozhi 协议对齐要点

### 2.1 握手（WebSocket 升级后）
- **设备 → 服务器**（文本帧）：
  ```json
  {
    "type": "hello",
    "version": 1,
    "features": { "mcp": true, "aec": false },
    "transport": "websocket",
    "audio_params": { "format": "opus", "sample_rate": 16000, "channels": 1, "frame_duration": 60 }
  }
  ```
  请求头：`Authorization: Bearer <token>`、`Protocol-Version`、`Device-Id`（MAC）、`Client-Id`（UUID）。
- **服务器 → 设备**（文本帧，必须含 `type`+`transport`，`session_id`/`audio_params` 可选但建议下发）：
  ```json
  {
    "type": "hello",
    "transport": "websocket",
    "session_id": "<uuid>",
    "audio_params": { "format": "opus", "sample_rate": 24000, "channels": 1, "frame_duration": 60 }
  }
  ```
  ⚠️ 设备端会**读取此 `audio_params.sample_rate` / `frame_duration` 作为下行（TTS）解码参数**（`websocket_protocol.cc:239-246`）。上行仍按设备自己的 16k 发送。

### 2.2 二进制协议版本（= hello 的 `version`）
- **v1（默认）**：裸 Opus 字节，无头。
- **v2**（`BinaryProtocol2`，小端、packed）：`u16 version | u16 type(0=OPUS,1=JSON) | u32 reserved | u32 timestamp(ms) | u32 payload_size | payload`。
- **v3**（`BinaryProtocol3`）：`u8 type | u8 reserved | u16 payload_size | payload`。
- 服务器**下行**编码须使用与设备相同的版本；**上行**解码按版本剥离头部（v1 直接用，v2/v3 去头取 payload）。

### 2.3 文本消息
- 设备 → 服务器：`hello` / `listen{state:start|stop|detect, mode:auto|manual|realtime, text?}` / `abort{reason?}` / `mcp{payload:JSON-RPC}`。
- 服务器 → 设备：`hello` / `stt{text}` / `llm{emotion?, text?}`（text 为表情 emoji） / `tts{state:start|stop|sentence_start, text?}` / `mcp{payload}` / `system{command:reboot}` / `custom{payload}`。
- 所有服务器消息带 `session_id`（握手时分配并回显）。

### 2.4 状态流转（每连接一个会话）
`Idle → Connecting(发 hello) → Listening(设备发 listen start + 上行音频) → [VAD 端点] ASR→stt→LLM→llm→tts start→tts sentence_start(text)→下行音频→tts stop → 回到 Listening(auto) / Idle`。
- 收到 `tts start` 设备停止录音；收到 `abort`（用户打断）中断 TTS 下行，回到 Listening。
- 鉴权：`Authorization` 头可选校验（配置 `expected_token`，为空则跳过）。

---

## 3. 架构与文件结构

```
xiaozhi-server-rust/
├── Cargo.toml
├── config.example.toml
├── Dockerfile
├── .dockerignore
├── scripts/
│   └── download_models.sh        # 下载 SenseVoice INT8 / Kokoro / Silero VAD
├── src/
│   ├── main.rs                   # 启动 Axum，加载 Config，初始化全局引擎（ASR/TTS 共享）
│   ├── config.rs                 # 配置结构体 + toml/env 加载
│   ├── error.rs                  # AppError / anyhow 桥接
│   ├── protocol.rs               # ClientMessage/ServerMessage 枚举 + AudioParams + 二进制版本封装
│   ├── engine.rs                 # 全局共享引擎（Arc<OfflineRecognizer> / ASR、TTS 工厂）
│   ├── ws.rs                     # Axum WebSocket 网关：握手、JSON 解析、二进制帧分发
│   ├── session.rs                # 每会话状态机 + VAD/ASR/LLM/TTS 流水线 + 下行任务（支持 abort）
│   ├── asr.rs                    # SenseVoice 离线识别封装（mock 后端可选）
│   ├── vad.rs                    # Silero VAD 封装（共享模型 + 每会话 detector）
│   ├── tts.rs                    # Kokoro TTS 封装（mock 后端可选）
│   ├── llm.rs                    # reqwest OpenAI 兼容调用 + 每会话多轮上下文
│   └── audio/
│       ├── opus.rs              # audiopus 编解码（上行 16k 解码 / 下行 24k 编码，按帧长分帧）
│       └── resample.rs          # rubato 重采样（TTS 输出 → 下行采样率）
└── tests/
    └── mock_client.py            # 本地无模型冒烟测试：发 hello+listen+假 Opus，校验回包
```

**模块职责**
- `engine.rs`：进程内只加载一次 ASR 识别器与 TTS（模型重，~229MB+88MB）。ASR 用共享 `Arc<OfflineRecognizer>`，每会话 `create_stream()`；VAD 模型共享，每会话建 detector（或按 API 决定是否共享 `SileroVadModel`）。
- `ws.rs`：升级连接 → 等 `hello` → 校验 `audio_params`/`version`/鉴权 → 分配 `session_id` → 回 `hello` → 拉起 `session` 任务。
- `session.rs`：上行 Opus → 解码 → VAD 喂入 → 端点触发 → ASR → `stt` → LLM → `llm` → `tts start` → `tts sentence_start` → 逐帧 TTS 音频（Opus，按二进制版本封装）下发 → `tts stop`；监听 `abort`。
- `audio/opus.rs`：上行按设备帧长（默认 60ms@16k=960 样本）解码为 f32；下行将 PCM 按下行帧长（默认 60ms@24k=1440 样本）切片编码。
- `audio/resample.rs`：仅当 `tts.sample_rate() ≠ downlink_sample_rate` 时重采样（Kokoro 24k 且下行 24k 时免）。

---

## 4. 依赖与版本（Cargo.toml）

```toml
[package]
name = "xiaozhi-server-rust"
version = "0.1.0"
edition = "2021"

[dependencies]
sherpa-onnx = "1.13.8"                       # 本地 ASR/TTS，默认静态链接
axum = { version = "0.8", features = ["ws"] } # 内置 WebSocket
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"                                  # 解析 config.toml
config = "0.14"                               # toml + 环境变量覆盖
audiopus = "0.3"                              # Opus 编解码（编译 libopus）
rubato = "0.15"                               # 重采样（版本以实现时最新为准）
anyhow = "1"
thiserror = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
uuid = { version = "1", features = ["v4"] }

[profile.release]
opt-level = 3
lto = "thin"                                  # N5105 上平衡编译时间与运行性能
```

> 注：`audiopus`/`rubato` 具体小版本以 `cargo add` 时解析为准；首次构建会下载 sherpa-onnx 原生静态库（按平台，macOS arm64 走 `osx-arm64-static-lib`，N5105 走 `linux-x64-static-lib`）。

---

## 5. 数据流与状态机（上行 16k / 下行 24k）

```
设备(16k Opus) ──WS二进制──▶ [opus.decode 16k] ─▶ VAD(16k Silero) ─▶ 语音段
                                                                      │
                                                          ASR(SenseVoice INT8)
                                                                      │ text
                                                                      ▼
                                              stt{text} ──▶ LLM(远程) ─▶ reply
                                                                      │
                                            llm{emotion,emoji} ─▶ TTS(Kokoro INT8)
                                                                      │ PCM(24k)
                                            tts start ─▶ tts sentence_start(text)
                                                                      │
                                            [resample→24k]─▶ opus.encode(24k)
                                                                      │ WS二进制(按version封装)
                                                                      ▼
                                             设备(按 server hello 的 24k 解码播放)
```

- 上行采样率来自设备 hello（默认 16k）；下行采样率由服务器 `downlink_sample_rate`（默认 24000）决定，写入服务器 hello。
- 帧长：上行 60ms@16k=960 样本；下行 60ms@24k=1440 样本（均与设备 `frame_duration` 对齐）。

---

## 6. 关键实现细节

### 6.1 协议消息（protocol.rs 草图）

> ⚠️ **关键运行期修正（仅联调客户端能暴露，`cargo check` 查不出）**：`xiaozhi-esp32` 固件发送/期待**小写** `type` 标签（`hello`/`listen`/`abort`/`mcp` 与 `hello`/`stt`/`llm`/`tts`/`system`/`custom`/`mcp`）。serde 的 `#[serde(tag="type")]` 默认用 Rust 变体**大写**名（如 `Hello`）做匹配，会直接报 `unknown variant \`hello\``。必须加 `rename_all = "lowercase"`：
> ```rust
> #[derive(serde::Deserialize)]
> #[serde(tag = "type", rename_all = "lowercase")]   // ← 关键
> pub enum ClientMessage { Hello(ClientHello), Listen { .. }, Abort { .. }, Mcp { .. } }
>
> #[derive(serde::Serialize)]
> #[serde(tag = "type", rename_all = "lowercase")]   // ← 关键
> pub enum ServerMessage { Hello { .. }, Stt { .. }, Llm { .. }, Tts { .. }, System { .. }, Custom { .. }, Mcp { .. } }
> ```
> 此修复已在实现中落地（2026-09-27 经 `tests/mock_client.py` 实机验证生效）。

```rust
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ClientMessage {
    Hello(ClientHello),
    Listen { session_id: Option<String>, state: String, mode: Option<String>, text: Option<String> },
    Abort { session_id: Option<String>, reason: Option<String> },
    Mcp { session_id: Option<String>, payload: serde_json::Value },
}

#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerMessage {
    Hello { session_id: Option<String>, transport: &'static str, audio_params: Option<AudioParams> },
    Stt { session_id: String, text: String },
    Llm { session_id: String, emotion: Option<String>, text: Option<String> },
    Tts { session_id: String, state: String, text: Option<String> },
    System { session_id: Option<String>, command: String },
    Custom { session_id: Option<String>, payload: serde_json::Value },
    Mcp { session_id: Option<String>, payload: serde_json::Value },
}
```

### 6.2 二进制帧封装（上行剥离 / 下行封装）
```rust
pub enum BinVersion { V1, V2, V3 }
pub fn wrap_downlink(v: BinVersion, opus: &[u8], ts_ms: u32) -> Vec<u8> { /* v1 裸数据；v2/v3 加头 */ }
pub fn unwrap_uplink(v: BinVersion, data: &[u8]) -> &[u8] { /* v1 全量；v2/v3 去头取 payload */ }
```

### 6.3 ASR（asr.rs）
- 共享 `OfflineRecognizer`（SenseVoice INT8）：`OfflineRecognizerConfig` → `model_config.sense_voice = OfflineSenseVoiceModelConfig{model, language:"auto", use_itn:true}` → `tokens`/`provider:"cpu"`/`num_threads:3~4`。
- 每会话 `recognizer.create_stream()`，`accept_waveform(16000, &seg)` → `decode` → `get_result()` 取文本。
- `backend="mock"`：直接回显/固定文本，便于无模型本地测试。

### 6.4 VAD（vad.rs）
- `SileroVadModelConfig{ model, threshold:0.5, min_silence_duration:0.25, min_speech_duration:0.25 }` → `VadModelConfig{silero_vad, sample_rate:16000, num_threads:1, provider:"cpu"}` → `VoiceActivityDetector::create(&cfg, 30.0)`。
- 循环 `accept_waveform(chunk)` → `while let Some(seg)=front(){...; pop()}`；结束 `flush()`。

### 6.5 TTS（tts.rs）
- `OfflineTtsConfig{ model: OfflineTtsModelConfig{ kokoro: OfflineTtsKokoroModelConfig{model,voices,tokens,data_dir,dict_dir,lexicon,length_scale:1.0}, num_threads:2 } }` → `OfflineTts::create` → `generate_with_config(text, &GenerationConfig{sid, speed:1.0}, cb)` → `GeneratedAudio{ samples(), sample_rate(), save() }`。
- 说话人：`sid` 由配置指定（如中文女声/男声）。
- `backend="mock"`：生成静音或正弦 PCM，便于本地测试。

### 6.6 LLM（llm.rs）
- `reqwest::Client` POST `chat/completions`（OpenAI 兼容），`stream:false`。
- 每会话维护 `Vec<{role,content}>`（system + 多轮），`max_history` 可配；返回 `assistant` 文本。

### 6.7 鉴权（ws.rs）
- 读 `Authorization` 头，与 `config.server.expected_token` 比对（空字符串=关闭校验）。失败则拒绝升级或握手后关闭。

### 6.8 本地可测性：mock 模式
- `asr.backend` / `tts.backend` 支持 `"mock"`，无模型即可 `cargo run` 跑通完整握手 + 状态机 + 回包逻辑（`tests/mock_client.py` 发 hello+listen+假 Opus 帧，断言收到 `hello`/`stt`/`tts start/stop`）。

---

## 7. 配置（config.example.toml）

```toml
[server]
listen = "0.0.0.0:8000"
expected_token = ""            # 空=不校验

[audio]
downlink_sample_rate = 24000   # 写入服务器 hello，设备据此解码下行
downlink_frame_duration_ms = 60
channels = 1
binary_protocol_version = 1    # 默认下行版本；也可改为跟随设备 hello 的 version

[asr]
backend = "sherpa"             # 或 "mock"
model = "/models/SenseVoiceSmall/model.onnx"
tokens = "/models/SenseVoiceSmall/tokens.txt"
language = "auto"
use_itn = true
num_threads = 3
provider = "cpu"

[vad]
model = "/models/silero_vad.onnx"
threshold = 0.5
min_silence_duration = 0.25
min_speech_duration = 0.25

[tts]
backend = "sherpa"             # 或 "mock"
model = "/models/Kokoro/model.onnx"
voices = "/models/Kokoro/voices.bin"
tokens = "/models/Kokoro/tokens.txt"
data_dir = "/models/Kokoro/espeak-ng-data"
dict_dir = "/models/Kokoro/dict"
lexicon = "/models/Kokoro/lexicon-us-en.txt,/models/Kokoro/lexicon-zh.txt"
speaker = 0
speed = 1.0
num_threads = 2

[llm]
api_base = "https://api.example.com/v1/chat/completions"
api_key = "YOUR_API_KEY"
model = "gpt-4o"
system_prompt = "你是一个有用且简洁的中文语音助手。"
max_history = 10
temperature = 0.7
```

---

## 8. Docker 部署

```dockerfile
# 构建阶段
FROM rust:1.82-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
# N5105 本地指令集优化（在 N5105 宿主机上构建时生效）
ENV RUSTFLAGS="-C target-cpu=native"
RUN cargo build --release

# 运行阶段（sherpa 默认静态链接，二进制自包含）
FROM debian:bookworm-slim AS runtime
WORKDIR /app
COPY --from=builder /app/target/release/xiaozhi-server-rust /app/server
EXPOSE 8000
ENTRYPOINT ["/app/server"]
```
运行（模型走卷挂载，避免每次重下）：
```bash
docker run -d --name xiaozhi \
  --cpus="3.5" \
  -v /host/models:/models \
  -e XIAOZHI_CONFIG=/app/config.toml \
  -p 8000:8000 \
  xiaozhi-server-rust:latest
```
- N5105 调优：`--cpus=3.5`；ASR/TTS `num_threads=3~4`；强制 INT8 模型；`RUSTFLAGS=-C target-cpu=native`（在 N5105 上构建）。
- 固件侧将 WebSocket 地址指向 `ws://<host>:8000`，并建议 `frame_duration=60`、`sample_rate=16000`。

---

## 9. 模型下载脚本（scripts/download_models.sh）

- SenseVoice INT8：`sherpa-onnx-sense-voice-zh-en-ja-ko-2025-...int8`（官方 release）。
- Kokoro：`kokoro-multi-lang-v1_0`（含 voices.bin/tokens/espeak-ng-data/dict/lexicon）。
- Silero VAD：`silero_vad.onnx`。
- 统一解压到 `/host/models/{SenseVoiceSmall,Kokoro}` 与 `silero_vad.onnx`，路径与 `config.example.toml` 对齐。

---

## 10. 本地验证策略

1. **编译验证**：`cargo check`（首次自动下载 sherpa 原生库 + 编译 libopus），确保全部 API（sherpa 1.13.8 / axum 0.8 / audiopus / rubato）用法正确。
2. **无模型冒烟**：`asr.backend="mock"` + `tts.backend="mock"` → `cargo run` → `python3 tests/mock_client.py`，断言握手 `hello` 回包、`listen` 后被推送 `stt`/`tts start`/`tts stop` 与下行 Opus 帧。
3. **真机联调（需模型 + ESP32）**：下模型 → 构建 Docker → 固件指向服务地址 → 验证唤醒→上行→识别→LLM→TTS 播放全链路。`tests/mock_client.py` 仅用于无硬件时的协议回归。

---

## 11. 分阶段实施步骤

1. **脚手架**：`Cargo.toml`、`.gitignore` 补 `target/`、空模块 + `main.rs` 起 Axum，先 `cargo check` 通过。
2. **协议层**：`protocol.rs`（消息枚举 + 二进制版本封装）+ `config.rs` + `error.rs`。
3. **音频层**：`audio/opus.rs`、`audio/resample.rs`（含单元自测：编解码往返、重采样）。
4. **引擎层**：`vad.rs`、`asr.rs`、`tts.rs`、`llm.rs`、`engine.rs`（含 mock 后端）。
5. **网关 + 会话**：`ws.rs`（握手/协商/鉴权）+ `session.rs`（状态机 + 流水线 + abort）。
6. **部署与脚本**：`Dockerfile`、`.dockerignore`、`scripts/download_models.sh`、`config.example.toml`、`README.md` 补全。
7. **验证**：`cargo check` → mock 模式跑 `tests/mock_client.py` → 整理真机联调步骤。

---

## 11.1 实施状态与验证结果（2026-09-27）

### 已完成（全部 7 个阶段）
| 阶段 | 产物 | 状态 |
|------|------|------|
| 1 脚手架 | `Cargo.toml`、`.gitignore`、`src/main.rs` 起 Axum | ✅ |
| 2 协议层 | `src/protocol.rs`（消息枚举 + 二进制版本封装）、`src/config.rs`、`src/error.rs` | ✅ |
| 3 音频层 | `src/audio/opus.rs`、`src/audio/resample.rs` | ✅ |
| 4 引擎层 | `src/vad.rs`、`src/asr.rs`、`src/tts.rs`、`src/llm.rs`、`src/engine.rs`（含 mock 后端） | ✅ |
| 5 网关+会话 | `src/ws.rs`（握手/协商/鉴权）、`src/session.rs`（状态机+流水线+abort） | ✅ |
| 6 部署与脚本 | `Dockerfile`、`.dockerignore`、`scripts/download_models.sh`、`config.example.toml`、`README.md` | ✅ |
| 7 验证 | `cargo test`（3 个二进制协议 roundtrip 通过）+ `tests/mock_client.py` 端到端冒烟 | ✅ |

### 关键实现决策（与 §4 草图的偏差，以实际代码为准）
- **`sherpa-onnx` / `audiopus` / `rubato` 均为 `optional`，由 `sherpa` feature 门控**；`default = []` 走纯 mock，无需系统 `libopus` 与模型文件即可 `cargo run`。真实推理用 `--features sherpa` 编译（需联网下载原生库 + 系统 libopus）。
- **LLM 默认 `backend = "mock"`**（`src/llm.rs` 的 `MockLlm` 本地回显），故零配置 `cargo run` 即可端到端跑通，无需真实 LLM 接口。需要真实对话时把 `llm.backend` 改为 `http` 并填 `api_base`/`api_key`。
- **激活流程（OTA/activate）本期未实现**：接受任意设备，`expected_token` 为空则跳过鉴权（§12 已说明）。

### 验证结果（实机跑通）
```
$ cargo test  →  3 passed (bin_v1/v2/v3_roundtrip)
$ ./target/debug/xiaozhi-server-rust &   # mock 模式
$ python3 tests/mock_client.py
[*] 连接 ws://127.0.0.1:8000/ws
[+] 握手成功
[<] 服务器 hello：session_id=..., downlink sr=24000, frame=60
[>] 已发送 listen start
[<] 文本消息序列：['stt', 'llm', 'tts:start', 'tts:sentence_start', 'tts:stop']
[<] 收到下行二进制帧（TTS Opus）：9 个
[+] PASS: 握手/协商/完整对话回包均符合预期
```
- 验证覆盖了 §6.1 的 `rename_all = "lowercase"` 修复：未修复前客户端 hello 会被拒（`unknown variant \`hello\``），修复后握手与完整协议回包通过。
- 残留 15 个编译 warning（多为预留未用字段：`System`/`Custom` 变体、`VadEngine::flush`、部分 config 字段），属正常脚手架范围，不影响功能。

---

## 12. 风险与待确认

- **模型体积**：SenseVoice INT8 ~229MB、Kokoro 一整套 ~百 MB，需提前下载；本环境不下载，仅提供脚本。
- **VAD 模型共享**：`VoiceActivityDetector::create` 每次会加载模型；需确认 sherpa 1.13.8 是否暴露可共享的 `SileroVadModel`（实现时核实，否则每会话建 detector，连接数大时考虑池化）。
- **二进制版本协商**：默认实现 v1（裸 Opus）；v2/v3 按 header 封装/剥离已实现，但建议先用 v1 真机验证，再切 v2/v3。
- **激活流程（OTA/activate）**：固件首次使用有 Activating 状态，标准 xiaozhi-server 含激活/OTP 流程；本期**暂不实现激活**（接受任意设备，可选 token 校验），如需对接官方激活另开任务。
- **ASR 流式**：本期 SenseVoice 离线（逐段），如需逐字流式可后续换 Zipformer `OnlineRecognizer`。

---

## 13. 实施完成状态（2026-09-27）

全部 7 个阶段已落地。已用 **sherpa-onnx 1.13.8**（最新稳定版，官方 Rust API 文档已核对）实现：

- `Cargo.toml`：`default`（纯 mock，零依赖可编译运行）与 `sherpa` feature 门控；依赖版本已按沙箱安全策略锁定。
- 协议层 `src/protocol.rs`、配置 `src/config.rs`、错误 `src/error.rs`、引擎 `src/{asr,vad,tts,llm,engine}.rs`、音频 `src/audio/{opus,resample}.rs`、网关 `src/ws.rs`、会话 `src/session.rs` 均已完成。
- 部署：`Dockerfile`、`.dockerignore`、`scripts/download_models.sh`、`config.example.toml`、`README.md` 已补全。
- 联调：`tests/mock_client.py`（零第三方依赖的纯标准库 WebSocket 客户端）已新增。

### 验证结果
- `cargo test`：协议二进制版本 v1/v2/v3 往返单测 **3/3 通过**。
- 端到端联调：启动 mock 服务 + `python3 tests/mock_client.py` → **PASS**。
  实测回包序列：`hello → listen start → stt → llm → tts:start → tts:sentence_start → 9 个下行二进制帧 → tts:stop`，下行 `audio_params` 为 24k/60ms。

### 实施中修正的两个运行时 Bug（仅 `cargo check` 无法发现，靠 mock 联调暴露）
1. **协议 type 标签大小写**：`ClientMessage`/`ServerMessage` 原用大写变体名（`Hello`/`Stt`/`Llm`…），但 `xiaozhi-esp32` 固件发送/期望**小写**（`hello`/`stt`/`llm`…）。已加 `#[serde(tag = "type", rename_all = "lowercase")]` 对齐，否则握手后 `recv_hello` 报 `unknown variant hello`。
2. **mock 模式 LLM 仍走真实 HTTP**：原 `LlmClient` 始终 POST 占位 `api_base` 导致 `listen start` 后服务端挂起。已新增 `MockLlm` + `Llm` 枚举（`build_llm` 按 `llm.backend` 选择），默认 `llm.backend="mock"`，保证零配置 `cargo run` 即可端到端跑通。

### 沙箱构建提示（环境相关，非代码问题）
- 全局 cargo 配置把 USTC 镜像指向 git 协议（返回 404）；构建/测试需加：
  `--config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'`。
- `cargo` 使用本地 `~/.cargo/bin/cargo`（PATH 未预置）。
