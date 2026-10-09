# agents.md — 给 AI 编码助手的项目指引

> 本文件供 CodeBuddy / Claude Code / Cursor / Codex 等 AI 编码助手在本仓库工作时阅读。
> 它聚焦**本仓库特有的环境坑与协议约束**，通用 Rust 知识不在此赘述。

> 📌 模块级职责与复杂逻辑以 `server/src/*.rs` 顶部的 `//!` 模块注释为**权威说明**（配置加载优先级、`/api/config` 读写语义、引擎线程预算、协议二进制封装、四大引擎的 `spawn_blocking` 隔离等均已下沉到源码）。本文件聚焦 AI 易踩的**陷阱速查**；陷阱涉及的具体实现以源码 `//!` 注释为准，下文各条已加交叉链接。

## 1. 项目是什么

`xiaozhi-server-rust` 是一个用 Rust 实现的 **小智（xiaozhi-esp32）语音终端服务端**：

- 本地 ASR：SenseVoice INT8（离线，`sherpa-onnx`）
- 本地 TTS：Kokoro INT8（离线，`sherpa-onnx`）
- VAD：Silero VAD
- 远程 LLM：OpenAI 兼容的 **Responses** HTTP 接口
- 灵魂（`[soul]`）：结构化人格档案 → 有序 prompt 段（内置**默认灵魂**预设「小智」，留空即用、填了就覆盖；关闭时逐字节退回 `[llm].system_prompt`）
- 记忆（`[memory]`）：本地图记忆（SQLite：原文事实源 + 摘要 + 倒排索引；关键路径召回 + 后台抽取）
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
| `rusqlite` | 0.40（`bundled`） | 图记忆库（SQLite 源码内置编译，**无需**系统 sqlite；首次构建多约 1 分钟） |
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
# 编译检查（无 feature，不下载原生库；运行必须 --features sherpa）
~/.cargo/bin/cargo build \
  --config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'

# 类型检查 sherpa feature 代码（DOCS_RS=1 跳过原生库下载，见「验证不下载模型」）
DOCS_RS=1 ~/.cargo/bin/cargo check --features sherpa \
  --config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'

# 单元测试
~/.cargo/bin/cargo test \
  --config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'

# 真实引擎（需联网下载 sherpa 原生库 + 系统 libopus + 模型文件）
cd server && ~/.cargo/bin/cargo run --features sherpa -- --config config.toml
```

## 4. Feature 门控（重要）

`Cargo.toml` 中：

- `default = []`：无真实引擎，仅供 `cargo check/test` 编译（运行会报错）。
- `sherpa = ["dep:sherpa-onnx", "dep:audiopus", "dep:rubato"]`：真实引擎（**无 mock**：ASR=SenseVoice、TTS=Kokoro、LLM=HTTP、VAD=Silero）。

> TTS 用 `[tts].engine` 选实现（`kokoro`=本地 Kokoro / `xfyun`=科大讯飞在线，见 `[tts.xfyun]`；旧键 `backend` 与本地旧值 `sherpa` 仍可读），改后保存即热切换；**LLM（`[llm].model`/`api_key`/`api_base`…）与记忆（`[memory].*`）、人格（`[soul].*`）同样保存即对新会话生效**（`Engines::refresh_from_disk` 在会话开始按签名重建引擎，见 `engine.rs`）；ASR 与 `[server]`/`[audio]`/`[aiui]` 仍需重启。热生效语义的**唯一权威**是 `plugins/registry/` 每个描述符/字段的 `hot`（`live`/`next_session`/`restart_only`，`GET /api/config/schema` 直接暴露）。默认（无配置文件）模型路径即 `/data/models/...` 生产值；`[llm]` 需填 `api_base`/`api_key`。ESP 接入走完整正式流水线；macApp 测试台 hello 带 `test:true`，走 `asr_test/tts_test/llm_test` 三个独立服务端点（非测试会话发送这三类消息会被忽略）。

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

> 这一 bug **`cargo check` 完全无法发现**，只有真实 WebSocket 客户端（`tests/mock_client.py`）才能暴露。任何修改协议枚举的 PR 都必须重跑协议联调。

> 协议枚举定义与 serde 约束的权威说明见 `server/src/app/protocol.rs` 模块注释。

### 5.2 LLM 恒为真实 HTTP（无 mock）

LLM 只有 OpenAI 兼容 Responses API 一条路径；联调失败先检查 `[llm].api_base`/`api_key`/`model` 配置（服务端启动即构建引擎，模型/配置缺失会给出明确报错）。

### 5.2b 讯飞在线 TTS（`[tts].engine="xfyun"`）要点

- 协议 = **AIUI 主动合成 API** `wss://aiui.xf-yun.com/v3/aiint/sos`
  （`scene="IFLYTEK.tts"`、`interact_mode="oneshot"`、`header.status=payload.text.status=3` 文本一帧发完；
  鉴权同官方 doc-404：HMAC-SHA256 URL 签名；响应 `payload.tts.audio` base64 PCM 分帧，`header.status==2` 收尾）。
  相比旧 `tts-api.xfyun.cn/v2/tts`：**支持的音色更广**（x4 超拟人在 v2/tts 报 11200、AIUI 链路可用）。
  实现见 `plugins/tts/aiui.rs`；`plugins/tts/xfyun.rs` 现仅存凭据结构 + 签名/错误提示助手。
- **音色目录 = 真实 API 探测**（`app/voices.rs`）：讯飞无"列举发音人"的官方 API，服务端对候选池
  （`CANDIDATES`，148 个命名空间条目）逐项调用合成 API 验证，**通过的才进目录**（缓存 /data/voices.json）。
  ⚠️ 平台有**并发限流**（实测 6 并发大量假阴性 11200/10163，串行/低并发恢复）→ 探测并发固定 2 + 失败重试一次；
  全量探测约 1 分钟（macApp「探测音色目录」触发 → 轮询 `probing` 字段 → 完成后重拉）。
- **测试台/流水线共用流式**：服务端合成过程中边收 base64 PCM 边下发（首片即出声）；
  重采样器与 Opus 编码器**跨分片复用**（各自独立会在边界产生相位跳变/爆音）。
- 合成失败（构建失败/凭据错误/网络）经 `tts_test` 结果帧回报测试台（`{type:"tts_test", state, engine, text}`），
  不再静默——卡片会显示「合成失败：原因」；测试台所选 vcn 随 `tts_test.vcn` 下发（本次合成即用，无需先保存）。
- 密钥只在服务端配置里；管理页保存后无需重启（热切换：会话开始读盘比对签名重建 TTS/LLM 引擎）。
  **密钥打码已上线**（2026-10，见 `docs/plugin-architecture-unification.md` §0.5）：`GET /api/config`
  **永不回传明文**，只回 `has_api_key` / `has_api_secret` presence 标记；语义 = 表单留空不修改
  （客户端 `putSecret` 干脆不发该字段）、显式 `null` 清除、非空覆盖。⚠️ **新增密钥字段必须三处同改**：
  服务端 `SECRET_PATHS`（`app/ws/config.rs`，打码/空值语义/presence 共用该表）、客户端 `fill()` 读 presence、
  `buildConfigJson()` 用 `putSecret`——漏一处就会出现「保存一次把密钥清空」或「明文回传」。
- **本地/远程分层（UI 与配置约定）**：`[tts]` 本体只留**跨实现通用项**（`engine`/`speed`/`speaker`/`cache_entries`）；
  实现私有参数进 `[tts.<id>]` 子段（本地 Kokoro 也进：`[tts.kokoro]`；远程凭据 `[tts.xfyun]`）。
  管理页 TTS 卡为两级下拉（合成方式 → 引擎/服务商）。
  新增供应商 = `plugins/tts/<id>.rs` 实现 + `[tts.<id>]` 结构 + `plugins/registry/impls.rs` **描述符加 1 条**
  （`fields` 放 `registry/fields.rs`）。**不再需要**：枚举变体 / `backend_kind()` / 中心化 `match` 分发
  （P4 已全部删除：选择走描述符 `selected`，构建走描述符 `build`，签名走描述符 `signature`）。
  客户端仍需 `ConfigFormState.ttsRemoteEngines` 追加 + 卡片字段区（P5 目标：字段元数据驱动后只剩 1~2 处）。
- **P4 配置约定（全能力统一，见 `docs/plugin-architecture-unification.md` §0.10）**：
  ① `engine = "<实现 id>"` 选实现，id = 注册表描述符 id 去掉 `"<cap>."` 前缀（`[tts] engine="kokoro"` ↔ `tts.kokoro`）；
  ② 私有字段进 `[<cap>.<id>]`；③ 跨实现参数（被 `Trait::method` 当入参用的，如 TTS 的 `speed`/`speaker`）留本体。
  - **旧写法必须继续可读**：`[tts] backend="sherpa"`（别名 → `kokoro`）、`[tts].model` 等旧扁平字段、
    `[memory].backend`。由 `Config::normalize()`（配置的唯一入口：`load`/`default`/`merge_client_patch`）
    搬到规范位置；`app/ws/config.rs::rewrite_legacy_paths` 负责**旧客户端补丁**的重写
    （否则旧 macApp 改 model 会被"新位置优先"静默丢弃）。保存只写新结构（旧字段 `skip_serializing`）。
  - **未知 engine 值绝不静默回退默认**：`PluginHost::plan` 标 `Failed` + 可选值提示，`boot`/`decide_refresh` 直接报错
    （`registry::unknown_engine_hint`）。`[memory].engine="none"` 是合法显式停用，不算未知。
  - 新增/改名实现时必改的**只有两处**：`plugins/*/mod.rs` 的 `<CAP>_ENGINES` 表（含别名）与注册表描述符；
    `config/tests.rs::declared_engine_ids_match_registry_implementations` 会守住二者一致。

### 5.2c 🚨 下行音频必须按实时节奏节流（`DownlinkPacer`，勿删）

**症状**：ESP 短回复正常，长回复「先听到头几个字 → 卡住 → 断断续续」；而 macApp（无队列限制）
反而流畅——勿因此误判"服务端没问题"。

**根因链**（三方证据）：
1. ESP 固件 `AudioService` 解码队列上限 `MAX_DECODE_PACKETS_IN_QUEUE = 1200/OPUS_FRAME_DURATION_MS`
   = **20 包（≈1.2 秒音频）**；队满时 `PushPacketToDecodeQueue(wait=false)` **直接丢弃、零日志**
   （`main/audio/audio_service.h` + `audio_service.cc`）。
2. TTS 合成远快于实时（实测讯飞在线 **~7 倍速**、整段一次性回传时更快）——若尽速灌入，
   长回复必然溢出：设备先播完 1.2s 缓冲，随后队列在阈值上下震荡 → 断续。
3. 官方 Python 服务端有专门的 `core/utils/audioRateController.py`（`AudioRateController`，
   注释"按照60ms帧时长精确控制音频发送"）——**节流是协议层的隐含要求，不写在 hello 协商里**。

**How to apply**：
- 实现于 `app/downlink.rs` 的 [`DownlinkPacer`]：每帧目标时刻 = 锚点 + N×帧长（**绝对时间表**，
  无累积漂移）；生产者慢（本地 Kokoro RTF>1）时自然零等待；数据源真空 ≥1 帧后重锚定
  （播放已追平，不突发回补）。
- 生命周期 = **一次回复**（`stream_response` / `aiui_downlink` 入口 `reset()`）；多句共享时间表
  → 句间无缝。**切勿**在句间 reset（每句重新锚定会把句间隙暴露成听感停顿）。
- ⚠️ 改下行链路时**不得移除 `pacer.pace()` 调用**——它看起来像"多余延迟"，实为 ESP 队列保护；
  删掉后 macApp 依旧流畅、ESP 长回复必现断续（自动化测试也很难在没有真机时发现）。
- 测试台路径（`session/bench.rs`）同样节流（单次合成即一整段，更快、更容易溢出，更需要）。
- TTFA 日志 = **首帧实际下发时刻**（`first_frame_at`，非"句子下发完成"）——pacing 后两者含义不同。

### 5.2d 二进制帧 v2/v3 是**网络序（大端）** + 下行版本必须跟随设备（2026-10-07 实测修正）

两个曾同时存在的对齐缺口（设备默认 v1 裸包时都无感，切 v2/v3 必坏）：

1. **字节序**：固件 `websocket_protocol.cc` 收发 v2/v3 用 `htons/htonl/ntohs/ntohl`
   （i.e. **网络序大端**），结构体 `__attribute__((packed))` 与原字节流同布局。
   服务端曾用 `to_le_bytes()`（小端）——`payload_size` 被设备读成天量值 → 解码失败/无声。
   现已全部改 `to_be_bytes`（`app/protocol.rs`，回归测试 `bin_v2_v3_big_endian_layout` 逐字节锁定）。
   ⚠️ 官方 Python 服务端**只实现 v1**（仓库内无 v2/v3 打包），字节序只能以固件实现为准。
2. **下行版本跟随设备**：设备按其 NVS `websocket.version` 同时决定收/发帧格式；
   服务端下行若用配置里的另一个版本，设备会按自己的版本解析 → 乱码。
   现 `ws/mod.rs` 握手时 `downlink_bin_ver = uplink_bin_ver`（hello.version）。
   `[audio].binary_protocol_version` 的作用改为**仅经 OTA 写进设备 NVS**
   （`ota_payload` 的 `websocket.version`），二者因此天然一致。

### 5.2e 灵魂 / 记忆（`[soul]` / `[memory]`）接入要点（2026-10 新增）

- **默认全关**：`[soul].enabled=false` 时 `instructions` = `[llm].system_prompt` **原文**
  （逐字节一致，有回归单测）；`[memory].enabled=false` 时装配为 `NoopMemory`（不建库、不读盘、不调模型）。
  改动这两条路径时**必须**保持该语义，否则"关掉就回到今天"这个回滚承诺失效。
- **内置默认灵魂（`[soul].preset`）**：人格不必先填 20 个字段——`preset` 默认 `"xiaozhi"`
  （内置「小智」人格，见 `plugins/soul/presets.rs`）。合并规则见 `SoulConfig::effective`：
  **留空的字段**（空串/空数组）由预设补，**填了的字段**逐字段覆盖它；`preset = "none"` = 不用预设。
  要点：
  - `preset` **不是** P4 的 `engine`：`engine` 选实现（闭集），`preset` 是可覆盖的**内容基线**；
  - 未知 preset 值**绝不静默回退**（报错 + 列出可选值），但 `enabled=false` 时完全豁免（可回滚）；
  - 预设只提供**文本/列表**字段（`presets::provided_fields`）：布尔与数值恒有具体值、不存在"留空"，
    一律以用户配置为准——**别给预设加 `colloquial`/`emoji`/`max_*` 的期望**；
  - `enabled_soul` 校验跑在 **effective 档案**上：空 `name`/`address_user` 只有在 `preset="none"`
    或预设未提供该字段时才是错误；
  - 客户端「载入预设内容到表单」靠 `POST /api/soul/preview` 回传的 `effective`（**完整档案**）
    整段喂 `fill()`——因此 `effective` 少一个数组键就会把用户原文冲成空（有回归测试钉住）；
    客户端**不复制**合并规则，合并语义只有服务端一份；
  - 新增预设：`presets.rs` 加 `fn <id>() -> SoulConfig` + `PRESETS` 加一条，并同步客户端
    `ContextSectionState.soulPresetOptions`（否则只能在 TOML 里手写、UI 选不到）。
- **人格必须可观测**：`GET /api/soul/presets`（清单 + 可直接喂表单的 `profile`）与
  `POST /api/soul/preview`（最终 `instructions` + 分段 + `from_preset`/`overridden` 字段来源，
  并作为**保存前校验器**）。没有这两个端点，用户改完人格无法确认"到底发出去了什么"，
  只会得到"配了没用/不敢改"的结论（与记忆的「测试召回」同一个道理）。
- **热生效**：两者都是 `next_session`（会话开始时 `Engines::refresh_from_disk` 按注册表签名重建记忆引擎；
  人格每轮按 `live_config()` 组装）。改完保存 → **新会话生效**（与 TTS/LLM 一致）。
- **`LlmProvider::chat_stream` 签名含 `&TurnPrompt`**：`TurnPrompt.instructions` 是稳定前缀
  （平台约束 + 人格 + `[llm].system_prompt`），`memory` 块按 `inject_position` 插进 `input[]`
  （默认 `after_history`：保住 `instructions + history` 的前缀缓存）。**新增 LLM provider 必须实现该入参**。
- **记忆库路径必须在挂载卷内**（默认 `/data/memory.db`）：容器重建会丢全部记忆；
  `docker-compose.yml` 已挂 `./server-data:/data`，但把 db 放到容器层里就会静默丢。
- **关键路径纪律**：`recall` 有 `recall_budget_ms`（默认 300ms）硬预算，超时/失败一律**静默降级为空**、
  绝不返回 `Err`；`record` 只落原文（快）；抽取（1 次辅助 LLM 调用）**只在后台任务**里跑，
  失败进隔离表 + 有限重试，**永不阻塞对话**。改这条链路时不要把抽取挪回关键路径。
- **密钥三处同改**：`[memory.extractor].api_key` 已加入 `SECRET_PATHS`（`app/ws/config.rs`），
  客户端 `fill()` 读 `has_api_key` presence、`save()` 用 `putSecret`（留空不发送）。
- **AIUI 全链路下人格/记忆不生效**：云端闭环绕过本地 LLM（`aiui_downlink` 既不 recall 也不 record）。
  管理页记忆卡与 `GET /api/memory/status` 都带显式提示——否则用户会报"配了没用"。
- **中文词法召回是自建倒排索引**（汉字二元组 + ASCII 单词），**不是** SQLite FTS5：
  `unicode61` 把整个中文串当一个 token、`trigram` 要求 ≥3 字且换个说法就失配。
  改召回时不要"顺手换成 FTS5"，会直接丢掉中文召回能力。
- **保留策略不得删被引用的原文**：候选查询必须带
  `id NOT IN (SELECT message_id FROM gm_turn_memory_sources)`——上游 graph-memory 正是在这里
  误删了轮次记忆引用的证据（`docs/soul-and-graph-memory-plan.md` §2.4 缺陷 1），我们已修且有回归测试。

### 5.2f 语音指令闸门（`[command]`）接入要点（2026-10 新增）

说「退下 / 闭嘴 / 关闭」这类**不是提问**的话，送进 LLM 只会换来一段没用的回复（还计费、还合成）；
正确处置是结束这次会话。实现落在 ASR → LLM **之间**：

- **位置**：级联链路在 `session.rs::recognize_segments` 里 `handle_command_gate(&user_text)`，
  命中即 `return`（本批剩余语音段一并丢弃，**一次 LLM 都不调用**）；AIUI 全链路在
  `aiui_segments` 里对回来的 `turn.stt` 同样判定——识别在云端闭环内完成，只能在轮次回来后
  "丢弃本轮云端回复"，但会话照样按指令断开（两种链路语义一致，见 `session/command.rs`）。
- **断开方式 = 主循环收尾**：命中置位 `close_requested` → `run()` 跳出循环 → **跳过 VAD flush**
  （用户已明确要求退下，不该再为残留音频开一轮对话）→ 等一段排空时间（`downlink_lead_ms`，
  夹在 100ms~1s；告别语是按实时节奏下发的，最后一帧发出时设备队列里还有约一个 lead 的音频，
  立即 close 会把告别语尾巴掐掉）→ 调 `Transport::close()`（WS 发关闭帧并让 `closed` 粘滞）
  → `run_session` 返回、`WsTransport` 释放。**不要**在分支里直接 drop socket/break：
  那会绕过告别语、日志与节流收尾。
- **⚠️ 命中后必须先 `abort.store(false)`**：`abort` 只在 `stream_response` 入口重置，而指令闸门
  刻意不走那条路；上一轮若以设备打断结束（abort 仍为 true），告别语会被逐帧打断轮询立刻掐断，
  表现为"没告别就断线"（`session/command.rs` 有注释，勿删）。
- **默认 `exact` 是刻意的**：识别文本先归一化（只留字母数字/汉字 → 剥掉句末语气助词「吧/啊/了…」）
  再整句比较，「退下吧。」≡「退下」；宽松的 `contains` 会让「关闭闹钟」「把灯关闭」直接断线——
  那比"没识别到指令"严重得多，只在确认场景后显式开启。
- **长度闸门（`max_chars`，默认 5）是第二道独立保险**：归一化后超过 5 个字的识别文本**不做指令判断**、
  直接放行给 LLM。理由是"长句携带信息"——「帮我关闭卧室的灯」「退下之后帮我放首歌」都含指令词，
  但它们是正常请求/追问，断线是灾难；真正的指令只有两三个字。两道闸的关系：长度挡"长句误伤"、
  `exact` 挡"短句误伤"（关闭闹钟），缺一不可。计数口径与匹配**一致**（归一化之后，标点/空白/句末
  语气词不计入），所以「退下！！！！」仍是 2 个字、照常命中；`max_chars = 0` = 不限制（拆掉保险）。
  ⚠️ 与词表的交互坑：指令词本身长于 `max_chars` 时永远命中不了（静默失效），
  `CommandConfig::validate` 会拦下并说明怎么改——加/改词表时别把这条校验删了。
- **配置与语义**：`[command]`（`enabled` 默认 false / `keywords` 默认 退下·闭嘴·关闭 / `match_mode` /
  `max_chars` 默认 5 / `reply`），注册表声明为 `next_session` + **非必需能力**（"启用但词表为空"只降级上报，绝不阻止启动）；
  词表为空时闸门恒不拦截（若按"空 = 命中一切"处理，用户清空一次就会断掉所有会话）。
  默认关闭时零开销、行为与加它之前完全一致——这是可回滚承诺，勿破。
- **改配置段必须同步注册表字段元数据**（`registry/fields_command.rs`），否则
  `config/tests/impact.rs::every_config_section_has_declared_hot_semantics` 打红、界面永远显示"需重启"。

### 5.3 二进制协议版本 = 设备 hello 的 `version`

- `version` 字段 = **二进制协议版本（1/2/3）**，不是握手协议号。
- 服务器**下行**二进制帧必须使用与设备相同的版本（`wrap_downlink`）。
- 上行按版本剥离头部（`unwrap_uplink`）。v1 裸 Opus；v2 16 字节头；v3 4 字节头。
- 建议先用 v1 真机验证，再切 v2/v3。

> 二进制帧字节布局（v1/v2/v3）的权威表见 `server/src/app/protocol.rs` 模块注释。

### 5.4 服务器 hello 的 `audio_params` = 下行解码参数

设备读取服务器 hello 的 `audio_params.sample_rate` / `frame_duration` 来解码下行 TTS 音频。上行仍按设备自己的 16k。下行采样率由 `audio.downlink_sample_rate`（默认 24000）决定。

> 上行/下行协商逻辑见 `server/src/app/ws.rs` 的 `handle_handshake`。

### 5.5 构建镜像的 Rust 版本必须与生成 `Cargo.lock` 的 cargo 对齐（当前 1.90）

`Cargo.lock` 由本机 cargo **1.90** 生成，锁定的传递依赖需要较新 Rust：`idna_adapter`（`reqwest → url → idna`）要求 `edition2024`（Cargo ≥ 1.85 才能解析）；`time` / `icu_*` 要求 `rustc 1.88`。`Dockerfile` 构建阶段用 **`rust:1.90-bookworm`**（与本机 cargo 同版本，保证 lock 一定可编译）。构建镜像版本约束见根 `Dockerfile`。

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

### 5.6b 构建产物与「容器零日志 exit 0」陷阱

Dockerfile 依赖缓存层用 dummy `fn main(){}` 先编译全部依赖（含 sherpa 原生库下载）；
COPY 真实源码后**必须** `touch src` + `cargo clean -p xiaozhi-server-rust --release` 强制重编本 crate——
否则 mtime 陷阱（COPY 保留的上下文 mtime 早于容器内编译时刻）会让 cargo 判定"没改过"而静默跳过，
镜像里留的是占位二进制，容器启动即退出且零日志。**不要用 grep 在二进制里找哨兵串做产物断言**：
断言依赖构建机 grep 实现对二进制的匹配行为（GNU/ugrep 别名差异），ACR 上连续两次假阴性
（2026-10 连同版本戳特性一并移除）；「Compiling + 非 0.00s 耗时」日志即重编证据。
配置文件不可写时 entrypoint 启动即打 ⚠️（保存 500 的根因多为挂载属主/权限）。

### 5.7 容器启动会自动检测并下载缺失模型（`docker-entrypoint.sh`）

报错 `tokens.txt does not exist` / `创建 SenseVoice 识别器失败` 的根因是 `/data/models` 里没有模型文件。Docker 入口脚本在 `exec` 服务器**之前**会自检关键文件（`silero_vad.onnx`、`SenseVoiceSmall/model.int8.onnx`、`Kokoro/model.int8.onnx` 等，**官方包内文件名是 `model.int8.onnx`，不是 `model.onnx`**）：

- **缺失 → 自动从 k2-fsa/sherpa-onnx 官方 GitHub Release 下载整包 tar.bz2 并解压**到挂载的 `/data/models`（默认行为），下载后持久化，后续启动检测到即跳过（按模型粒度幂等 + .part 断点续传）。
- 默认**直连原始地址**；docker compose 配置了 `GITHUB_PROXY` 才走代理（如 `https://tvv.tw/`）。`SENSEVOICE_URL` / `KOKORO_URL` / `SILERO_VAD_URL` 可覆盖为完整直链（内网镜像，不会被二次套代理）。
- curl 带 `--connect-timeout 15 --retry 3`（避免连接假死 134s）；下载/解压失败会让入口**中止启动**（未设 `XIAOZHI_ALLOW_MISSING_MODELS` 时），避免带着缺模型崩溃重启循环。
- 行为开关 `XIAOZHI_AUTO_DOWNLOAD_MODELS`：`missing`（默认）/`force`（每次重下）/`off`（不下载）。
- 运行期镜像需装 `curl` + `bzip2`（`tar` 自带）用于下载与解包；`docker-compose.yml` 的 `./server-data` 挂载**必须可写**（不要 `:ro`）。
- 离线/内网：先 `./server/scripts/download_models.sh /host/data/models` 预置再挂载，或设 `XIAOZHI_AUTO_DOWNLOAD_MODELS=off`。

默认模型包（已完整下载验证，与 config.example.toml 路径一一对应）：`sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2025-09-09`（SenseVoice INT8，`model.int8.onnx`）、`kokoro-int8-multi-lang-v1_1`（Kokoro INT8 **中英双语**，含 model.int8.onnx/voices.bin/tokens.txt/espeak-ng-data/lexicon-zh.txt/lexicon-us-en.txt/dict（jieba）/date-zh.fst）、`silero_vad.onnx`（Silero VAD）。⚠️ 此前误判 `kokoro-int8-multi-lang-v1_1` 不是完整包（流式列清单被管道截断所致），实际 144MB/417 文件完整；`kokoro-int8-en-v0_19` 仅英文、无 lexicon，中文场景勿用。

### 5.10 文件规模：**逻辑文件 ≤ 600 行**（硬约定，2026-10 起）

- **任何 `.rs` / `.kt` 逻辑文件不得超过 600 行**。接近上限时按职责拆分子模块，不要"再挤一点"。
- **测试一律独立文件（无例外，2026-10 更新）**：任何 `#[cfg(test)] mod tests` 都必须外移到同级 `<name>/tests.rs`
  （`foo.rs` → `foo/tests.rs`；`foo/mod.rs` → `foo/tests.rs`），外层只保留两行声明：

  ```rust
  #[cfg(test)]
  mod tests;
  ```

  即使只有 1 个断言也不内联；测试文件首行用 `//!` 写明来源。**空的 `mod tests {}` 直接删除**，不要留占位。
  子模块对父模块私有项的可见性与内联时**完全一致**（`use super::*;` 照旧），因此外移**不需要**放宽任何可见性。
- 校验（两条都应无输出）：
  ```bash
  grep -rnE 'mod[[:space:]]+tests[[:space:]]*\{' server/src --include=*.rs   # 内联测试块
  # ⚠️ 必须排除 wc 的 total 行，否则它的常量总数（万级）会被误判为"超 600 的文件"
  find server/src -name '*.rs' -exec wc -l {} + | awk '$1>600 && $2!="total"'
  ```
- Rust 拆分布局：`foo.rs` + `foo/` 子目录（`foo.rs` 内 `mod bar;` → `foo/bar.rs`）。**子模块可访问父模块私有项**（字段、私有 fn 都行），所以大多数拆分**不需要**放宽可见性。**唯一例外**：被**父模块调用**的子模块方法必须标 `pub(super)`（父模块看不到子模块的私有项）——用 `pub(super)` 而不是 `pub`，若被迫写成 `pub` 说明拆错了边界。
- 拆分只允许**行为不变的搬移**：搬完必须 `cargo test`（默认 + `--features sherpa`）或 `./gradlew :apps:h5App:publishWeb` 通过。
- 现状（2026-10）：`server/src` 有 **33 个 `tests.rs`**、**0 个内联测试块**；client 侧暂无测试，新增请放 KMP `commonTest/`。
  子目录拆分的既有先例：`plugins/registry/{fields,fields_context,impls}.rs`、`plugins/memory/store/{maintenance,tests}.rs`、
  `app/ws/config/tests/{secrets}.rs`、`config/tests.rs`、`plugins/tts/tests.rs`（父模块只留 `mod`）。
  整改记录见 `docs/plugin-architecture-unification.md` §0.7~§0.10。

## 6. 项目结构速查（monorepo）

本仓库为 monorepo：`server/` 是 Rust 服务端，`client/` 是 Kuikly 多端工程（管理后台 web）。

```
server/                        # Rust 服务端
  src/
    main.rs       入口：加载配置 → 初始化引擎 → 启动 Axum
    config.rs     TOML 配置聚合（Server/Audio 段 + 插件段 re-export；加载/默认值/`normalize()` 约定）
    config/tests.rs  规范化与 engine 约定护栏（engine id ↔ 注册表一致、样例可解析）
    engine.rs     Engines 装配器：共享引擎池（ASR/TTS/LLM）+ 每会话 VAD 工厂 + TTS/LLM 热切换
    app/          应用层（与具体引擎无关）：
      ws/           网关与 HTTP 端点组：mod.rs（路由）/ config.rs（+ tests/secrets.rs）/ memory.rs（记忆端点）/ soul.rs（灵魂预设与预览端点，+ soul/tests.rs）/ plugins.rs / usage.rs / ota.rs
      ws.rs         WebSocket 网关：握手 / 协商 / 鉴权 + 静态托管管理页（/）+ OTA 端点
      session.rs    每连接会话状态机 + 语音流水线（支持 abort；AIUI 全链路分支）
      protocol.rs   消息枚举 + 二进制版本封装（v1/v2/v3）—— 改这里必看 §5.1
      session/reply.rs  回复生成与下发（LLM→分句→TTS→下行；含人格/记忆组装与后台写入）
      session/command.rs 指令闸门落点（ASR→LLM 之间：命中即说告别语 + 断开会话，见 §5.2f）
      transport.rs  承载抽象（WS/未来 MQTT+UDP；`close()` = 本端主动断开）；downlink.rs 下行音频；splitter/sse
      audio/        Opus 编解码 + 重采样；firmware.rs 固件托管；voices.rs 发音人目录
    plugins/      引擎插件层（按能力分目录）：
      registry/     能力/实现的可枚举清单（描述符：字段元数据 + 热生效 + 签名 + 校验 + 构建）
        mod.rs        类型定义 + 全量注册表 + 查找 API + JSON 投影
        fields.rs     字段元数据表（能力公共字段 + 各实现私有字段）
        fields_context.rs  soul/memory 的字段元数据表（与 fields.rs 合并投影）
        fields_command.rs  指令闸门（[command]）字段元数据表
        impls.rs      选择/签名/校验/构建 + 11 个描述符常量
        tests.rs      回归测试（id 唯一性、字段漂移护栏、schema 形状）
      host.rs       装配与校验前移、enabled/phase 分离的状态快照（/api/plugins 数据源）
      （新增厂商 = 加实现文件 + registry/impls.rs 加 1 条 + registry/fields.rs 加字段表）
      asr/{mod,sensevoice}.rs   trait + SenseVoice 实现（sherpa）
      tts/{mod,tests}.rs + {kokoro,xfyun}.rs  trait + Kokoro 本地 / 讯飞在线实现（`[tts.kokoro]`/`[tts.xfyun]`）
      llm/{mod,openai}.rs       LlmProvider trait + OpenAI Responses 实现（+ failure/retry 护栏）
      vad/{mod,silero}.rs       trait + Silero 实现（sherpa）
      aiui/mod.rs               FullChainEngine trait + AIUI 全链路（识别+大模型+合成云闭环）
      prompt/{mod,tests}.rs     有序 prompt 段注册表（稀疏 order + 启动期变量校验）
      soul/{mod,tests}.rs       [soul] 人格档案 → prompt 段（关闭时逐字节退回 system_prompt）
                                 + presets.rs（内置默认灵魂预设 + 基线字段投影，+ presets/tests.rs）
                                 + preview.rs（最终提示词预览：分段构成 + 字段来源 + 生效路径）
      memory/                   图记忆：mod.rs（配置/trait/NoopMemory）+ graph.rs（召回/抽取）
                                 + store.rs + store/maintenance.rs（SQLite 读写/保留策略）
                                 + schema.rs（DDL/迁移/稳定 ID）+ terms.rs（汉字二元组倒排索引）
                                 + assemble.rs（注入块渲染 + token 预算裁剪）+ llm_extract.rs（抽取契约）
      command/{mod,tests}.rs    语音指令闸门（[command]）：ASR→LLM 之间拦截指令词，
                                 长度闸门（max_chars，默认 5）+ 词表匹配，命中即由会话层
                                 说告别语并断开会话（默认关闭；见 §5.2f）
  config.toml / config.example.toml
  scripts/download_models.sh
  tests/mock_client.py         零依赖 WebSocket 联调客户端
client/                        # Kuikly 多端工程（管理后台 web + macOS 测试台）
  settings.gradle.kts         rootProject.name = "client"
  gradle.properties           kuiklyVersion 唯一来源（升级需与根 build.gradle.kts 的 buildscript 双写同步）
  build.gradle.kts            Kuikly 插件经 buildscript classpath 引入 + 插件版本锁定（含 KSP）
  kotlin-js-store/yarn.lock   Kotlin/JS npm 依赖锁（⚠️ 必须入库，缺失则 yarn 解析漂移）
  shared/
    commonMain/               公共代码：@Page("config") 管理后台（跨端）+ 配置表单 + NetworkModule
                              （P5 新文件：ConfigSections.kt 页签常量表 / PluginMetaState.kt 能力总览
                               / ConfigOverviewCard.kt 总览渲染 / FormSchemaState.kt 字段热生效提示）
                              （[command] 指令卡：ConfigCommandState.kt 表单状态 + ConfigCommandCard.kt 渲染）
    macosArm64Main/           仅 macOS(arm64) 编译：@Page("test") ASR/TTS 测试台 + XiaoZhiModule
    （js(IR) 目标在此模块：@Page 注册靠 KSP(core-ksp)，业务包由 Kuikly 插件打包）
  apps/
    h5App/                    Web(H5) 壳：渲染器(core-render-web:h5) + Main.kt；publishWeb 汇聚 web/
    web/                      产物契约：index.html(入库) + nativevue2.js(业务包) + h5App.js(壳)
    androidApp/ iosApp/ ohosApp/   各原生宿主
    macosApp/                 macOS 宿主（原生，Mac Catalyst）：ASR/TTS 测试台，复用 iOS 渲染器
```

> 单元测试一律在同级 `<name>/tests.rs`（外层只留 `#[cfg(test)] mod tests;`，见 §5.10）；`server/tests/mock_client.py` 是独立的协议联调客户端（非 Rust 单测）。
> server 在 `/` 静态托管 `client/apps/h5App/web`（`[server].admin_dir` 默认值即此；
> 容器内由镜像内置 `XIAOZHI_ADMIN_DIR=/app/web` 覆盖），并通过 `GET/POST /api/config`
> 让管理页读写 `config.toml`。本地产出管理页：`cd client && ./gradlew :apps:h5App:publishWeb`。

### 5.8 client（Kuikly）构建链要点与陷阱

- **web 双 bundle 架构**（对齐 xiaoya-player）：`shared` 持有 js 目标 → Kuikly 插件 `packLocalJSBundleRelease` 产业务包 `nativevue2.js`；`apps/h5App` 是壳（渲染器 + 入口）→ webpack 产 `h5App.js`；`web/index.html` 先载业务包、后载壳。**js 消费方无法解析无 js 目标的 KMP 模块**——不要把 js 目标从 shared 挪走。
- **Web 渲染器真实坐标是 `com.tencent.kuikly-open.core-render-web:h5`**（artifactId 为 `h5`）。`com.tencent.kuikly-open:core-render-web` 在腾讯镜像上 404（实测 Could not resolve）。
- **DSL 陷阱**：容器带 `@ScopeMarker`（@DslMarker）——body 的嵌套容器里不能隐式访问 Pager 成员；官方口径 `val ctx = this` + `ctx.xxx`。可复用 UI 片段写成**文件级** `private fun ViewContainer<*,*>.xxx()` 扩展（类内成员扩展实测编译失败）。持有 `ViewBuilder` 值在嵌套闭包内执行必须显式传接收者（`page.content(this)`），隐式 `page.content()` 报 `No value passed for parameter 'p1'`。
- **UI 必须用 Kuikly 官方组件与布局（用户多次强调，勿自造样式替代官方机制）**：写 UI 前先查 `.agents/skills/kuikly-*`（kuikly-ui-framework 内含官方文档/源码克隆 `references/KuiklyUI/`）与官方组件清单；编码规范见 `.codebuddy/rules/kuiklyDSL.mdc`。已整改：下拉=官方 AlertDialog、开关=官方 Switch、标签页=官方 **Tabs+PageList**（TabItem 官方结构 + `indicatorInTabItem` 官方指示条，不自画选中胶囊）、按钮=官方 compose Button、backend 选择=官方 AlertDialog 下拉（手搓 segmentedControl 已删——官方 SegmentedControlIOS 是 iOS 渲染器专属、web 无实现）、多行散文=官方 TextArea（`labeledTextArea`）。**PageList 必须显式设 `pageItemWidth`/`pageItemHeight`**（官方示例按导航/tab 高度手算；AdminShell/ConfigPage 各自算好传入 `tabbedPanel`），不设则 item 无尺寸约束整页塌成单行（实测）。Tabs↔PageList 联动 = PageList `scroll` 事件回传 ScrollParams 喂 `Tabs.scrollParams`、点击 tab `scrollToPageIndex(index)` 翻页；封装见 AdminTheme.kt `tabbedPanel`。
- **组件官方化审计结论（2026-10-05 全量）**：AdminTheme.kt 是唯一组件层，全部基于官方组件组合（Text/View flex/官方 Button/Input/TextArea/Switch/AlertDialog/Tabs/PageList/Scroller/ActivityIndicator）；无官方对应物的组合件仅剩：`statusBadge`（官方无徽标）、`waveformPlayer`（官方无音频波形/播放器 UI）、`sidebarItem`+`largeTitleBar`+`groupedCard`+`ToastHost`（官方无侧边栏/导航/卡片/Toast 组件，属 View+Text 官方布局组合）。新增 UI 一律先查官方清单，禁止绕过 AdminTheme 直接散写。
- **API 位置**：`Color`/`Border`/`BorderStyle` 在 `com.tencent.kuikly.core.base`（不在 base.attr）；**按钮用官方 compose Button**（`com.tencent.kuikly.core.views.compose.Button`：`titleAttr` 设文字/颜色且须写在 attr 块内、`highlightBackgroundColor` 按压高亮；文字居中由 ButtonView 内部 `justifyContentCenter+alignItemsCenter` 保证——手搓 View+Text 按钮（allCenter/textAlignCenter/flex）在 Catalyst 上居中实测不可靠，「连接/断开」多次偏侧即此因；官方 Button 无子节点插槽，loading 态用灰底+文案切换，不塞菊花）；无 `paddingHorizontal/Vertical`（用 padding(left/right/top/bottom)）；Input 文本变化事件是 `event { textDidChange { } }`。
- **Docker 构建内存**（ACR 构建机实测 OOM 表现：日志戛然而止 + rpc EOF）：web 阶段 Gradle/Kotlin daemon/webpack-Node 三处显式限堆（1280m/1024m/1024m，取 xiaoya 实测口径）+ `-PwebSourceMap=false` 关 source map；⛔ 不要调高。
- **settings.gradle.kts 勿设 PREFER_SETTINGS**（会丢 Kotlin/JS 插件自动加的 nodejs.org dist 仓库，org.nodejs:node 必然解析失败）；根工程/子模块勿声明项目级 repositories（多余仓库的 DNS 异常会中断整条解析链）。

### 5.9 macosApp（Mac Catalyst）构建与运行要点

- **只能用「My Mac (Mac Catalyst)」运行目标**：渲染器 OpenKuiklyIOSRender 是 UIKit 的；选原生 "My Mac" 必报 `'UIKit/UIKit.h' file not found`。防呆已内建：target 的 preBuildScripts 守卫检查 `EFFECTIVE_PLATFORM_NAME != -maccatalyst` 时直接报中文指引（原生目标构建在编译前即被拦下，双向实测）。
- **生成顺序**：`xcodegen generate` → `pod install`（xcodegen 重写 .xcodeproj 会抹掉 Pods 集成，重新生成后必须重跑 pod install）。开 `macosApp.xcworkspace`。
- **签名**：project.yml 用 Manual + ad-hoc（`CODE_SIGN_IDENTITY: "-"`，即 Xcode 的 Sign to Run Locally）；Automatic 无开发者团队直接拒建。⚠️ 曾试过「iOS 平台 + SUPPORTS_MACCATALYST」做 Catalyst-only 杜绝误选：iOS 平台目标强制要求 development team，而本机全部 Apple Development 证书均无 Mac 开发能力（3 个团队逐一实测均报 `No "Mac Development" signing certificate matching team ID`）——勿回退那套，守卫脚本已解决误选问题。
- **Podfile Catalyst 配方**：`platform :ios`（**不能** `:osx`——UIKit Pod 按 macOS 原生平台编不出产物）+ post_install 给全部 Pod 目标开 `SUPPORTS_MACCATALYST`、排除 x86_64、**强制 `IPHONEOS_DEPLOYMENT_TARGET=15.0`**（⚠️ Catalyst 的 macOS 部署目标由 iOS 目标**映射**而来：iOS 13→macOS 10.15 会被新 Xcode SDK 拒建；只改 MACOSX_DEPLOYMENT_TARGET 对 iOS 平台 Pod 无效，实测踩坑）。
- **框架路径**：macosApp 在 `apps/` 子目录，shared 在 `client/` 根——相对路径要上**两级**（`../../shared/...`），且链接搜索路径用 `$(PROJECT_DIR)/../../shared/...`（纯相对路径 ld 找不到）；产物目录是 `releaseFramework`（无 releaseShared）。
- **macabi 平台补丁（已自动化）**：Kotlin/Native 无 macabi 目标，静态框架（ar 归档）内目标文件平台标记是 macOS，ld 拒绝链入 Catalyst。`shared/linkReleaseFrameworkMacosArm64` 等框架链接任务已 `finalizedBy` 补丁任务，自动执行 `scripts/patch_framework_macabi.py`（ar x → 改写 LC_BUILD_VERSION platform 1→6、minos≥12.0 → ar cr → ranlib，幂等）。本机 vtool 不认 macabi 平台名，勿走 vtool。

## 6.5 HTTP 端点与插件化约定（2026-10 新增）

| 端点 | 语义 |
|---|---|
| `GET /api/plugins` | 能力清单 + 实际状态（`enabled` 与 `phase` 分离，带 `last_error`）；`config.path/persistent` 一并返回。**管理页「总览」tab 的唯一数据源**（`PluginMetaState.kt`）：必需/可选两组 + 计数摘要 + **降级/异常原因行内显示** |
| `GET /api/config/schema` | 字段元数据（标签/类型/默认值提示/`hot` 热生效语义/`secret` 密钥标记）+ `app_sections`（`[server]`/`[audio]` 这类**非能力段**的档位）；**只做元数据，不做自动表单**。客户端 `FormSchemaState.kt` 用它渲染「这个字段多久生效」（`live` 不标注） |
| `GET /api/usage`、`GET /api/session/{id}/usage` | 令牌用量（全局 + 按会话；`pressure` = 输入侧总量，回答"记忆注入让 prompt 长了多少"） |
| `GET/POST /api/config` | **部分更新**（缺失字段保持不变、显式 `null` 清除）；GET **密钥打码**（明文永不回传，只给 `has_<field>` presence）并带 `revision`，POST 回传 `expected_revision` → 不一致 **409**（客户端顶栏出现「重新加载」，草稿保留）；密钥空串/缺失 = 保留原值。**POST 响应带 `hot`/`hot_hint`/`changed_sections`**：本次保存**实际**的生效档位（取变更字段里最严格的一个，判定在 `app/ws/config/impact.rs`），客户端据此显示三档文案 |
| `GET /api/memory/status` | 记忆状态卡：引擎（`engine`）/库大小/条数/待抽取/隔离/各设备作用域 + 能力 `phase` 与降级原因 + AIUI 提示 |
| `POST /api/memory/recall` | **当场试召回**（`{text, scope?}`；scope 省略 = 跨设备）：命中数/匹配词项/耗时/**将注入的正文预览**——"记忆开了没用"的唯一反馈手段 |
| `POST /api/memory/maintain` | 立即维护（`{force, dry_run}`）：隔离释放 + 保留策略；`dry_run` 只报数量不删除 |
| `POST /api/memory/clear` | 清空（**必须**显式给 `scope` 或 `all:true`，防误删长期记忆）|
| `GET /api/soul/presets` | 内置人格预设清单（`preset` 可选值 + 可直接喂给 `[soul]` 表单的 `profile`）——管理页「灵魂」卡的下拉与「载入预设内容到表单」的数据源 |
| `POST /api/soul/preview` | **最终提示词预览**（`{device_id?, soul?}`，`soul` = 表单草稿）：`instructions` + 分段 + 字符/token 估算 + `from_preset`/`overridden` 字段来源 + `effective` 完整档案；未知 preset/非法变量在这里当场报错（= **保存前校验器**，不必先写盘）|

约定（改插件前必读）：

- **热生效语义**由 `plugins/registry/` 的 `hot` 三档声明（`live`/`next_session`/`restart_only`），
  **不要**在文档里另写一套"需重启"的说法；`GET /api/config/schema` 是给前端看的权威。
  ⚠️ **新增配置段必须登记档位**：能力字段写进注册表字段表；`[server]`/`[audio]` 这类应用级段写进
  `registry/APP_SECTION_HOT`。忘了登记不会报错，只会让 `POST /api/config` 的 `hot` 保守返回
  `restart_only`、界面永远提示"需重启"——`config/tests/impact.rs::every_config_section_has_declared_hot_semantics`
  把这种静默降级变成硬失败。
- **`enabled` 与 `phase` 必须分开**：前者是用户保存的选择，后者是实际状态。只报一个布尔值会让 UI
  无法表达「已启用但降级/加载失败」。
- **必需 vs 可选**：`asr/vad/tts/llm` 校验/构建失败 = 启动失败；`fullchain`（AIUI）启用后校验失败
  也是致命（保持既有行为）；其余可选能力（`memory`/`soul`/`voices`/`firmware`）失败只降级（`Degraded`）。
  有共享实例的可选能力经 `PluginHost::build_optional` 构建（当前只有 `memory`）。
- `GET /api/config/schema` 的 `common_fields` 是**数组**：`[{fields_path, fields}]`
  （同一能力可有多个配置段，如 `[memory]` / `[memory.retention]` / `[memory.extractor]`）；
  改这个形状要同步改消费方（`client/.../FormSchemaState.kt`，P5 已接入）。
- **管理页 tab 由常量表驱动**：`client/.../ConfigSections.kt` 的 `SectionDescriptor(id, order, label)`
  是唯一清单（构建期校验 id 唯一/order 升序），内容在 `ConfigCards.renderForm` 的 `when` 里。
  新增 tab = 改这张表 + 加一条 `when` 分支；`id` 是稳定标识（不随 label 文案变化）。
  ⚠️ 内容必须写在传给 `TabPage` 的 lambda 里——把 lambda 装进 `when` 再返回 `ViewBuilder`
  会丢接收者（实测编译报 `receiver type mismatch`）。
- 校验文案必须含「怎么修」，且**一次汇总多个问题**（不要修一个报一个）。

## 7. 运行与联调

```bash
# 启动服务（后台；需 --features sherpa 与本地模型）
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

`XIAOZHI_CONFIG` 环境变量 → `--config <path>` 参数 → 内置默认（/data/models 生产路径）。

## 9. 已知限制 / 未实现

- **激活流程（OTA/activate）本期未实现**：接受任意设备，`expected_token` 为空则跳过鉴权。
- **VAD 模型加载**：每次会话 `VoiceActivityDetector::create` 会加载模型；连接数大时建议池化（待优化）。
- **ASR 流式**：SenseVoice 为离线逐段识别；如需逐字流式可后续换 Zipformer `OnlineRecognizer`。
- **未启用 `sherpa` 的编译下行音频为空帧**：`encode_opus_frame` 占位实现返回空，仅供编译/测试。
- **记忆（P1）不含语义检索**：无 embedding、无 PPR/LPA 社区、无 SPO 三元组图（均属 P2）；
  当前是"摘要 + 汉字二元组倒排索引"的本地词法召回（零外网调用，中文可用）。
- **记忆未接测试台协议消息**：`protocol.rs` 未新增 `memory_test`（避免动协议枚举；管理页的
  「测试召回」走 `POST /api/memory/recall`）。因此 `mock_client.py` 的断言序列不变。
- **人格未支持 `profile_path`**（Markdown 档案文件）与设备级多人格绑定：属于后续阶段。
- **AIUI 全链路下人格与记忆均不生效**（云端闭环绕过本地 LLM）：`aiui_downlink` 既不 recall 也不 record。
- 编译 warning：**客户端两个 Kotlin 目标 0 warning**（2026-10 P5 批次清掉了未用导入与 `data == null`
  这类编译期恒假的死判断——`NMAllResponse` 的 `data` 是非空 `JSONObject`，别再照抄该写法）；
  Rust 侧 `cargo test` 0 warning，但 `cargo check` 仍有 **3 条既有 dead_code**（`ExtractorConfig::is_empty` /
  `MaintainOpts::forced` / `ORDER_MEMORY`，属预留 API）。

## 10. 提交规范（重要）

本项目约定：**不要自动提交代码**。完成改动并编译通过后停在工作区，由用户决定何时、以什么范围提交。不要自行 `git add` / `git commit`。

## 11. 更多上下文

- 设计取舍、分阶段实施、协议对齐要点见 [`PLAN.md`](./PLAN.md)。
- 用户向文档、快速开始、Docker、协议要点见 [`README.md`](./README.md)。
