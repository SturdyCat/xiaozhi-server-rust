# xiaozhi-server-rust

**小智（xiaozhi-esp32）语音服务端，纯 Rust 单二进制**：本地 ASR/TTS 离线推理（SenseVoice / Kokoro INT8）、Silero VAD 池化、OpenAI 兼容 LLM，WebSocket 协议严格对齐固件（v1/v2/v3 二进制 Opus 帧）。内置 Kuikly 多端管理台（Web 直开 + macOS 原生测试台）。为 N5105 级低功耗主机的 Docker 部署而生。

> 架构与设计详见 [docs/architecture.md](<docs/architecture.md>)；功能、配置与维护入口见 [docs/README.md](<docs/README.md>)。

---

## 项目特色

- 🎙️ **完全本地语音链路** —— SenseVoice 识别 + Kokoro 合成 + Silero 切段全部离线（CPU 推理），仅 LLM 走 HTTP；下行缓冲三段式（水库 + 起跑闸）保证长回复不断音。
- 🧠 **智能上下文，可插拔、默认零成本** —— 记忆三后端（图记忆 / 纯存储 / **本地文档 RAG**，词法检索零外网零向量库）、灵魂人格预设、唤醒问候（稳态零 LLM 零 TTS）、指令闸门（**MCP 工具清单驱动**，老固件自动降级）——不开启时行为与裸服务端完全一致。
- 🎛️ **配置即产品** —— schema 驱动 + 热生效三粒度 + 密钥打码护栏 + 部分更新/乐观并发；**配置预设**一键快照回滚；**每设备 overlay** 白名单个性化（这台设备换音色/语速/人格，其他设备不受影响）。
- 🩺 **状态可自诊** —— 11 个能力健康自检（坏了显示原因与怎么修）、延迟大盘 p50/p95、设备/会话/对话/用量面板、设备消息注入、LLM/TTS/VAD 内联连通性测试。
- 📦 **运维闭环** —— OTA 固件按「功能 × 型号」托管分发（元数据自动识别，防发错包变砖）、模型自动下载、可选 Bearer 鉴权。
- 💻 **双端管理** —— Web 管理台（浏览器直开）+ macOS 原生测试台（麦克风采集 + Opus 播放，端到端语音验收）。

---

## 快速开始（Docker）

```bash
cp server/config.example.toml server-data/config.toml   # 填 expected_token / llm.api_key
docker compose up -d --build                       # 首启自动从 GitHub Release 下载模型包到 ./server-data
curl http://127.0.0.1:8000/api/health              # => {"status":"ok",...}
```

浏览器打开 `http://<host>:8000/` 即管理台（配置 / 预设 / 设备个性化 / 健康自检 / OTA）。

裸机运行：`cd server && cargo run --release --features sherpa -- --config config.toml`（需本地模型 + libopus）。
协议联调：`python3 server/tests/mock_client.py`。

---

## 文档

| 文档 | 内容 |
|---|---|
| [`docs/architecture.md`](<docs/architecture.md>) | 模块划分、会话流水线、协议设计、部署架构与扩展指引 |
| [`docs/README.md`](<docs/README.md>) | 文档索引：功能、配置、协议、管理台与模块维护入口 |
| [`server/config.example.toml`](<server/config.example.toml>) | 全部配置字段的行内注释（权威） |
| [`docs/overall-implementation-plan.md`](<docs/overall-implementation-plan.md>) | 路线图与交付状态 |
| [`docs/p1-features-prd.md`](<docs/p1-features-prd.md>) | 本期功能需求与验收标准 |

---

## 项目结构（monorepo）

```
.
├── Dockerfile / docker-compose.yml / docker-entrypoint.sh   # 部署编排
├── server/                      # Rust 服务端
│   ├── src/                    # main/config(含 overlay)/engine + app/(session/ws/protocol/events/pipeline/metrics)
│   │                           # + plugins/(asr/tts/llm/vad/aiui/memory/soul/command/wakeup/mcp + registry/health)
│   ├── config.example.toml     # 配置字段权威说明
│   └── tests/mock_client.py    # 场景化协议联调（多轮 + abort + 唤醒问候）
└── client/                      # Kuikly 多端管理台
    ├── shared/commonMain/       # 管理台 + 配置页（跨端共用）
    ├── shared/macosArm64Main/   # macOS 壳 + 测试台（仅 macOS 编译）
    └── apps/                    # h5App(Web) / macosApp(Catalyst) / iosApp / androidApp / ohosApp
```

---

## 状态与已知限制

- 未实现官方激活流程（OTA/activate），本期接受任意设备（可选 token 校验）。
- ASR 为离线逐段识别（SenseVoice），逐字流式需换 Zipformer `OnlineRecognizer`。
- RAG 召回为词法检索（零 embedding）；本地 ONNX 语义检索列为后续可选增强。
- iOS/Android/OHOS 宿主已就位但未启用语音测试台（原生桥接当前仅 macOS 编译）。

---

## 许可证

MIT（见 [`LICENSE`](<LICENSE>)）。
