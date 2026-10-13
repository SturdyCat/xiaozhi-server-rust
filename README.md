# xiaozhi-server-rust

基于 [`sherpa-onnx`](https://github.com/k2-fsa/sherpa-onnx) 的 **小智（xiaozhi-esp32）** 语音服务端，纯 Rust 实现：

- **本地 ASR**：SenseVoice INT8（离线，CPU 推理）。
- **本地 TTS**：Kokoro INT8（离线，CPU 推理）；亦支持讯飞在线合成（`tts.engine = "xfyun"`）。
- **VAD**：Silero VAD（实例池化，按会话借还），负责上行音频的语音段切分。
- **远程 LLM**：通过 OpenAI 兼容的 Chat Completions HTTP 接口接入。
- **智能上下文**（可选，全部离线）：图记忆 / 纯存储 / 本地文档 RAG 三种记忆后端、灵魂人格档案（内置预设 + 提示词预览）、唤醒问候语、语音指令闸门。
- **传输**：WebSocket 协议，严格对齐 `xiaozhi-esp32` 固件（文本消息 + 二进制 Opus 音频帧，支持 v1/v2/v3 二进制协议版本）。
- **管理台**：Web（浏览器直开）+ macOS 测试台，Kuikly 多端工程一处实现双端生效。

> 设计目标：在 Intel Celeron N5105（x86-64，Jasper Lake）这类低功耗主机上，用 Docker 跑起一个完全本地化（ASR/TTS 离线）的小智服务端，LLM 走本地或云端的 HTTP 接口。
>
> 📐 项目架构详见 [docs/architecture.md](./docs/architecture.md)（模块划分、会话流水线、协议设计、部署架构与扩展指引）。

---

## 特性

**语音链路与协议**

- 协议层严格对齐 `xiaozhi-esp32` 固件：设备 hello `version` 即二进制协议版本；服务器 hello 的 `audio_params` 作为**下行（TTS）解码参数**。
- 上行采样率跟随设备（默认 16k），下行采样率由服务器决定（默认 24k），二者独立协商。
- **下行缓冲三段式**（服务器水库 + 起跑闸 + 提前量）：上游 LLM/TTS 慢于实时时吸收停顿，长回复不断音。
- **Feature 门控**：`default`（无真实引擎，仅供 `cargo check/test` 编译；运行必须 `--features sherpa`）；`sherpa`（引入 `sherpa-onnx` / `audiopus` / `rubato`，真实引擎）。
- **无 mock**：ASR 恒为 SenseVoice、TTS 恒为 Kokoro、LLM 恒为 OpenAI 兼容 HTTP、VAD 恒为 Silero——ESP 接入走完整正式流水线；macApp 测试台连接带 `hello.test=true`，走 `asr_test`/`tts_test`/`llm_test` 三个独立服务请求-响应端点。
- 可选 Bearer Token 鉴权（配置 `server.expected_token`）。

**智能上下文（可选能力，默认零成本）**

- **记忆多后端注册表**（`[memory].engine`）：`graph`（内置图记忆：SQLite 原文 + 辅助模型抽取摘要 + 汉字二元组倒排词法召回）、`store`（纯存储：只存原文不抽取不召回，先积累再决定）、`rag`（本地文档 RAG：扫描目录对 txt/md 建词法索引，设备可回答手册/备忘里的内容，全程零外网、零向量库）。切换后端数据不丢（同一 `db_path` 各表独立）。默认关闭。
- **灵魂人格档案**（`[soul]`）：内置预设（默认人格）+ 逐字段覆盖 + `POST /api/soul/preview` 最终提示词预览（"人格配了之后到底发了什么"当场可见）。
- **语音指令闸门**（可选，`[command]`）：ASR 识别结果在送进 LLM **之前**按指令词匹配（默认「退下 / 闭嘴 / 关闭」），命中即先说一句可配置的告别语，随后**用 MCP 让设备自己结束聆听**（服务器是 MCP Client，发 `tools/call` 调设备工具 `self.audio_listener.stop`：设备回到待唤醒、**音频通道保留**，下次唤醒复用同一条连接、不再重连）。**调用决策由会话开始时探测到的每设备 MCP 工具清单驱动**：清单含 stop 工具才发起调用；清单不含（老固件）→ 零次调用直接降级断开；清单未知（探测超时）→ 沿用试探调用 + `-32602` 降级。全程不调用大模型、不产生回复开销。只对短话生效（默认 ≤ 5 个字，`max_chars` 可调）：长句携带信息，直接交给大模型。默认关闭。
- **唤醒问候语**（可选，`[wakeup]`）：固件唤醒词命中后**立刻**上报 `listen state=detect`，服务器据此马上说一句预制话术（默认「我在」），**跳过 LLM 与正常回复**——唤醒是全系统最高频的动作，为一句固定的话付一次大模型调用没有道理。成品音频（已编码的下行 Opus 帧）按**音色**三级复用：内存缓存（跨会话）→ 磁盘 `<配置文件目录>/hot-audio/`（跨重启）→ 真合成一次后自动落盘，稳态下每次唤醒**零 LLM、零 TTS**；换音色因缓存键含引擎签名/speaker/speed/采样率/帧长而自动重新合成。默认关闭。
- **AIUI 全链路**（可选，`[aiui]`，与本地级联互斥）：VAD 切段 → 讯飞云端 ASR + LLM + TTS 闭环（本地引擎闲置）。

**配置与运维**

- **配置预设**（P1-5）：把当前全局配置快照为 `presets/<name>.toml`，管理台一键应用（复用部分更新语义 + 原子写）/ 删除，重启保留。
- **每设备个性化 overlay**（P1-5）：`overlays/devices/<device_id>.toml` 只存**白名单字段**（音色/语速/提示词/历史长度/人格/唤醒话术/指令词表/记忆召回参数等会话内参数）的增量；`server`/`audio`/引擎选择等结构型字段不可覆盖（越界写入被拒并给出「怎么修」提示）；无 overlay 的设备行为与现状完全一致（零成本契约）。生效公式：`生效配置(设备) = 默认值 ⊕ config.toml ⊕ 设备 overlay`。
- **引擎健康自检**（P1-6）：`GET /api/health/engines` 对全部 11 个 capability 聚合 `enabled/phase/last_error`（纯只读投影），管理台概览页 + 配置页健康卡直接可见"记忆为什么没生效、怎么修"。
- **运维面板**：延迟大盘（`/api/metrics` 整轮与各段首包 p50/p95）、设备消息注入（`POST /api/devices/{id}/inject`，文本作为用户话语走完整管线）、MCP 工具清单可视化（含「指令闸门兼容」标记）。

---

## 当前依赖版本（已核对官方文档）

- `sherpa-onnx` **1.13.8**（截至 2026-09 的最新稳定版；官方 Rust API 文档：<https://k2-fsa.github.io/sherpa/onnx/rust-api/index.html>）。
- `axum` 0.8（内置 `ws`，无需 `tokio-tungstenite`），`tokio` 1.44，`reqwest` 0.12（rustls）。

API 已对照 1.13.8 rustdoc 校验：`OfflineSenseVoiceModelConfig`、`OfflineTtsKokoroModelConfig`、`VoiceActivityDetector` 的字段与方法签名与实现一致。

---

## 快速开始（Docker，正式流程）

```bash
cp server/config.example.toml server-data/config.toml   # 填 expected_token / llm.api_key
docker compose up -d --build                       # 首启自动从 GitHub Release 下载模型包到 ./server-data
curl http://127.0.0.1:8000/api/health              # => {"status":"ok",...}
```

浏览器打开 `http://<host>:8000/` 即管理后台（配置读写 / 测试台）。

裸机运行（需本地模型 + libopus）：`cd server && cargo run --release --features sherpa -- --config config.toml`。

协议联调：`python3 server/tests/mock_client.py`（模拟管理端测试台，验证三个独立服务端点，见下文 [测试](#测试)）。

---

## 配置

配置加载优先级：**`XIAOZHI_CONFIG` 环境变量** → **`--config <path>` 参数** → **内置默认（/data/models 生产路径）**。

```bash
# 方式一：环境变量
export XIAOZHI_CONFIG=$PWD/server/config.toml
cd server && ~/.cargo/bin/cargo run

# 方式二：命令行参数
cd server && ~/.cargo/bin/cargo run -- --config config.toml
```

配置字段说明见 [`server/config.example.toml`](./server/config.example.toml)。关键项：

- `server.port`：监听端口，默认 `8000`（服务器永远监听 `0.0.0.0`，外部可达性由端口映射/防火墙决定）。
- `server.expected_token`：Bearer token；为空表示不校验 `Authorization` 头。
- `audio.downlink_sample_rate` / `downlink_frame_duration_ms`：下行（TTS）采样率与帧长，写入服务器 hello。
- `audio.binary_protocol_version`：下行二进制协议版本（1/2/3）。**建议先用 1 真机验证，再切 2/3。**
- `audio.downlink_buffer_ms` / `downlink_buffer_start_ms`：服务器侧下行水库与起跑闸——上游 LLM/TTS 慢于实时时长回复断音的解药（详见 example 内注释）。
- `llm.api_base` / `api_key` / `model` / `system_prompt`：OpenAI 兼容 Responses API（LLM 恒为真实 HTTP，无 mock）。
- `tts.engine`：`kokoro`（本地 Kokoro INT8，默认，参数见 `[tts.kokoro]` 子段）或 `xfyun`（科大讯飞在线合成，凭据见 `[tts.xfyun]`）。旧写法（`backend = "sherpa"` + `[tts]` 直挂模型路径）加载时自动迁移到新结构，保存一次即写成新格式。
- `command.enabled` / `keywords` / `match_mode` / `max_chars` / `reply`：语音指令闸门（ASR → LLM 之间）。开启后识别到指令词（默认「退下 / 闭嘴 / 关闭」）会先说 `reply`，再**用 MCP 让设备结束聆听**（`tools/call` → `self.audio_listener.stop`；设备回到待唤醒、音频通道保留，下次唤醒不重连），跳过 LLM。调用决策由**每设备 MCP 工具清单驱动**：清单含 stop 才调用；清单不含（老固件）→ 零次调用直接降级断开（管理台 MCP 工具页有「指令闸门兼容」标记）；清单未知 → 试探调用 + `-32602` 降级。
  `match_mode = "exact"`（默认）要求整句相等——识别文本会先去标点并剥掉句末语气词（「退下吧。」≡「退下」），因此**不会**把「关闭闹钟」误判为关机；改成 `"contains"` 才是"句中出现即命中"。
  `max_chars = 5`（默认）是**长度闸门**：归一化后超过这么多字的识别文本一律不做指令判断、直接交给大模型——「帮我关闭卧室的灯」含「关闭」但不会被断线；`0` = 不限制。注意指令词本身长于 `max_chars` 时会永远命中不了，保存时会被拦下提示。保存后**新会话**生效。
- `wakeup.enabled` / `greeting` / `persist`：唤醒问候语（唤醒 → LLM 之间）。设备唤醒词命中后固件**立刻**上报 `listen state=detect`（`CONFIG_SEND_WAKE_WORD_DATA=y`），服务器收到即说 `greeting`（默认「我在」），**不调用 LLM、不进入正常回复**。音频三级复用：内存 TTS 缓存 → 磁盘 `<配置文件目录>/hot-audio/`（`persist`）→ 真合成一次后落盘；因此稳态下每次唤醒零 LLM、零 TTS。缓存键含引擎签名/speaker/speed/下行采样率/帧长/文本，**换音色会自动重新合成并另存一份**，不会串用旧音色。`persist` 需要 `[tts].cache_entries > 0`，否则配置校验会拦下并说明怎么改。建议问候语 ≤ 10 字：它播放期间上行音频在缓冲区排队，太长会推迟用户第一句话。保存后**新会话**生效。
- `memory.enabled` / `engine` / `recall_*` / `inject_position`：记忆（默认关闭）。`engine` 三选一：`graph`（图记忆：原文 + 抽取摘要 + 词法召回）、`store`（只存原文不抽取不召回）、`rag`（本地文档 RAG，文档目录见 `[memory.rag].docs_dir`）。`db_path` / `docs_dir` 必须在挂载卷内。`[memory.extractor]` 可为抽取单独路由更便宜的小模型。
- `soul.enabled` / `preset` / 人格字段：灵魂人格档案（留空字段由内置预设补，填写即覆盖）；`POST /api/soul/preview` 可在保存前预览最终提示词。
- `aiui.*`：讯飞 AIUI 全链路（云端 ASR+LLM+TTS 闭环）。⚠️ 启用后本地 ASR/LLM/TTS 引擎与 `[soul]`/`[memory]` 均不参与。

### 配置预设与每设备 overlay（落盘布局）

与主 `config.toml` 同目录（P1-5）：

| 目录 | 内容 | 说明 |
|---|---|---|
| `presets/<name>.toml` | 全局配置快照 | 管理台「配置 → 预设」保存/应用/删除；应用 = 复用 `PUT /api/config` 部分更新管线（含密钥护栏与生效粒度回报） |
| `overlays/devices/<device_id>.toml` | 单设备增量 | 仅白名单会话内字段；设备页「个性化配置」入口或 `PUT /api/devices/{id}/overlay` 写入 |

```bash
curl http://<host>:8000/api/devices/<device_id>/overlay        # 查看设备 overlay（无则 {}）
curl -X PUT -H "Content-Type: application/json" \
     -d '{"tts":{"speaker":3,"speed":1.2}}' \
     http://<host>:8000/api/devices/<device_id>/overlay        # 该设备换音色/语速（新会话生效）
curl -X POST http://<host>:8000/api/config/presets -d '{"name":"night"}'   # 当前配置快照为预设
curl -X POST http://<host>:8000/api/config/presets/night/apply             # 应用预设
```

> ⚠️ **容器部署**：`presets/`、`overlays/`、`hot-audio/` 与 `config.toml`、`memory.db`、`rag-docs/` 同理，都在配置文件所在目录下派生——**配置文件必须放在挂载卷内**，否则重建容器即丢预设/overlay/记忆/问候音频。


---

## 真实引擎（`sherpa` feature）

真实引擎需要：

1. 系统 `libopus`（audiopus 编译/链接依赖，且 `sherpa-onnx` 部分路径需要）。
2. 网络（构建脚本/原生库下载）。
3. 本地模型文件（SenseVoice / Kokoro / Silero VAD）。

### 1. 下载模型

```bash
./server/scripts/download_models.sh /host/data/models
```

脚本会从 k2-fsa/sherpa-onnx 官方 GitHub Release 下载整包 tar.bz2 并本地解压（代理由 compose 的 `GITHUB_PROXY` 决定，不配置则直连原始地址），并提示核对文件路径。下载后按 `config.example.toml` 的 `[asr]` / `[vad]` / `[tts]` 路径对齐 `model` / `tokens` / `voices` 等。

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
  -v /host/data/models:/data/models \
  -v $PWD/config.toml:/app/config.toml \
  -e XIAOZHI_CONFIG=/app/config.toml \
  xiaozhi-server-rust:latest
```

### 模型自动下载

容器入口（`docker-entrypoint.sh`）会在启动时检测关键模型文件（`silero_vad.onnx`、`SenseVoiceSmall/tokens.txt`、`Kokoro/model.int8.onnx`）：

- **缺失则自动下载**到挂载的 `/data/models`（默认行为，从官方 GitHub Release 拉整包 tar.bz2 本地解压，按模型幂等 + 断点续传）。
- 默认**直连原始地址**；docker compose 配置了 `GITHUB_PROXY` 才走代理（如 `https://tvv.tw/`）。下载地址可用 `SENSEVOICE_URL` / `KOKORO_URL` / `SILERO_VAD_URL` 覆盖为完整直链（内网镜像，不会被二次套代理）。
- 行为开关 `XIAOZHI_AUTO_DOWNLOAD_MODELS`：`missing`（默认，缺失才下）/ `force`（每次重下）/ `off`（不下载，依赖挂载或预置）。

> 因此**不手动预置模型也能直接 `docker compose up` 跑起来**（模型从 GitHub Release 自动拉取，代理可配）。
> 离线/内网环境：先 `SENSEVOICE_URL=<内网镜像仓库> ./scripts/download_models.sh /host/data/models` 预置，再挂载，或设 `XIAOZHI_AUTO_DOWNLOAD_MODELS=off`。

### 部署机指令集

构建期固定 `RUSTFLAGS="-C target-cpu=x86-64-v2"`（见 `Dockerfile`），以兼容部署机 N5105（Tremont，无 AVX）。**不要用 `native`**——CI 构建机 CPU 与部署机不同，`native` 会嵌入部署机不支持的 AVX/AVX2/AVX-512 指令，导致运行时 `SIGILL` 崩溃。

---

## 协议要点

- **设备 hello**：`{"type":"hello","version":1,"audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60},"features":{...}}`。`version` = 二进制协议版本（1/2/3）。
- **服务器 hello**：回 `transport`、`session_id`、`audio_params`（下行 TTS 解码参数）。
- **一次对话**：`listen start` → `stt` → `llm`（首包）→ `tts start` → `tts sentence_start`（带文本）→ 若干**下行二进制 Opus 帧** → `tts stop`。
- **上行音频**：设备以二进制帧（按协商版本封装）发送 Opus；服务器解码 → VAD → ASR → LLM → TTS（正式流水线，无 mock 捷径）。`listen start` 仅状态同步，不触发对话。
- **abort**：任何阶段客户端可发 `{"type":"abort"}` 中断当前 TTS 下行。

二进制帧封装（v1/v2/v3 字节布局）见 `server/src/app/protocol.rs` 的模块注释（权威说明）；`wrap_downlink` / `unwrap_uplink` 是唯一的封装/解封装出口。

---

## 测试

### 单元自测（Rust）

协议/音频/配置/插件/记忆/overlay/preset 各层均含 `#[cfg(test)]` 用例（二进制版本往返、密钥护栏与部分更新语义、overlay 白名单拒止、预设落盘往返、registry 护栏、VAD/ASR 桩等，全量 468+ 例）：

```bash
cd server && ~/.cargo/bin/cargo test
```

### 协议联调（Python 测试台客户端）

`tests/mock_client.py` 是一个**零第三方依赖**的 WebSocket 客户端（纯标准库实现 RFC 6455 帧）。它模拟 macApp 测试台（hello 带 `test:true`），**场景化回归**：握手协商、`asr_test`/`tts_test`/`llm_test` 三个独立服务端点、`abort` 打断后连接存活探针（fatal）、唤醒问候（容忍型：未启用 `[wakeup]` 只记 warning）、多轮对话：

```bash
# 先启动服务（需 --features sherpa 与本地模型；或直接用 Docker）
cd server && cargo run --release --features sherpa -- --config config.toml &

# 运行联调客户端
python3 server/tests/mock_client.py
# => 逐场景输出 PASS / WARNING，末尾汇总 PASS 或 FAIL
```

可用参数：`--host 127.0.0.1 --port 8000 --token <bearer>`（当 `expected_token` 非空时传 `--token`）。

### 管理后台（Web 管理台）

管理后台是一个 **Kuikly 多端工程**（`client/`），位于 `client/shared` 公共层，跨端复用（web / Android / iOS / OHOS）。
其 web 构建产物由 server 在 `/` 静态托管。

默认落地页是 **管理台**（`@Page("admin")`）：左侧分组导航 + 搜索框 + 右侧单 section，与参考项目 `xiaozhi-esp32-server` 的
`manager-web` 同一信息架构（那边是「首页看板 + 设备管理 + 模型配置 + …」，本项目按自身能力裁剪）：

| Section | 数据源 | 回答什么问题 |
|---|---|---|
| 概览 | `GET /api/stats` + `/api/plugins` | 现在有几台设备在线、活会话多少、11 个能力有没有异常（degraded/failed 可点击跳配置） |
| 测试台 | —（Web 占位） | Web 端无麦克风显示不可用说明 + 替代路径（LLM/TTS/VAD 内联测试在配置页可用）；端到端语音测试用 macOS 测试台 |
| 设备 | `GET /api/devices` | 哪些设备连过、最后活跃、协商参数、上次会话为什么断；支持消息注入、个性化配置入口 |
| 会话 | `GET /api/sessions` | **此刻**哪些连接活着、各自在聆听/思考/说话 |
| 对话记录 | `GET /api/conversations` | 最近聊了什么（内存环形，非持久化） |
| 用量 | `GET /api/usage` | 令牌花在哪、上下文被记忆/人格撑大了多少 |
| 延迟 | `GET /api/metrics` | 整轮与各段（VAD/ASR/LLM/TTS）首包延迟 p50/p95、错误率（内存态） |
| 配置 | `GET/POST /api/config` 等 | 读写 `config.toml` 全部参数 + 预设 + 每设备 overlay + 能力健康卡 + 内联连通性测试 |

> ⚠️ **设备/会话/对话记录/延迟是进程内内存态**（口径写在每个页面与每个响应里）：重启清零、
> `online` 由会话生命周期派生而非心跳、对话只保留最近若干轮。要长期留存请开启 `[memory]`。
>
> 单页模式仍可用 `?page_name=config` 单独打开配置页（`@Page("config")`，支持 `&edit_device=<id>` 直接进入某设备个性化编辑；适合嵌入式/调试场景）。

### HTTP API 一览（管理动作）

| 端点 | 语义 |
|---|---|
| `GET /api/health` / `GET /api/health/engines` | 存活探针 / 11 个 capability 健康自检（纯只读投影，幂等无副作用） |
| `GET / POST(=PUT) /api/config` | 配置读写（部分更新：缺失字段保持原值、显式 `null` 清除、密钥空串保留、`expected_revision` 乐观并发 409 拦过期写入） |
| `GET/PUT/DELETE /api/devices/{id}/overlay` | 每设备白名单增量（POST = PUT、`POST .../overlay/delete` = DELETE 的别名，供 Kuikly 客户端使用） |
| `GET/POST /api/config/presets`、`POST .../{name}/apply`、`DELETE/POST .../{name}/delete` | 配置预设：列出 / 快照当前 / 应用 / 删除 |
| `POST /api/test/{llm,tts,vad}` | 内联连通性与首包延迟测试（按磁盘最新配置 ad-hoc 建引擎，无需麦克风，Web 可用） |
| `GET /api/memory/status`、`POST /api/memory/{recall,maintain,clear,rag/reindex}` | 记忆状态 / 手动试召回 / 立即维护 / 清空 / RAG 索引重建 |
| `GET /api/soul/presets`、`POST /api/soul/preview` | 内置人格预设清单 / 最终提示词预览（保存前校验器） |
| `POST /api/devices/{id}/inject` | 设备消息注入（文本作为用户话语走完整管线） |
| `GET /api/metrics`、`GET /api/usage` | 延迟大盘 / 令牌用量（内存态） |

配置读写沿用 `GET` / `POST`（或 `PUT`）`/api/config`，**部分更新**语义（缺失字段保持原值、显式 `null` 清除、
密钥空串保留），无需手改文件；auth 沿用 `server.expected_token` 语义，不另造简化鉴权。

- **访问**：启动 server（带 `--config config.toml`）后浏览器打开 `http://<host>/`（默认即管理台）；
  修改配置后点「保存配置」写回 `config.toml`（原 TOML 注释会被覆盖）。
- **API**：
  - `GET /api/config` → 返回当前配置 JSON（启动时指定了文件则实时读盘）。
  - `POST /api/config`（亦接受 `PUT`）→ 接收配置 JSON 并写回启动加载的配置文件（部分更新）；
    **引擎相关参数（ASR/TTS/LLM）对新会话生效**，`[server]`/`[audio]` 部分需重启。
- **注意**：以内置默认配置启动（未指定文件）时保存无目标文件，会返回 400；请用 `--config` 指定 `config.toml` 后重启再保存。

```bash
# 构建 web 管理页（产物目录含 index.html 与 nativevue2.js，由 [server].admin_dir 指向）
cd client && ./gradlew :apps:h5App:publishWeb

# 启动 server（admin_dir 默认 ../client/apps/h5App/web，按实际构建输出调整）
cd server && ~/.cargo/bin/cargo run -- --config config.toml
# 浏览器访问 http://127.0.0.1:8000/（管理台）；单页配置页：?page_name=config
```

### 固件 OTA 托管（多型号 / 多功能）

设备开机 POST `/api/ota` 检查版本，服务端按设备上报的 **功能 + 型号** 匹配托管固件，命中且版本更新才下发下载地址（设备自升级）。管理页「OTA」tab 即这套托管的上传/清单/删除界面。

- **寻址维度**：`功能(project) × 型号(model)`，可选 `板型(board)` 收窄。功能 = 固件 `esp_app_desc_t.project_name`（设备上报 `application.name`），型号 = 芯片（设备上报 `chip_model_name`，与 `CONFIG_IDF_TARGET` 同名，如 `esp32c3` / `esp32s3`），板型 = 设备 `board.type`（如 `bread-compact-wifi`）。
- **匹配规则**：功能与型号**必须完全相同**；同型号下优先"板型精确匹配"，否则回落到"该型号通用"（上传时不填 board）。**不同型号/不同功能的包绝不互发**——发错包等于变砖；没有匹配就不下发升级地址。
- **元数据自动识别**：ESP-IDF app 镜像自带元数据（镜像头 `chip_id` + `esp_app_desc_t`），上传时服务端自动解析出 型号/功能/版本/IDF 版本/编译时间，**无需手填**；非 ESP-IDF 镜像或字段为空时用查询参数覆盖。
- **存储布局**：`<XIAOZHI_FIRMWARE_DIR>/<project>/<model>/<board|_>/firmware.bin + meta.json`（默认 `/data/firmware`，随 compose 的 `/data` 卷持久化）。同一 key 上传即覆盖，不留历史版本。

| 端点 | 语义 |
|---|---|
| `POST /api/ota/firmware` | 上传 app bin（body = 裸二进制；`expected_token` 非空时需 `Authorization: Bearer`）。可选覆盖：`?project=&model=&board=&version=`。返回解析后的元数据 + `metadata_from`（`package`/`query`） |
| `GET /api/ota/firmware` | 托管清单（管理页数据源）：`{dir, count, entries[]}` |
| `GET /api/ota/firmware/latest?project=&model=&board=` | 设备下载（**不鉴权**，设备无法带 Authorization 头）：裸 bin 流，`Content-Length` 完整 |
| `POST /api/ota/firmware/delete` / `DELETE /api/ota/firmware` | 删除条目（管理动作，需鉴权）：`?project=&model=&board=` |

```bash
# 上传（型号/版本自动识别，无需参数）
curl -H "Authorization: Bearer <expected_token>" \
     --data-binary @build/xiaozhi-esp32.bin \
     "http://<host>:8000/api/ota/firmware"
# 查看清单 / 删除
curl "http://<host>:8000/api/ota/firmware"
curl -X POST -H "Authorization: Bearer <expected_token>" \
     "http://<host>:8000/api/ota/firmware/delete?project=xiaozhi&model=esp32s3&board="
```

> 旧版（只保留一个根目录固件）的 `firmware.bin` + `latest.json` 会被列为 `_legacy` 条目：可删除、可查看，但**不参与匹配**（没有型号信息，继续下发等于把不确定的包发给任何设备）——请按 功能+型号 重新上传。

### ASR/TTS 测试台（macOS App）

`web` 受浏览器麦克风 / WebSocket 二进制等限制，不适合做语音链路测试。因此另起 **macOS App**（`apps/macosApp`，Mac Catalyst，复用 iOS 渲染器 `OpenKuiklyIOSRender`）承载 ASR/TTS 测试，替代原 web 测试台：

- **测试页仅放在 `client/shared/src/macosArm64Main`**：只编译进 macOS 框架，**web/Android/iOS/OHOS 的包都不含测试代码**；web 管理台侧边栏的「测试台」入口会显示不可用说明与替代路径（P1-3 平台分支）。
- **四个 tab**：连接（服务器地址 + 自动 WS）/ ASR 识别（录音试听 → 发送识别解耦 + 波形回放）/ TTS 合成（音色下拉 + 语速，可临时覆盖当前配置音色）/ LLM 对话；每个 tab 右侧内嵌对应配置卡（改动即存表单，配合顶栏「保存配置」一键写回）。
- 测试逻辑复用 server 的专用测试协议（见 `server/src/app/protocol.rs` / `server/src/app/session.rs`）：
  - `asr_test {action: start|stop}`：整段录音缓冲后一次性识别，回 `stt`；
  - `tts_test {text, speaker, lang, speed}`：合成并流式下发 Opus 音频；
  - `llm_test {text}`：LLM 连通性对话。
- 原生桥接在 `apps/macosApp/XiaoZhiModule*.m`（Kuikly 自定义 Module `XiaoZhiModule`）：WS 建连、麦克风采集、音频播放、**Opus 编解码**（`XiaoZhiModule+Opus.m`，libopus）均已实现，测试台端到端可用。

```bash
# ① 编出 KMP 业务框架 shared.framework（含管理页 + 测试台，仅 Apple Silicon）
cd client && ./gradlew :shared:linkReleaseFrameworkMacosArm64
#    产物在 client/shared/build/bin/macosArm64/releaseFramework/shared.framework
#    （注：Kotlin/Native 没有 assembleMacosArm64 任务，正确任务名是 linkReleaseFrameworkMacosArm64；
#     改动任何 @Page 或 shared 业务代码后必须重跑本任务，否则 Xcode 跑的是旧框架）

# ② 生成 Xcode 工程（需 brew install xcodegen）
cd client/apps/macosApp && xcodegen generate        # 生成 macosApp.xcodeproj，并 embed 上面的 shared.framework

# ③ 安装渲染器依赖（需 CocoaPods：sudo gem install cocoapods 或 brew install cocoapods）
pod install                                        # 拉取 OpenKuiklyIOSRender，生成 macosApp.xcworkspace

# ④ 用 Xcode 打开 workspace，选「My Mac (Mac Catalyst)」运行
open macosApp.xcworkspace
```

> 启动顺序不可省：必须先 ① 编出 `shared.framework`，② 的 xcodegen 才能把框架正确 embed 进 App；
> 否则链接阶段报 `shared.framework not found`。Xcode 重跑后若白屏（页面回退占位），先 Clean Build Folder。

> 协议要点与 `tests/mock_client.py` 共用同一套 `hello(test=true) / asr_test / tts_test / llm_test` 语义（macApp 测试台同款），便于回归。

---

## 项目结构（monorepo）

本仓库为 monorepo：`server/` 是 Rust 服务端，`client/` 是 Kuikly 多端工程（管理后台 web）。

```
.
├── Dockerfile / docker-compose.yml / docker-entrypoint.sh   # 部署编排（仓库根）
├── server/                      # Rust 服务端（模块职责与复杂逻辑见 server/src/**/*.rs 的 //! 注释；整体架构见 docs/architecture.md）
│   ├── src/                    # 根：main/config（含 config/overlay.rs 每设备白名单）/engine；
│   │                           # app/ 应用层（session/ws/protocol/events/pipeline/metrics/…）；
│   │                           # plugins/ 引擎与能力插件（asr/tts/llm/vad/aiui/memory/soul/command/wakeup/mcp + registry/health）—— 各文件顶部 //! 即权威说明
│   ├── config.toml / config.example.toml
│   ├── scripts/download_models.sh
│   └── tests/mock_client.py    # 场景化协议联调（握手/识别/合成/对话 + abort 打断 + 唤醒问候，多轮回归）
└── client/                      # Kuikly 多端工程（管理后台）
    ├── settings.gradle.kts      rootProject.name = "client"
    ├── shared/
    │   ├── commonMain/         公共代码：@Page("admin") 管理台 + @Page("config") 配置页（跨端）+ 配置表单/预设/设备 overlay 编辑 + 运维面板
    │   └── macosArm64Main/     仅 macOS(arm64) 编译：@Page("router") 单窗口壳（连接页 + 测试台 + 配置）+ XiaoZhiModule
    └── apps/
        ├── h5App/               Web(H5) 宿主：Main.kt + index.html（管理台 + 配置页；无语音测试）
        ├── androidApp/          Android 宿主（仅 arm64-v8a，ndk.abiFilters 限定）
        ├── iosApp/              iOS 宿主（原生；仅 arm64：iosArm64 + Apple Silicon 模拟器，Podfile 排除 x86_64）
        ├── macosApp/            macOS 宿主（原生，Mac Catalyst）：ASR/TTS/LLM 测试台（仅 arm64）
        └── ohosApp/             HarmonyOS 宿主（原生）
```

> server 在 `/` 静态托管 `client/apps/h5App` 的构建产物（由 `[server].admin_dir` 指定目录），
> 并通过 `GET/POST /api/config` 让管理页读写 `config.toml`。

---

## 状态与已知限制

- **本期未实现激活流程（OTA/activate）**：标准 xiaozhi-server 含激活/OTP，本期接受任意设备（可选 token 校验）。如需对接官方激活另开任务。
- **ASR 流式**：本期 SenseVoice 为离线逐段识别；如需逐字流式可后续换 Zipformer `OnlineRecognizer`。
- **未启用 `sherpa` 的编译下行音频为空帧**：`encode_opus_frame` 占位实现返回空（仅供编译/测试）；真实音频需 `--features sherpa` + libopus。
- **RAG 召回为词法检索**（BM25/倒排路线，与图记忆同一中文分词口径）：无 embedding；语义检索（本地 ONNX 小模型）列为后续可选增强。
- **iOS/Android/OHOS 宿主已就位但未启用语音测试台**：测试台原生桥接当前仅在 macOS 编译；跨端复用下沉列为按需项。

---

## 许可证

MIT（见 [`LICENSE`](./LICENSE)）。
