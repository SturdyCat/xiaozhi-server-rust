# 设备协议与语音链路

> 本文为当前功能使用说明；实现细节以邻近源码、配置样例与测试为准。

# 功能与配置详解

> 本文是 [README](<../../../../../README.md>) 的展开：功能细节、配置字段、HTTP API、OTA 托管、测试台构建与测试回归。
> 权威配置字段说明以 [`server/config.example.toml`](<../../../../config.example.toml>) 的行内注释为准；整体架构见 [architecture.md](<../../../../../docs/architecture.md>)。

---

## 1. 语音链路与协议

- 协议层严格对齐 `xiaozhi-esp32` 固件：设备 hello `version` 即二进制协议版本（1/2/3）；服务器 hello 的 `audio_params` 作为**下行（TTS）解码参数**。
- 上行采样率跟随设备（默认 16k），下行采样率由服务器决定（默认 24k），二者独立协商。
- **下行缓冲三段式**（提前量 + 服务器水库 + 起跑闸）：缓冲吸收上游间歇停顿；若平均生产速率持续慢于实时，仍会断音，必须提高上游吞吐。
- **Feature 门控**：`default`（无真实引擎，仅供 `cargo check/test` 编译；运行必须 `--features sherpa`）；`sherpa`（引入 `sherpa-onnx` / `audiopus` / `rubato`，真实引擎）。
- **无 mock**：ASR 恒为 SenseVoice、TTS 可选 Kokoro / 讯飞在线、LLM 恒为 OpenAI 兼容 HTTP、VAD 恒为 Silero——ESP 接入走完整正式流水线；macApp 测试台连接带 `hello.test=true`，走 `asr_test`/`tts_test`/`llm_test`/`nlu_test` 四个独立服务请求-响应端点（`nlu_test` 按磁盘最新 `[nlu]` 配置构建 TinyBERT 直推，需 `--features nlu` 且已配置 bert 模型，未配置回错误原因帧）。
- 可选 Bearer Token 鉴权（配置 `server.expected_token`）。

### 协议要点

- **设备 hello**：`{"type":"hello","version":1,"audio_params":{...},"features":{...}}`。`version` = 二进制协议版本（1/2/3）。
- **服务器 hello**：回 `transport`、`session_id`、`audio_params`（下行 TTS 解码参数）。
- **一次对话**：`listen start` → `stt` → `llm`（首包）→ `tts start` → `tts sentence_start`（带文本）→ 若干**下行二进制 Opus 帧** → `tts stop`。
- **上行音频**：设备以二进制帧（按协商版本封装）发送 Opus；服务器解码 → VAD → ASR → LLM → TTS（正式流水线，无 mock 捷径）。`listen start` 仅状态同步，不触发对话。
- **abort**：任何阶段客户端可发 `{"type":"abort"}` 中断当前 TTS 下行。

二进制帧封装（v1/v2/v3 字节布局）见 `server/src/workflow/protocol/protocol.rs` 的模块注释（权威说明）；`wrap_downlink` / `unwrap_uplink` 是唯一的封装/解封装出口。

---

## 上级入口

[上级 AGENTS](<../../AGENTS.md>)
