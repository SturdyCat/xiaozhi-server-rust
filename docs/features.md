# 功能与配置详解

> 本文是 [README](../README.md) 的展开：功能细节、配置字段、HTTP API、OTA 托管、测试台构建与测试回归。
> 权威配置字段说明以 [`server/config.example.toml`](../server/config.example.toml) 的行内注释为准；整体架构见 [architecture.md](./architecture.md)。

---

## 1. 语音链路与协议

- 协议层严格对齐 `xiaozhi-esp32` 固件：设备 hello `version` 即二进制协议版本（1/2/3）；服务器 hello 的 `audio_params` 作为**下行（TTS）解码参数**。
- 上行采样率跟随设备（默认 16k），下行采样率由服务器决定（默认 24k），二者独立协商。
- **下行缓冲三段式**（提前量 + 服务器水库 + 起跑闸）：上游 LLM/TTS 慢于实时时吸收停顿，长回复不断音。
- **Feature 门控**：`default`（无真实引擎，仅供 `cargo check/test` 编译；运行必须 `--features sherpa`）；`sherpa`（引入 `sherpa-onnx` / `audiopus` / `rubato`，真实引擎）。
- **无 mock**：ASR 恒为 SenseVoice、TTS 恒为 Kokoro、LLM 恒为 OpenAI 兼容 HTTP、VAD 恒为 Silero——ESP 接入走完整正式流水线；macApp 测试台连接带 `hello.test=true`，走 `asr_test`/`tts_test`/`llm_test` 三个独立服务请求-响应端点。
- 可选 Bearer Token 鉴权（配置 `server.expected_token`）。

### 协议要点

- **设备 hello**：`{"type":"hello","version":1,"audio_params":{...},"features":{...}}`。`version` = 二进制协议版本（1/2/3）。
- **服务器 hello**：回 `transport`、`session_id`、`audio_params`（下行 TTS 解码参数）。
- **一次对话**：`listen start` → `stt` → `llm`（首包）→ `tts start` → `tts sentence_start`（带文本）→ 若干**下行二进制 Opus 帧** → `tts stop`。
- **上行音频**：设备以二进制帧（按协商版本封装）发送 Opus；服务器解码 → VAD → ASR → LLM → TTS（正式流水线，无 mock 捷径）。`listen start` 仅状态同步，不触发对话。
- **abort**：任何阶段客户端可发 `{"type":"abort"}` 中断当前 TTS 下行。

二进制帧封装（v1/v2/v3 字节布局）见 `server/src/app/protocol.rs` 的模块注释（权威说明）；`wrap_downlink` / `unwrap_uplink` 是唯一的封装/解封装出口。

---

## 2. 智能上下文（可选能力，默认全部关闭、零成本）

- **记忆多后端注册表**（`[memory].engine`）：
  - `graph`：内置图记忆——SQLite 原文事实源 + 辅助模型抽取摘要 + 汉字二元组倒排**词法召回**（零外网），注入原始 Q/A 作为背景资料；
  - `store`：纯存储——只存原文、不抽取、不召回，定位"先积累数据再决定"；
  - `rag`：本地文档 RAG——扫描 `[memory.rag].docs_dir`（默认 `/data/rag-docs`）对 txt/md 建词法索引（后台任务，重建不阻塞对话），设备可回答手册/备忘里的内容；全程零外网、零向量库。
  - 切换后端数据不丢（同一 `db_path`，graph/store/rag 各表独立）；`[memory.extractor]` 可为抽取单独路由更便宜的小模型。
- **灵魂人格档案**（`[soul]`）：内置预设（默认人格）打底 + 逐字段覆盖（留空 = 预设补）；`POST /api/soul/preview` 展开最终提示词（来源/预设/覆盖字段/字符数），保存前即可校验变量拼写。
- **唤醒问候语**（`[wakeup]`）：固件唤醒词命中后立刻上报 `listen state=detect`，服务器马上说 `greeting`（默认「我在」），**跳过 LLM**。成品音频按音色三级复用：内存缓存 → 磁盘 `hot-audio/`（跨重启）→ 真合成一次落盘，稳态零 LLM 零 TTS；换音色因缓存键含引擎签名/speaker/speed 自动重合成。`persist` 需要 `[tts].cache_entries > 0`。
- **语音指令闸门**（`[command]`）：ASR 文本送 LLM **之前**按指令词匹配（默认「退下 / 闭嘴 / 关闭」），命中先说告别语再**用 MCP 让设备结束聆听**（`tools/call` → `self.audio_listener.stop`：设备回待唤醒、音频通道保留、下次唤醒不重连）。**调用决策由每设备 MCP 工具清单驱动**：清单含 stop 才调用；不含（老固件）→ 零次调用直接降级断开（管理台 MCP 工具页有「指令闸门兼容」标记）；清单未知 → 试探调用 + `-32602` 降级。`match_mode="exact"`（默认，整句相等、去标点剥语气词，不会误伤「关闭闹钟」）；`max_chars=5` 为长度闸门（长句直接交给大模型，`0` 不限制）。
- **AIUI 全链路**（`[aiui]`，与本地级联互斥）：VAD 切段 → 讯飞云端 ASR + LLM + TTS 闭环；启用后本地引擎与 `[soul]`/`[memory]` 均不参与。

---

## 3. 配置

配置加载优先级：**`XIAOZHI_CONFIG` 环境变量** → **`--config <path>` 参数** → **内置默认（/data/models 生产路径）**。

```bash
# 方式一：环境变量
export XIAOZHI_CONFIG=$PWD/server/config.toml
cd server && ~/.cargo/bin/cargo run

# 方式二：命令行参数
cd server && ~/.cargo/bin/cargo run -- --config config.toml
```

关键项（完整字段见 `config.example.toml`）：

- `server.port`（默认 8000，恒监听 `0.0.0.0`）；`server.expected_token`（Bearer，空 = 不校验）。
- `audio.downlink_sample_rate` / `downlink_frame_duration_ms`：下行协商参数；`binary_protocol_version` 建议 1 真机验证后再切 2/3。
- `audio.downlink_buffer_ms` / `downlink_buffer_start_ms` / `downlink_lead_ms`：下行缓冲三段式（详见 example 注释，重启生效）。
- `llm.api_base` / `api_key` / `model` / `system_prompt`：OpenAI 兼容 Responses API。
- `tts.engine`：`kokoro`（本地 INT8，默认，参数进 `[tts.kokoro]` 子段）或 `xfyun`（在线，凭据进 `[tts.xfyun]`）。旧写法（`backend="sherpa"` + `[tts]` 直挂模型路径）加载时自动迁移，保存一次写成新结构。改后**新会话/测试台立即热切换**。
- `command.*` / `wakeup.*` / `memory.*` / `soul.*` / `aiui.*`：见 §2。

---

## 4. 配置预设与每设备 overlay（落盘布局）

与主 `config.toml` 同目录：

| 目录 | 内容 | 说明 |
|---|---|---|
| `presets/<name>.toml` | 全局配置快照 | 管理台「配置 → 预设」保存/应用/删除；应用 = 复用 `PUT /api/config` 部分更新管线（含密钥护栏与生效粒度回报） |
| `overlays/devices/<device_id>.toml` | 单设备增量 | 仅白名单会话内字段（音色/语速/音色语言、提示词与历史长度、人格、唤醒话术、指令词表、记忆召回参数）；`server`/`audio`/引擎选择等结构型字段不可覆盖（越界写入被拒并给出「怎么修」提示）；无 overlay 的设备行为与现状完全一致 |

生效公式：`生效配置(设备) = 默认值 ⊕ config.toml（全局） ⊕ 设备 overlay`。overlay 引擎不受影响：引擎仍只按全局配置构建（overlay 永不触发引擎重建）。

```bash
curl http://<host>:8000/api/devices/<device_id>/overlay        # 查看设备 overlay（无则 {}）
curl -X PUT -H "Content-Type: application/json" \
     -d '{"tts":{"speaker":3,"speed":1.2}}' \
     http://<host>:8000/api/devices/<device_id>/overlay        # 该设备换音色/语速（新会话生效）
curl -X POST http://<host>:8000/api/config/presets -d '{"name":"night"}'   # 当前配置快照为预设
curl -X POST http://<host>:8000/api/config/presets/night/apply             # 应用预设
```

> ⚠️ **容器部署**：`presets/`、`overlays/`、`hot-audio/`、`memory.db`、`rag-docs/` 都在配置文件所在目录下派生——**配置文件必须放在挂载卷内**，否则重建容器即丢预设/overlay/记忆/问候音频。

---

## 5. 管理台（Web + macOS）

管理台是 **Kuikly 多端工程**（`client/`），逻辑在 `client/shared/commonMain` 跨端复用；web 构建产物由 server 在 `/` 静态托管。

默认落地页 **管理台**（`@Page("admin")`）：左侧分组导航 + 搜索 + 右侧单 section：

| Section | 数据源 | 回答什么问题 |
|---|---|---|
| 概览 | `GET /api/stats` + `/api/plugins` | 设备在线数、活会话、11 个能力健康（degraded/failed 可点击跳配置） |
| 测试台 | —（Web 占位） | Web 无麦克风 → 不可用说明 + 替代路径；端到端语音测试用 macOS 测试台 |
| 设备 | `GET /api/devices` | 连过的设备、协商参数、上次会话为什么断；消息注入、个性化配置入口 |
| 会话 | `GET /api/sessions` | 此刻活着的连接与阶段（聆听/思考/说话） |
| 对话记录 | `GET /api/conversations` | 最近聊了什么（内存环形，非持久化） |
| 用量 | `GET /api/usage` | 令牌去向、记忆/人格把上下文撑大了多少 |
| 延迟 | `GET /api/metrics` | 整轮与各段首包延迟 p50/p95、错误率（内存态） |
| 配置 | `/api/config` 等 | 全部参数 + 预设 + 每设备 overlay + 能力健康卡 + 内联连通性测试 |

> ⚠️ 设备/会话/对话记录/延迟是**进程内存态**：重启清零、`online` 由会话生命周期派生、对话只留最近若干轮。长期留存请开启 `[memory]`。
> 单页模式：`?page_name=config`（支持 `&edit_device=<id>` 直接进入某设备个性化编辑）。

### HTTP API 一览（管理动作）

| 端点 | 语义 |
|---|---|
| `GET /api/health` / `GET /api/health/engines` | 存活探针 / 11 capability 健康自检（纯只读投影，幂等无副作用） |
| `GET / POST(=PUT) /api/config` | 配置读写（部分更新：缺失保持原值、显式 `null` 清除、密钥空串保留、`expected_revision` 乐观并发 409 拦过期写入） |
| `GET/PUT/DELETE /api/devices/{id}/overlay` | 每设备白名单增量（POST = PUT、`POST .../overlay/delete` = DELETE 别名，供 Kuikly 客户端使用） |
| `GET/POST /api/config/presets`、`POST .../{name}/apply`、`DELETE/POST .../{name}/delete` | 预设：列出 / 快照当前 / 应用 / 删除 |
| `POST /api/test/{llm,tts,vad}` | 内联连通性与首包延迟测试（按磁盘最新配置 ad-hoc 建引擎，无需麦克风，Web 可用） |
| `GET /api/memory/status`、`POST /api/memory/{recall,maintain,clear,rag/reindex}` | 记忆状态 / 手动试召回 / 立即维护 / 清空 / RAG 索引重建 |
| `GET /api/soul/presets`、`POST /api/soul/preview` | 人格预设清单 / 最终提示词预览 |
| `POST /api/devices/{id}/inject` | 设备消息注入（文本作为用户话语走完整管线） |
| `GET /api/metrics`、`GET /api/usage` | 延迟大盘 / 令牌用量（内存态） |

```bash
# 构建 web 管理页（产物含 index.html 与 nativevue2.js，由 [server].admin_dir 指向）
cd client && ./gradlew :apps:h5App:publishWeb

# 启动 server（admin_dir 默认 ../client/apps/h5App/web，按实际构建输出调整）
cd server && ~/.cargo/bin/cargo run -- --config config.toml
# 浏览器访问 http://127.0.0.1:8000/（管理台）
```

---

## 6. OTA 固件托管（多型号 / 多功能）

设备开机 POST `/api/ota` 检查版本，服务端按设备上报的 **功能 + 型号** 匹配托管固件，命中且版本更新才下发下载地址（设备自升级）。

- **寻址维度**：`功能(project) × 型号(model)`，可选 `板型(board)` 收窄。功能 = `esp_app_desc_t.project_name`，型号 = 芯片（`esp32c3`/`esp32s3`…），板型 = `board.type`。
- **匹配规则**：功能与型号**必须完全相同**；同型号优先板型精确匹配，否则回落"该型号通用"。**不同型号/功能的包绝不互发**；没有匹配就不下发。
- **元数据自动识别**：ESP-IDF 镜像自带元数据，上传时自动解析 型号/功能/版本/IDF 版本/编译时间，无需手填。
- **存储布局**：`<XIAOZHI_FIRMWARE_DIR>/<project>/<model>/<board|_>/firmware.bin + meta.json`（默认 `/data/firmware`）。同一 key 上传即覆盖，不留历史版本。

| 端点 | 语义 |
|---|---|
| `POST /api/ota/firmware` | 上传 app bin（`expected_token` 非空需 Bearer；可选 `?project=&model=&board=&version=` 覆盖） |
| `GET /api/ota/firmware` | 托管清单（管理页数据源） |
| `GET /api/ota/firmware/latest?project=&model=&board=` | 设备下载（**不鉴权**：设备无法带 Authorization 头） |
| `POST /api/ota/firmware/delete` / `DELETE /api/ota/firmware` | 删除条目 |

> 旧版单目录固件（`firmware.bin` + `latest.json`）会被列为 `_legacy`：可删可看，但**不参与匹配**——请按 功能+型号 重新上传。

---

## 7. ASR/TTS 测试台（macOS App）

`web` 受浏览器麦克风 / WebSocket 二进制等限制，不适合语音链路测试。macOS App（`apps/macosApp`，Mac Catalyst，复用 iOS 渲染器 `OpenKuiklyIOSRender`）承载端到端语音测试：

- **测试代码只在 `client/shared/src/macosArm64Main`**：web/Android/iOS/OHOS 的包不含测试代码；web 管理台「测试台」入口显示不可用说明与替代路径。
- **四个 tab**：连接 / ASR 识别（录音试听 → 识别解耦 + 波形回放）/ TTS 合成（音色下拉 + 语速，可临时覆盖当前配置）/ LLM 对话；每个 tab 右侧内嵌对应配置卡，配合顶栏「保存配置」写回。
- 原生桥接 `apps/macosApp/XiaoZhiModule*.m`：WS 建连、麦克风采集、音频播放、**Opus 编解码**（`XiaoZhiModule+Opus.m`，libopus）均已实现。

```bash
# ① 编出 KMP 业务框架（仅 Apple Silicon）
cd client && ./gradlew :shared:linkReleaseFrameworkMacosArm64
#    产物：client/shared/build/bin/macosArm64/releaseFramework/shared.framework
#    （⚠️ 改动任何 @Page 或 shared 业务代码后必须重跑本任务，否则 Xcode 跑旧框架）

# ② 生成 Xcode 工程（需 brew install xcodegen）
cd client/apps/macosApp && xcodegen generate

# ③ 安装渲染器依赖（需 CocoaPods）
pod install                                        # 生成 macosApp.xcworkspace

# ④ Xcode 打开 workspace，运行目标选「My Mac (Mac Catalyst)」
open macosApp.xcworkspace
```

> 启动顺序不可省（① 在 ② 之前，否则链接报 `shared.framework not found`）。Xcode 重跑后若白屏（页面回退占位），先 Clean Build Folder——那是 Kuikly 找不到 page 的 fallback，不是代码 bug。

---

## 8. 测试与回归

### 单元自测（Rust）

协议/音频/配置/插件/记忆/overlay/preset 各层均含 `#[cfg(test)]` 用例（二进制版本往返、密钥护栏与部分更新语义、overlay 白名单拒止、预设落盘往返、registry 护栏、VAD/ASR 桩等，全量 468+ 例）：

```bash
cd server && ~/.cargo/bin/cargo test
```

### 协议联调（Python 测试台客户端）

`tests/mock_client.py` 是**零第三方依赖**的 WebSocket 客户端（纯标准库实现 RFC 6455 帧），场景化回归：握手协商、`asr_test`/`tts_test`/`llm_test`、`abort` 打断后连接存活探针（fatal）、唤醒问候（容忍型）、多轮对话：

```bash
cd server && cargo run --release --features sherpa -- --config config.toml &
python3 server/tests/mock_client.py        # 逐场景 PASS/WARNING，末尾汇总
```

可用参数：`--host 127.0.0.1 --port 8000 --token <bearer>`（`expected_token` 非空时传 `--token`）。

---

## 9. Docker 部署细节

### 模型自动下载

容器入口（`docker-entrypoint.sh`）启动时检测关键模型文件（`silero_vad.onnx`、`SenseVoiceSmall/tokens.txt`、`Kokoro/model.int8.onnx`）：

- **缺失则自动下载**到挂载的 `/data/models`（从官方 GitHub Release 拉整包 tar.bz2 本地解压，幂等 + 断点续传）。
- 默认直连原始地址；compose 配置 `GITHUB_PROXY` 才走代理。下载地址可用 `SENSEVOICE_URL` / `KOKORO_URL` / `SILERO_VAD_URL` 覆盖为完整直链（内网镜像）。
- 行为开关 `XIAOZHI_AUTO_DOWNLOAD_MODELS`：`missing`（默认）/ `force` / `off`。

> 不手动预置模型也能直接 `docker compose up`。离线/内网环境：先 `SENSEVOICE_URL=<内网镜像> ./server/scripts/download_models.sh /host/data/models` 预置再挂载，或设 `XIAOZHI_AUTO_DOWNLOAD_MODELS=off`。

### 部署机指令集

构建期固定 `RUSTFLAGS="-C target-cpu=x86-64-v2"`（见 `Dockerfile`），兼容 N5105（Tremont，无 AVX）。**不要用 `native`**——会在构建机嵌入部署机不支持的 AVX 指令，运行时 `SIGILL` 崩溃。

### 真实引擎（`sherpa` feature）本地依赖

1. 系统 `libopus`；2. 网络（构建期原生库下载）；3. 本地模型文件：

```bash
./server/scripts/download_models.sh /host/data/models     # 裸机部署时手动下载
cd server && ~/.cargo/bin/cargo run --release --features sherpa -- --config config.toml
```
