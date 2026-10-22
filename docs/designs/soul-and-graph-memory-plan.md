# 给 LLM 加「灵魂」与「记忆」——集成 graph-memory 图记忆 + 管理界面配置（历史设计与实施记录）

> 定位：本文是人格与图记忆的历史长文。早期“没有人格 / 记忆代码”等盘点与后续实施记录属于不同时间点，不应混作当前事实；记录时点状态为 **P0（灵魂）与 P1（记忆骨架）已实现，P2/P3 未开始**。
> 章节分布：**§0–§3、§8–§10 见本页**；§4 见[人格维护](<../../server/src/plugins/soul/AGENTS.md>)；§5 见[记忆维护](<../../server/src/plugins/memory/AGENTS.md>)；§6–§7 见[管理台界面设计与验收记录](<admin-ui-design.md>)；§11–§12 见[提示词维护](<../../server/src/plugins/prompt/AGENTS.md>)。
> 文中形如 `agents.md §N` 或 `AGENTS.md §N` 的引用指重组前的旧长文。
> 相关主题：[插件架构统一](<plugin-architecture-unification.md>)、[P1 第一轮](<../plans/p1-round1-design.md>)、[文档总索引](<../README.md>)。

---

> 状态：**P0（灵魂）+ P1（记忆骨架）已实现**（2026-10）；**P2（语义检索/图排序）与 P3（互操作/工具）未做**。
> ⚠️ §1「现状盘点」是**实现前的快照**（例如"系统提示词是一个静态字符串""仓库内没有任何 memory 代码"），
> 现在已不成立——落地实况与偏差见 **§0.5**，实现文件见 §10 的更新说明。
> 所有「现状」结论均标注了本仓库源码位置；graph-memory 结论标注了实测/源码出处。
> 目标读者：要动手实现这件事的工程师（含 AI 编码助手）。
> 关联：[插件维护](<../../server/src/plugins/memory/AGENTS.md>)（陷阱与约束）、[架构概览](<../architecture.md>)、[ESP 兼容与延迟计划](<../plans/esp-compat-and-latency-plan.md>)；原 `agents.md` 长文已随文档重组移除。
>
> **DSH 结论的取证方式（重要，避免复现踩空）**：`/Applications/DeepSeek Harness.app/Contents/Resources/app.asar` 是**归档文件**（121 MB，`file` 判为 `data`），**不是目录**——shell 执行 `ls .../app.asar/dsh` 报 `Not a directory`（DSH 自身工具能把该路径当虚拟目录读，普通 shell 不能）。DSH 源码在 asar **内部**的 `dsh/node_modules/`：实测 12973 个文件 / 520 个包，其中 `@deepseek-ai/*` **290 个**；读取需按 asar 头解析（`uint32 4` + `uint32 headerSize` + pickle：`uint32 payloadSize` + `uint32 jsonSize` + JSON，数据区基址 `8+headerSize`，**offset 在 JSON 里是字符串**）。`app.asar.unpacked/dsh/node_modules` 只有原生模块（node-pty 等）。
> 本机用户级状态：`~/.dsh/profiles/desktop/`（graph-memory 已装并在跑）、`~/.dsh/graph-memory/graph-memory.db`；**`~/.openclaw/` 不存在** —— graph-memory 的 OpenClaw 半边在本机未启用，实际在跑的是 DSH 半边。

---

## 0. TL;DR（三条结论）

1. **graph-memory 不是一个服务，是宿主的进程内插件。** 它没有 HTTP / MCP / CLI / stdio 接口（无 `bin` 字段、grep `createServer|listen|stdio|child_process` 零命中），`dist/dsh.js` 通过 Cordis 的 `inject = ["tools","llm","systemPrompt","agentLoop","agents","sessions","credentials","tokenMeter"]` 直接消费宿主能力。所以「Rust 服务端通过接口调用 graph-memory」这条路**不存在**——只能 (A) 用 Rust 重写其核心、(B) 自写 Node sidecar 包裹（上游不提供 IPC，得自己造）、(C) 借道 DSH；**推荐 A**：单进程零 Node 依赖，且按同 schema 建库后能**只读复用 DSH 已在跑的同一个 SQLite 库**（`~/.dsh/graph-memory/graph-memory.db`）实现跨宿主记忆复用（需复刻其 m15/m16 两层表与稳定 ID 生成规则）。
2. **「灵魂」和「记忆」都应该是 LLM 之前的可插拔上下文生产者**，而不是继续往 `[llm].system_prompt` 这一个字符串里塞。落地形态：新增 `[soul]`（人格档案）与 `[memory]`（图记忆）两个顶层配置段 + 一个 `ContextAssembler`（persona → memory → history → user），唯一改动点是把 `plugins/llm/openai.rs:130` 的静态 `instructions` 换成组装结果。
3. **管理界面建议新增「灵魂」「记忆」两个 tab（而非塞进 LLM 卡）**，并新增 `GET /api/config/schema` + `POST /api/memory/recall` 两个端点：前者让服务端成为字段的唯一权威（DSH 的 `dsh-settings` 正是把插件 `Config` 投影成表单、并**拒绝过期写入**；但实测其 `autoGenerate` 开关「no shipped client does so yet」——**不要指望"schema 自动生成表单"**，schema 用于校验/默认值/字段提示，tab 仍手写），后者让用户能**当场验证记忆是否真的在工作**（没有这个反馈，记忆功能一定被用户判定为"没生效"）。

---

## 0.5 落地状态与偏差（2026-10 实施）

> 设计（本文）→ 实现（`server/src/plugins/{prompt,soul,memory}/` + 管理页两个 tab）的映射与取舍。
> 统筹视角见 [`plugin-architecture-unification.md`](<plugin-architecture-unification.md>) §0.9。

### 已实现

| 设计章节 | 落地 | 文件 |
|---|---|---|
| §4 灵魂 | `[soul]` 21 个字段 → 有序 prompt 段；**关闭时逐字节退回 `[llm].system_prompt`**（回归单测） | `plugins/soul/`、`plugins/prompt/` |
| §4.2 字段模型 | 21 个字段；**`preset` 内置默认灵魂**（留空即用、填了就覆盖、`none` 完全自定义） | `plugins/soul/presets.rs`、`mod.rs::effective` |
| §4.3 组装顺序 | 平台约束(-1000) → 人格主体(0) → 表达风格(100) → `[llm].system_prompt`(200)；记忆块按 `inject_position` 进 `input[]` | `plugins/soul/mod.rs`、`plugins/llm/openai.rs` |
| §5.1 插件抽象 | `MemoryProvider` trait（`recall`/`record`/`extract_pending`/`maintain`/`stats`/`clear`）+ `NoopMemory`（默认零成本） | `plugins/memory/mod.rs` |
| §5.2 数据流 | 关键路径 `recall`（硬预算、失败静默降级）+ 后台 `record`→`extract_pending`→周期 `maintain` | `plugins/memory/graph.rs`、`workflow/session/reply.rs` |
| §5.3 schema | `gm_messages`/`gm_turn_memories`/`gm_turn_memory_sources`/`gm_summary_terms` + `_migrations`；稳定 ID 同方案（sha256 前缀） | `plugins/memory/schema.rs`、`store.rs` |
| §5.4 组装模板 | `<recalled_memory>` + 固定防注入前言 + `<memory_capsules>` + `<episodic_context>`（证据=原始 Q/A）+ XML 转义 | `plugins/memory/assemble.rs` |
| §5.5 召回链 | 词法召回（摘要倒排索引）→ 排除近况窗口 → 按命中词项数排序 → 取原始 Q/A | `plugins/memory/store.rs`、`terms.rs` |
| §5.6 配置 | `[memory]` + `[memory.retention]` + `[memory.extractor]`（实际字段见 `config.example.toml`） | `plugins/memory/mod.rs` |
| §5.7 作用域 | `scope = <prefix>:<device_id>`（`Device-Id` 握手头已 plumb 进会话） | `gateway/ws/mod.rs`、`workflow/session/mod.rs` |
| §6 插件化映射 | 两个新描述符 `memory.graph`/`soul.profile`；`Phase::Degraded` 而非致命；`/api/plugins` 可见 | `plugins/registry/impls.rs`、`host.rs` |
| §7 管理界面 | 新增「灵魂」「记忆」两个 tab + 状态卡 + 测试召回 + 立即维护 + 两段式清空 | `ConfigContextState.kt`、`ConfigSoulMemoryCards.kt` |
| §10 文件清单 | 全部落地（`protocol.rs` 除外，见偏差 6） | — |

### 与设计的偏差（逐条，含理由）

1. **召回用"自建倒排索引"而不是 FTS5**（§5.3 的增补表 + §2.4 缺陷 2）：`unicode61` 会把整段中文当成
   一个 token（子串查询不命中）、`trigram` 要求查询 ≥3 字且换说法就失配。改为把摘要切成
   **汉字二元组 + ASCII 单词**建 `gm_summary_terms` 倒排索引，召回按"命中的不同词项数"排序——
   无 embedding 时中文才真正可召回（`terms.rs` 有"换说法仍命中"的回归测试）。
2. **P2 的图能力未做**：embedding/语义 Top-K、局部 LPA 社区、query-time PPR、SPO 三元组与 `gm_navigation_terms/triples`
   全部未实现（抽取只产出 `summary` + `keywords`）。理由：那套复杂度只有在"图排序"阶段才有回报，
   先落会被**未被使用**的脏数据（模型输出的三元组不稳定）。
3. **不暴露"配了不生效"的旋钮**：`[memory.embedding]`、`semantic_score_threshold`、`pagerank_*`、
   `llm_failure_cooldown_s` 均未出现在配置里（§5.6 的示例因此与实现不同，以 `config.example.toml` 为准）。
4. **`recall_max_chars` → `recall_max_tokens`**（采纳 `plugin-architecture-unification.md` §4.3 的修订）：
   中文按 ≈1 token/字估算，超预算时**先丢证据原文、再丢摘要**，全丢光则返回空块（不注入空壳）。
5. **保留策略反而做了，并且修掉了上游的缺陷**（§2.4 缺陷 1）：候选查询带
   `id NOT IN (SELECT message_id FROM gm_turn_memory_sources)`——上游正是在这里误删被轮次记忆引用的原文，
   导致"证据层"变空。回归测试 `retention_never_deletes_referenced_evidence` 直接锁定这一点。
6. **未动协议枚举**：没有新增测试台 `memory_test` 消息（§1.6 的注意事项）；管理页「测试召回」走
   `POST /api/memory/recall`。理由：动协议必须真机联调（本机无模型，`mock_client.py` 跑不了）。
7. **`profile_path`（§4.2 档案文件）未做**：需要每轮/每会话读盘并处理"文件改了但服务未刷新"的一致性问题，
   收益在语音端很低；人格由配置字段内联表达。
8. **`LlmProvider::chat_stream` 增加 `&TurnPrompt` 入参**（§4.3 的落点从"预组装字符串"改为"传计划"）：
   `instructions` = 稳定前缀（平台+人格+附加约束），`memory` 块走 `input[]`，从而保住前缀缓存。
9. **`[soul]` 的段顺序**：平台约束段只在 `enabled=true` 时出现（保证关闭时逐字节回退）；
   `[llm].system_prompt` 作为附加约束(200)保留，且**不做变量插值**（旧配置里可能有花括号）。

### 验收证据（本次实施）

- `cargo test` 默认与 `--features sherpa` 各 **178 passed / 0 failed / 3 ignored**（含记忆库真 SQLite 单测、
  假 LLM 的抽取端到端、保留策略回归、persona 逐字节回退）；
- `./gradlew :apps:h5App:publishWeb` BUILD SUCCESSFUL（产物含「灵魂」「记忆」tab 与新字段文案）；
- ⚠️ **未做**：真实模型下的端到端联调（本机无 `/data/models`）、真机长对话的召回效果抽查、
  与 DSH 现有 `graph-memory.db` 的只读互操作（P3）。

---

## 1. 现状盘点（本仓库代码事实）

### 1.1 系统提示词：一个静态字符串，没有任何分层

| 事实 | 位置 |
|---|---|
| `LlmConfig` 只有 `api_base/api_key/model/system_prompt/max_history/temperature/stream` | `server/src/plugins/llm/mod.rs:13-31` |
| 请求体里 `instructions` = `cfg.system_prompt` 原文，无变量插值、无人格分层、无记忆注入 | `server/src/plugins/llm/openai.rs:128-134`（`"instructions": self.cfg.system_prompt`） |
| 默认人格 = 一句话 `"你是一个有用且简洁的中文语音助手。"` | `server/src/plugins/llm/mod.rs:40-42` |

### 1.2 多轮历史：`Vec<(String,String)>`，按**条目数**截断

- 会话历史字段与生命周期：`server/src/workflow/session/mod.rs:69`（`history: Vec<(String,String)>`）→ 写入 `564-566` → 截断 `568-571`。
- ⚠️ 另一处写入/截断在 `session.rs:236-241`（测试台路径）。
- ⚠️ **语义瑕疵**：`max_history` 默认 10，但比较的是 `history.len()`（user+assistant 各占 1 条）→ 实际只保留 **5 轮**。UI 标签 `max_history` 也没说明单位。做记忆功能时建议一并改为「轮数」语义或明确写成 `max_history_turns`。

### 1.3 引擎装配与热切换：已有成熟范式可复用

- `Engines` 持有 `asr / tts(RwLock) / tts_sig / tts_pool / llm / tts_cache / config / config_path`：`server/src/runtime/mod.rs:38-54`。
- **热切换范式**（新增能力照抄即可）：`refresh_tts_from_disk()` 读盘 → 比对 `engine_signature()` → 变化才重建 → `RwLock` 替换 → 清空派生缓存：`runtime/mod.rs:139-166`。
- 构建期快速失败范式（缺参数即启动报错）：AIUI 三要素校验 `runtime/mod.rs:64-71`。

### 1.4 工具调用面已预留，但未接通

- `ToolSpec` / `ToolCall` / `LlmEvent::ToolCall` 已定义：`server/src/plugins/llm/mod.rs:68-110`。
- `chat_stream(history, user_text, tools, on_event)`：`mod.rs:114-124`；会话当前传 `None`（`session.rs:381-384`）。→ 将来把 `gm_search` 作为工具暴露给模型，接口已经就绪。

### 1.5 配置读写与路由

| 能力 | 位置 |
|---|---|
| 顶层配置段 `server/audio/asr/vad/tts/llm/aiui` | `server/src/config.rs:32-51` |
| `GET /api/config`（实时读盘、坏文件回退内存配置） | `server/src/gateway/ws/config.rs:22-46` |
| `PUT/POST /api/config`（原子写 + 单文件 bind mount 回退原地写） | `ws/config.rs:55-77`、`82-135` |
| 路由表 | `server/src/gateway/ws/mod.rs:55-71`（已有 `.merge(firmware::router())`、`.merge(voices::router())` 两个子路由先例） |
| 配置往返单测（改字段必扩展） | `ws/config.rs:233-380`（`put_pipeline_client_json_roundtrip`） |

### 1.6 测试台是最合适的验证入口

- `hello.test=true` 的会话才受理 `asr_test/tts_test/llm_test`：`server/src/workflow/session/mod.rs:5`、`578`。
- 协议枚举：**`ClientMessage` 用 `rename_all = "snake_case"`**（`protocol.rs:108`，多单词 → `llm_test`）；**`ServerMessage` 用 `lowercase` + 逐个显式 `#[serde(rename=...)]`**（`protocol.rs:173`、`202-216`）。
- ⚠️ 注意：原 `agents.md` §5.1（已移除）写的是「ClientMessage 用 lowercase」，与当前代码（`snake_case`）不一致；**以 `protocol.rs` 为准**。新增 bench 消息类型必须两边都对齐，并重跑 `server/tests/mock_client.py`。

### 1.7 空白：仓库内**没有任何** memory / persona 代码

`grep -ni "memory|persona|soul|记忆|人格|灵魂"` 在 `server/src`、`server/config.example.toml` 中零命中（客户端仅有的 2 处是「记住上次地址」的无关注释）。属于全新能力。

---

## 人格与记忆接入记录


### 0.9 第五批：P6 灵魂 / 记忆接入（`soul-and-graph-memory-plan.md` 的 P0+P1）

> 设计与取舍的完整讨论见 [`soul-and-graph-memory-plan.md`](<soul-and-graph-memory-plan.md>)（该文档已同步标注本次落地状态与偏差）。

**落地内容**（后端 21 个新文件 + 客户端 2 个新文件）：

| 关注点 | 落地形态 |
|---|---|
| 有序 prompt 段 | `plugins/prompt/`：`PromptSection{order,name,text,interpolate}` + `assemble()`（升序、同 order 按 name、空段丢弃、`\n\n` 连接）+ **启动期变量校验**（未知/畸形变量直接报错，绝不把 `{{xxx}}` 塞给模型） |
| 人格 | `plugins/soul/`：`[soul]` 21 个字段 → 平台约束段(-1000) + 人格主体(0) + 表达风格(100) + `[llm].system_prompt`(200)；**关闭时逐字节退回 `system_prompt`**（回归单测） |
| 记忆 | `plugins/memory/`：`MemoryProvider` trait + `NoopMemory`（默认，零成本）+ `GraphMemory`（SQLite） |
| 记忆分层 | `gm_messages`（原文事实源，不可变）→ `gm_turn_memories`（摘要=导航层）+ `gm_summary_terms`（**自建倒排索引**）→ 注入正文取 `gm_turn_memory_sources` 绑定的**原始 Q/A**（证据层） |
| 关键路径 | `recall()` 有 `recall_budget_ms` 硬预算，超时/DB 错**静默降级为空**（返回结构里只有 `error` 字段，**从不 `Err`**） |
| 后台路径 | `record()` 只落原文；`extract_pending()` 才做那 1 次辅助 LLM 调用（`spawn` 在回复完成之后），失败→隔离表 + 指数退避 + 上限；`maintain()` 按 `maintenance_interval` 自行判断是否该跑 |
| 注入位置 | `[memory].inject_position`（默认 `after_history`）：记忆块进 `input[]` 而不是 `instructions`，保住 `instructions + history` 前缀的 prompt cache |
| 端点 | `GET /api/memory/status`、`POST /api/memory/{recall,maintain,clear}`（清空**必须**显式 `scope` 或 `all:true`） |
| 插件化 | 新增两个能力描述符 `memory.graph` / `soul.profile`（`Capability::Memory/Soul` 从"规划中"改为已实现）；`PluginHost::build_optional` + `Engines::refresh_memory`（可选能力失败只降级）；`Capability::implemented()` 恒真 |
| 客户端 | 管理页新增「灵魂」「记忆」两个 tab；`ConfigContextState`（`form.sm`）承载 40 个字段 + 记忆动作（刷新状态/测试召回/立即维护/演练/两段式清空） |

**与设计的偏差（逐条）**：

1. **没有 embedding / 语义 Top-K / PPR / LPA 社区 / SPO 三元组**（都是 P2）：v1 用**自建倒排索引**（汉字二元组 + ASCII 单词）做词法召回。为什么不用 SQLite FTS5：`unicode61` 把整段中文当一个 token、`trigram` 要求查询 ≥3 字且换个说法就失配（上游的 `LIKE '%完整短语%'` 更弱）。
2. **不暴露 `[memory.embedding]` 配置段**：不提供"配了不生效"的旋钮。同理不暴露 `semantic_score_threshold` / `pagerank_*` / `llm_failure_cooldown_s`。
3. **`recall_max_chars` → `recall_max_tokens`**（按本文件 §4.3 的修订）：中文按 ≈1 token/字估算，超预算先丢证据原文、再丢摘要。
4. **保留策略反而做了，并且修掉了上游的缺陷**：候选查询带 `id NOT IN (SELECT message_id FROM gm_turn_memory_sources)`——上游正是在这里误删了被引用的证据（已有回归测试 `retention_never_deletes_referenced_evidence`）。
5. **`profile_path`（Markdown 档案）未做**：它要求每轮/每会话读盘并处理"文件改了但服务没刷新"的一致性问题，收益在语音端很低。
6. **未新增测试台协议消息**：`protocol.rs` 一行未改（§5.1 的纪律：动协议必须真机联调，而本机无模型跑不了 `mock_client.py`）。管理页的「测试召回」走 `POST /api/memory/recall`。
7. **`LlmProvider::chat_stream` 增加 `&TurnPrompt` 入参**：这是本次唯一的 trait 签名变更（设计里已在 §4.4 预告）。`instructions` = 稳定前缀，记忆块按 `inject_position` 进 `input[]`。
8. **`GET /api/config/schema` 的 `common_fields` 由"字段数组"改为"`[{fields_path, fields}]` 数组"**：一个能力可有多个配置段（`[memory]` / `[memory.retention]` / `[memory.extractor]`）。客户端 P5 才消费该端点，故当前无兼容问题。
9. **`ExtractorConfig` 手写 `Default`**：`#[derive(Default)]` 会让 `temperature` 取 0.0，而 serde 字段级默认是 0.1——两者不一致会导致"整段不写"与"写了表但省略 temperature"行为不同（单测很难覆盖，已修 + 断言）。

**验收证据**：`cargo test` 默认与 `--features sherpa` **各 178 passed / 0 failed / 3 ignored**；`./gradlew :apps:h5App:publishWeb` BUILD SUCCESSFUL（产物含「灵魂」「记忆」tab 文案）；文件规模与测试分离校验均无输出。⚠️ **未做**：真实模型下的端到端联调（本机无 `/data/models`）、真机长对话的召回效果抽查。

---

## graph-memory 尽调与集成路线


## 2. graph-memory 尽调（把它当第三方依赖评估）

### 2.1 它是什么

- `adoresever/graph-memory` **v1.6.0-beta.17**，MIT，TypeScript，`engines.node >= 22.13`，**唯一运行期依赖 `@sinclair/typebox`**（其余 peer 依赖 `@deepseek-ai/cordis`、`@deepseek-ai/dsh-typert-protocol`、`openclaw` 全为 optional）。
- 定位：**图记忆 = 导航层，原文问答才是证据**。写入一轮 = 1 次辅助 LLM 调用（只送用户原问 + 最终可见回答，不送思维链/工具流水）；社区检测（局部 LPA）与 PPR 为**纯本地算法**；embedding 可选（不配则 FTS5）。
- 双宿主适配：
  - **DSH 原生 Cordis 插件**：`dist/dsh.js` — `export const name = "graph-memory-dsh"`、`export const inject = [...]`、`export function apply(ctx, input = {})`。
  - **OpenClaw Context Engine 适配器**：`index.ts`。
- 自报基准（其仓库 20 轮 GLM-5.2 实验，非严格 A/B，**需自行复现**）：T20 首请求 56,998 → 11,008 tokens（−80.7%），模型可见消息 171 → 21，20/20 抽取成功、0 隔离。

### 2.2 它**不是**什么（关键，三条独立证据）

- **不是 HTTP 服务、不是 MCP server、不是 CLI、不是 stdio 工具**：
  1. `package.json` **无 `bin` 字段**，运行时依赖只有 `@sinclair/typebox`，`exports` 仅 `"."`（OpenClaw 入口）/`"./dsh"`（DSH 入口）/`"./pro"`/`"./pro/dsh"` 四个 ESM 模块入口；仓库 tree 里没有 server/bin/cli 文件。
  2. 对已安装包（`dist/`、`src/`、`dsh.ts`、`index.ts`）全量 grep `createServer|\.listen\(|WebSocketServer|stdio|child_process|\bmcp\b` → **零命中**。
  3. 唯一「远程」痕迹是可选 `pro/`：`GraphMemoryProRemoteService extends TypertRemoteService`，用 `@Remote` 暴露只读 `snapshot()/detail()`——那是 **DSH 自己的 Host Gateway ↔ Client API 内部线协议**（`@deepseek-ai/dsh-typert-protocol`），面向 DSH Web 客户端，不是独立服务；上游 `dsh-pro/README_CN.md` 还强调「浏览器不获得数据库路径、连接、SQL、凭据」。属 planned/experimental，**不可作为生产集成面**。
- 它的「能力」全部来自宿主注入：`inject = ["tools","llm","systemPrompt","agentLoop","agents","sessions","credentials","tokenMeter"]`（`dist/dsh.js:26`）——注册工具、发起辅助 LLM 调用、改写模型可见的 prompt、订阅 agent 循环事件、读写凭据。
- 两个宿主适配器共享同一套 `src/` 核心：OpenClaw 侧注册 Context Engine 槽位（`info/bootstrap/ingest/assemble/compact/afterTurn/dispose`，注意**槽位里没有 `recall`**，召回由 `assemble`/hook 内部调用），DSH 侧挂在 `agent/pre-step` 上改写旧前缀 + 把召回快照插到当前用户消息之前。

**上游文档明确列出的宿主前置条件**（缺一不可）：① 不可变的事件/消息事实源；② 可单独修改的 model-visible surface；③ 模型请求前的插入钩子；④ 不改宿主核心即可调用的 replace/shadow API。外加 Node ≥22.13 进程、一条抽取 LLM 路由、可选 embedding 凭据。

> **结论**：`npx graph-memory` 之类不存在，也**不存在"起个服务让 Rust 调"的选项**。任何非 Node 宿主只有两条路：「用 Rust 重写核心」或「自带 Node sidecar + 自建 IPC（上游不提供任何 IPC 边界）」。
> ⚠️ 对本项目还有一层现实含义：xiaozhi-server-rust 的会话事实源是**语音轮次**（ASR 文本 + TTS 回复），model-visible surface 就是 `instructions` + `history`（`plugins/llm/openai.rs`）。上面前提 ①②③④ **我们恰好全都具备**——所以"借鉴其设计并在本仓库实现"是顺理成章，而"直接复用其代码"不行。

### 2.3 配置旋钮全清单（实测自本机已安装实例 + 上游 `openclaw.plugin.json`）

本机**已经装好并在跑**：`~/.dsh/profiles/desktop/node_modules/graph-memory`（依赖 `github:adoresever/graph-memory#5841b198...`），数据库 `~/.dsh/graph-memory/graph-memory.db`（实测含 2.5MB WAL，说明在用）。

⚠️ **只有 DSH 半边在本机运行**：`openclaw.plugin.json`（含 `id/name/version/description/contracts/configSchema`，**无** `slots` 键）与 `src/openclaw-runtime.d.ts` 是 OpenClaw 适配面，而本机 `~/.openclaw/` **不存在**。因此「Context Engine 槽位」在本机是**文档事实、非运行事实**；真正在跑的是 DSH 的 `systemPrompt` 服务 + `agent/pre-step` 路径（`dsh.ts`）。我们移植算法时以 `src/` 核心为准，不要以 OpenClaw 槽位签名为准。

接线文件（这是「DSH 插件理念」的实物证据）：

```yaml
# ~/.dsh/profiles/desktop/node_modules/graph-memory/cordis.patch.yml
- insert:
    - id: graph-memory
      name: 'graph-memory/dsh'
      config:
        dbPath: !!js dshHomePath('graph-memory/graph-memory.db')
        dbBusyTimeoutMs: !!js "...GRAPH_MEMORY_DB_BUSY_TIMEOUT_MS..."
        extractionEnabled: true
        recallEnabled: true
        recallMaxNodes: 6
        semanticScoreThreshold: !!js ...
        assistantTools: none          # none | search | all
        maintenanceInterval: 6
        llmProvider / llmModel / llmReasoningEffort / llmMaxTokens
        messageRetention: { keep: all, recentTurns: 0, retentionDays: 0, batchSize: 500, dryRun: false }
        contextCompactionEnabled: true
        freshTurnCount: 5
        embedding: { apiKeyEnv, baseURL, model, dimensions }
```

旋钮语义（供我们照抄字段设计）：

| 旋钮 | 默认 | 含义 |
|---|---|---|
| `freshTurnCount` | 5 | 上下文里原样保留的最近**用户轮**数 |
| `recallMaxNodes` | 6 | 每次按当前问题最多召回几个记忆节点 |
| `semanticScoreThreshold` | 未设 | 余弦下限；不设=排序后 Top-K（**建议不设**，需按模型校准） |
| `maintenanceInterval` / `compactTurnCount` | 6 | 每 N 轮跑一次 PageRank + 社区维护 |
| `pagerankDamping` / `pagerankIterations` | 0.85 / 20 | PPR 参数 |
| `extractionEnabled` / `recallEnabled` | true | 写入 / 召回总开关 |
| `assistantTools` | none | 是否把 `gm_search`（search）或全部 `gm_*`（all）暴露给模型 |
| `contextCompactionEnabled` | true | 是否接管模型可见历史面（旧前缀折叠为一个标记） |
| `messageRetention` | keep=all | 原文保留策略；`dryRun` 先演练 |
| `dbBusyTimeoutMs` | 5000 | SQLite 写锁等待 |
| `embedding.{apiKeyEnv,baseURL,model,dimensions}` | 无 | 不配 → 走词法回退（legacy 节点用 FTS5；**轮次摘要只有 `LIKE`，见 §2.4 缺陷 2**） |
| `llm.{provider,model,maxTokens,reasoningEffort}` | 跟随宿主 | 抽取专用模型路由 |

工具面：`gm_status` / `gm_search` / `gm_record` / `gm_stats` / `gm_maintain` / `gm_retry_extraction`；`openclaw.plugin.json` 的 `configSchema` 是 **OpenClaw 侧**清单里的 JSON Schema（`type/properties/default/description`），供 **OpenClaw** 渲染配置表单——**它不属于 DSH 机制**（DSH 的配置面是插件导出的 `Config` + `dsh-settings` 投影；对 121 MB 的 asar 全量检索 `configSchema` **0 命中**、`contextEngine` **0 命中**）。DSH 侧工具受 `assistantTools`（默认 `none`）门控，**默认一个工具都不注册**；自动召回不依赖工具调用。

⚠️ **默认值陷阱（重要）**：`semanticScoreThreshold` 在上游 `DEFAULT_CONFIG` 里是 **0.70**，两个适配器都写成 `?? DEFAULT_CONFIG.semanticScoreThreshold`，因此**"不设"实际等于启用 0.70 阈值**——这与它自己文档里「不设置时用排序后 Top-K」的注释矛盾。而且 0.70 是针对 `text-embedding-v4` 的 p90 校准值，**换 embedding 模型必须重标**。→ 我们自己的实现应把「未设」明确实现为**纯 Top-K**（阈值 0 表示关闭），避免继承这个语义歧义。

### 2.4 上游已知缺陷 / 坑（移植时必须修，否则会踩同一个雷）

1. **retention 会误删证据（高危）**：`runMessageRetention` 的候选查询与 DELETE 复检**只查 legacy `gm_node_sources`，不查 `gm_turn_memory_sources`**；而后者 `message_id` 是 `ON DELETE CASCADE`。新架构下事实源只由 turn memory 引用（上游自己的库里 legacy nodes = 0），因此一旦把 `messageRetention.keep` 从默认 `all` 改成 `referenced`/`recent`，**会把轮次记忆引用的原始 Q/A 删掉**，`<episodic_context>`（唯一的"证据"层）直接变空。默认 `all` 所以尚未爆雷。
2. **没有 turn-memory 级的 FTS5**：FTS5 虚表只覆盖 legacy `gm_nodes`；轮次摘要的词法回退是 `summary LIKE '%完整短语%'`（要求**完整短语**命中）。→ 这意味着**在没有 embedding 的情况下，新架构的召回能力其实很弱**（尤其中文口语化提问）。我们必须自己给 summary 建 FTS5（或在中文场景默认启用 embedding），不能照抄"FTS5 回退"就当兜底方案。
3. **向量没有 ANN**：Float32 BLOB + 全表扫描 + JS 余弦。语音场景库小（几千条）尚可，但要预期 `recall` 是 O(N) 且带解码成本，需配合预算与缓存。
4. **无自动重试**：抽取失败进 quarantine 后不会自愈，只能显式 `gm_retry_extraction`；另有 `LlmFailureGuard`——401/403/404 会**暂停 LLM 调用 10 分钟**（400/422 不暂停）。移植时要保留"失败隔离但不阻塞对话"这一性质。
5. **`node:sqlite` 是实验特性**：上游用 Node 内置 `DatabaseSync`（非 better-sqlite3），运行会打 ExperimentalWarning；对 sidecar 方案是个额外的稳定性考虑。

---

## 3. 集成路线三选一

| 方案 | 做法 | 优点 | 代价 | 结论 |
|---|---|---|---|---|
| **A. Rust 重写核心（推荐）** | `rusqlite`(bundled, FTS5) + 现有 `reqwest`/`LlmProvider`；复刻 schema / 抽取契约 / 召回链 / 组装 | 单进程单二进制（符合 N5105 + Docker 低功耗目标）；无 Node 依赖；**可开同一个 db 文件复用 DSH 已积累的记忆**；召回/维护走本地算法不耗额度 | 需自己实现抽取契约校验 + 召回排序 + 组装（约 P1~P2 两个阶段） | ✅ 主推 |
| B. Node sidecar | 以 npm 依赖引入 graph-memory，**自写一层** HTTP/JSON-RPC 包裹（上游无此物），Rust 侧调用 | 算法零重写、跟随上游升级 | 镜像多一套 Node22；常驻进程内存 +50~80MB；DB 写锁与 DSH 争用；进程生命周期/崩溃恢复要自己兜；跨进程调试面变大 | 备选（若要求"与上游逐字一致"） |
| C. 借道 DSH | Rust 调 DSH 的 API gateway 让它跑插件 | 复用现成插件 | 把桌面应用拖进生产依赖；部署面/版本面爆炸 | ❌ 否决 |

**推荐 A 的额外理由（最有价值的一点）**：graph-memory 的 SQLite schema 是宿主无关的（`dist/dsh.js` 注释明说"The memory algorithms and SQLite schema stay host-neutral"）。若我们**按同一 schema 建库**，则：
- 语音端可以召回你在 DSH 里与 AI 聊出来的记忆（同名/同用户维度）；
- 反向，语音端的对话也进入同一张图（需 `provider`/`session_key` 区分宿主，graph-memory 已用 `dsh:<id>` 前缀，语音端用 `xiaozhi:<device_id>`）；
- 迁移/导出零成本。

> 注意：**默认各用各的库**（`/data/memory.db`），共享 DSH 的 `~/.dsh/graph-memory/graph-memory.db` 作为 P3 可选增强——同文件双进程写需要 WAL + busy_timeout，且要接受"桌面应用与语音服务互相影响"的运维耦合。

---

## 上下文实施路线与风险


## 8. 分阶段实施计划

### P0 — 灵魂（无新依赖，1~2 人日）✅ 已完成

改动：`config.rs` 加 `pub soul: SoulConfig`；`plugins/llm/` 新增 `context.rs`（`ContextAssembler`）；`openai.rs:130` 换用组装结果；`config.example.toml` 加 `[soul]`；客户端加「灵魂」tab + 状态字段 + JSON 映射。
验收：`cargo test` 通过（新增 assembler 单测：关闭 soul 时输出与今天逐字节一致）；`mock_client.py` PASS；macApp 测试台 `llm_test` 观察到人格生效。

### P1 — 记忆骨架（3~5 人日）✅ 已完成（横切加固 1~3 项已在更早批次完成，见 §0.5）

改动：`Cargo.toml` 加 `rusqlite = { features = ["bundled"] }`；`plugins/memory/{mod.rs,graph.rs,schema.rs,extract.rs,recall.rs,assemble.rs}`；`config.rs` 加 `[memory]`；`runtime/mod.rs` 装配 + 热切换；`session.rs` 在关键路径前置 recall、回复后置 record（后台任务，`spawn_blocking`）；`gateway/ws/memory.rs`（新路由 `/api/memory/{stats,recall,maintain,clear}`，`ws/mod.rs` merge）；`protocol.rs` 加 bench 用 `memory_test`（**snake_case + 显式 rename**）；客户端「记忆」tab。
验收：无 embedding（**自建 summary FTS5**）下能写入+召回；关掉 `[memory].enabled` 时行为与 P0 完全一致；抽取失败入隔离表且对话不受影响；`mock_client.py` PASS。

**横切加固（≈0.5 人日，建议绑在 P1 一起做）**——把 §6 映射表里"值得照搬"的三条落到本仓库，改动都很小但用户可感知：

1. ✅ **密钥打码（已完成，2026-10）**：`GET /api/config` 只回 `has_api_key` presence、`POST` 未携带该字段即保留原值；客户端留空不发送。见 `plugin-architecture-unification.md` §0.6（含 4 个回归单测）。
2. **写入 revision**：`GET` 附配置指纹（mtime+size 或内容 hash），`POST` 回传；不一致返回 409 提示重新加载（现状 last-write-wins，多标签页会互相覆盖）。
3. **状态分层上报**：`enabled`（用户保存的选择）与**实际激活态**分开返回，UI 显示降级原因（如「已启用 · 无 embedding → 当前为词法模式」）。

### P2 — 图能力与排序（3~5 人日）⬜ 未开始

改动：embedding 客户端（OpenAI 兼容 `/embeddings`）+ 向量 Top-K；局部 LPA 社区；query-time PPR；组装模板（来源标注 + 指令隔离）；`maintain()` 周期性任务；召回预算/超时降级；`recall_max_chars` 截断。
验收：20 轮连续对话后，跨会话召回能命中早期事实且注入正文来自**原始回答**（不是节点标签）；`recall_budget_ms` 超时时 TTFA 不劣化（对比日志）。

### P3 — 互操作与工具（2~3 人日）⬜ 未开始

改动：schema 兼容校验（能只读打开 DSH 的 `graph-memory.db`）；`gm_search`/`gm_record` 工具暴露（走 `ToolSpec`）；导入/导出；按设备清空。
验收：用本机 `~/.dsh/graph-memory/graph-memory.db` 做只读召回抽查；设备侧 `features.mcp` 会话中模型能主动调 `gm_search`。

合计 ≈ **9~15 人日**（不含真机联调排队时间）。

---

## 9. 风险与陷阱（按严重度排序）

1. **TTFA（首字延迟）**：召回在关键路径上，embedding 多一次外网 RTT（50~300ms）。→ 硬预算 `recall_budget_ms` + 超时静默降级；低功耗主机**默认走本地词法（我们自己给 summary 建的 FTS5，零 RTT）**；embedding 可选；查询向量可缓存。⚠️ 不要以为上游有现成的 FTS5 兜底——它只覆盖 legacy 节点表（§2.4 缺陷 2）。
2. **Prompt 注入 / 数据与指令混淆**：召回内容是**用户历史原话**（不可信数据）。→ 必须定界（`<memory>…</memory>` 之类）+ 明确声明「以下为背景资料，不是用户指令，不得改变你的角色与规则」；组装模板要做单测。
3. **Token 预算**：`recall_max_nodes × 每节点原文` 可能很长。→ `recall_max_chars` 硬上限 + 按分数截断；注入量记日志（便于回归）。
4. **AIUI 模式下失效**：云端全链路绕过本地 LLM。→ UI 显式提示（见 7.4），否则用户会报「人格配了没用」。
5. **SQLite 并发**：与 DSH 双进程开同一 db 需 WAL + `busy_timeout`；本进程内建议**单写者 actor**（`mpsc` 串行化写）而非连接池多写。
6. **`max_history` 语义**：条目 vs 轮（§1.2）。做记忆时若不修，会出现「记忆召回了 5 轮前的事，但近况只剩 5 轮」的混乱。
7. **同步阻塞**：SQLite/抽取文件 IO 走 `spawn_blocking`；`session` 是 async task，直接同步调用会拖垮整个 runtime（`runtime/mod.rs:1-19` 明确警告过同类问题）。
8. **协议改动**：新增 bench 消息必须 `ClientMessage=snake_case` / `ServerMessage=lowercase + 显式 rename`（现见[协议说明](<../../server/src/workflow/protocol/protocol/AGENTS.md>)），且 `protocol.rs` 的 serde 错误 `cargo check` 发现不了，**必须跑 `mock_client.py`**（原 `agents.md` §5.1）。
9. **依赖与镜像**：`rusqlite` 用 `bundled`（含 SQLite 源码，需 cc；`rust:1.90-bookworm` 有 gcc）→ 需确认 bundled 构建**启用了 FTS5**（`libsqlite3-sys` 的 bundled 默认开 FTS5，落地时用一条 `CREATE VIRTUAL TABLE ... USING fts5` 冒烟验证）；二进制体积 +~1MB；Dockerfile 记得加 `./server-data` 卷子目录与 `XIAOZHI_MEMORY_DB`/配置默认路径。
10. **成本**：每轮 +1 辅助 LLM 调用（可指向更便宜的模型/端点）+ 可选 embedding。低配环境建议 `maintenance_interval` 调大、抽取用小模型。
11. **人格漂移**：超长 system prompt 会被稀释，语音场景还要求短句。→ 优先用 `examples`（few-shot）锁定风格，而不是堆形容词；`max_sentences/max_chars` 在组装时作为硬约束写进指令。
12. **隐私**：记忆=长期留存用户原话。→ 默认 `[memory].enabled=false`；提供按设备清空/导出；`dry_run` 先演练保留策略；文档明确数据落在 `db_path`。

---

## 10. 附：关键文件改动清单（P0+P1）

| 文件 | 改动 |
|---|---|
| `server/src/config.rs` | `+ pub soul: SoulConfig; + pub memory: MemoryConfig;`（并 re-export） |
| `server/src/plugins/llm/context.rs`（新） | `ContextAssembler`（persona → memory → history → user） |
| `server/src/plugins/llm/openai.rs` | `:130` `instructions` 改用组装结果（`chat_stream` 增加 `context` 入参或预组装后传入） |
| `server/src/plugins/llm/mod.rs` | `LlmConfig` 保持不变（回滚兼容）；trait 签名如需扩参在此 |
| `server/src/plugins/memory/*.rs`（新） | `MemoryProvider` + `NoopMemory` + `graph.rs`（P1 起步用 FTS5） |
| `server/src/runtime/mod.rs` | `+ memory: Arc<dyn MemoryProvider>`、`+ memory_sig`、`refresh_memory_from_disk()` |
| `server/src/workflow/session/mod.rs` | 关键路径前置 `recall`；回复完成后后台 `record`；`history` 语义修正 |
| `server/src/gateway/ws/memory.rs`（新）+ `ws/mod.rs:55-71` | `/api/memory/{stats,recall,maintain,clear}` + `config_schema` 路由 |
| `server/src/gateway/ws/config.rs` | ✅ **已落地**（2026-10）：`GET` 打码 `api_key`（改返 `has_<field>`）+ 返回配置指纹；`POST` 校验指纹（不一致 409）+ 省略密钥字段即保留原值（见 `plugin-architecture-unification.md` §0.6） |
| `server/src/workflow/protocol/protocol.rs` | bench 用 `MemoryTest` / `MemoryTestResult`（注意 serde 命名） |
| `server/src/gateway/ws/config.rs:233-380` | 往返单测覆盖 `[soul]`/`[memory]` |
| `server/Cargo.toml` | `rusqlite`（bundled，P1） |
| `server/config.example.toml` | `[soul]`、`[memory]`、`[memory.embedding]`、`[memory.extractor]` 示例 + 注释 |
| `client/.../ConfigCards.kt` | `renderForm` 加两个 `TabPage`；新增 `soulConfigCard` / `memoryConfigCard` |
| `client/.../ConfigFormState.kt` | 新 observable + `buildConfigJson` + `applyConfig` |
| `client/.../AdminFormControls.kt` | 复用现有官方组件（`labeledField` / `labeledTextArea` / `switchRow` / `appButton` / 官方 AlertDialog 下拉）；**不新增自造样式** |
| `client/shared/macosArm64Main/.../TestBenchPages.kt` | 测试台加「记忆」面板（对齐 `llm_test` 的现有形态） |

---
