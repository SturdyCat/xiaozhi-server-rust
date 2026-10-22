# 插件架构统一 —— 借鉴 DSH 插件理念 + 适合本项目的 LLM 插件 + 管理页 UI 设计（历史设计与实施记录）

> 定位：本文是插件架构统一的完整历史长文。盘点描述改造前状态、设计描述目标方案、实施记录描述当时交付，均不代替当前源码、测试与维护约束。
> 章节分布：**§0–§0.6、§1、§6–§9 见本页**；§0.7–§0.8 见[插件维护](<../../server/src/plugins/AGENTS.md>)；§0.9 见[人格与图记忆](<soul-and-graph-memory-plan.md>)；§0.10 与 §2–§3 见[注册表规范](<../../server/src/plugins/registry/AGENTS.md>)；§4 见 [LLM 维护](<../../server/src/plugins/llm/AGENTS.md>)；§0.11 与 §5 见[管理台界面设计与验收记录](<admin-ui-design.md>)。
> 文中形如 `agents.md §N` 或 `AGENTS.md §N` 的引用指重组前的旧长文，不表示当前指引存在该章节。
> 当前入口：[插件层](<../../server/src/plugins/AGENTS.md>)、[注册表](<../../server/src/plugins/registry/AGENTS.md>)、[架构概览](<../architecture.md>)。

---

> 状态：**设计（未实现）**。所有「现状」结论均给出本仓库 `文件:行号`；所有 DSH 结论均给出 asar 内包名+文件
> （取证方式见 [`soul-and-graph-memory-plan.md`](<soul-and-graph-memory-plan.md>) 文首说明：`app.asar` 是归档文件不是目录）。
> 目标读者：要动手实现这件事的工程师（含 AI 编码助手）。
> 关联：[插件维护](<../../server/src/plugins/AGENTS.md>)（陷阱与约束）、[架构概览](<../architecture.md>)、[人格与图记忆](<soul-and-graph-memory-plan.md>)（本文件的 P4 依赖它）。

---

## 0. TL;DR

1. **本仓库已经有"插件"的实质，但缺"插件的形"**：五个能力各自实现了 `trait + build_*` 工厂（`asr/tts/llm/vad/aiui`），配置段也各自下沉到插件目录——这是对的方向；但它们**各写各的装配、热切换、校验、上报**，导致「新增一个实现」是"照着上一个改 4~5 处"，而不是"注册一行"。本文件把 DSH 的清单/注册表/生命周期/状态上报四条机制落成 Rust 形态。
2. **统一的关键不是引入框架，而是引入三张表**：① 能力描述符注册表（可枚举、可上报、`signature`/`build` 挂在自己身上）；② 生命周期状态机（`enabled` 与 `phase` 分离 → UI 才能显示"已启用但降级/失败"）；③ 配置约定（`engine` + `[<cap>.<id>]` 私有子段 + `hot` 热生效标记）。**五个引擎的 trait 一行都不用改**，迁移可以逐个能力做、随时回滚。
3. **DSH 里最该借的是 LLM 侧的"护栏"，不是它的 agent 编排**：本项目 `reqwest::Client` **连超时都没设**（`plugins/llm/openai.rs:82-85` 只设了 UA）、**没有任何重试/退避/可重试错误分类**、**没有 token 用量观测**、system prompt 是一个不可拆分的字符串。DSH 的 `dsh-llm-retry` / `dsh-token-meter` / `dsh-system-prompt`（section+order）/ `dsh-llm-pi-ai`（多 provider 字典 + `contextWindow` + `retryPolicy`）正对着这四个缺口。而 `dsh-agent-loop` / `subagent` / `tool-*` 那一整套**不要借**（语音单轮短对话场景）。
4. **UI 借"信息架构"而非"视觉"**：DSH 的插件/设置页长在「槽位注册 + order 排序 + 分组卡片 + 行内状态徽标」上，这套结构可以用 Kuikly 官方组件（Tabs+PageList / AlertDialog / Switch / TextArea / Button / View+Text）等价实现；但 DSH 的自绘细节（自定义胶囊、指示器、悬浮层）**在 Kuikly 官方组件约束下不实现**（`AGENTS.md` §5.8 已明令禁止自造样式）。

---

## 0.5 落地进度与偏差（本次实施）

> 状态快照：**P0~P6 全部完成**（P5 见 §0.11；P4 分两半：配置约定已落地 §0.10，统一动作模型仍未做）。
> 本节记录**实际落地情况与设计不一致的地方**；其余章节仍按"设计权威"阅读（未落地的部分不要当作现状）。
> 实施分五批：**第一批** = P0~P3（下表）；**第二批** = 密钥打码 + presence（§0.6）；
> **第三批** = 文件规模整改（§0.7）；**第四批** = 测试与逻辑分离（§0.8）；**第五批** = P6 灵魂/记忆（§0.9）；
> **第六批** = P4 配置约定统一（§0.10）。

| 阶段 | 状态 | 落地内容 |
|---|---|---|
| **P0 止血** | ✅ | `POST /api/config` 部分更新（缺失=保持、显式 `null`=清除）；前端补 `downlink_lead_ms` / `cache_entries` / 整个 `[aiui]` 段（并新增 AIUI tab，此前该段在管理页完全不可见）；会话开始处接热刷新；**以完整 `Config` 为基准**的逐字段往返回归测试 |
| **P1 契约与注册表** | ✅ | `plugins/registry/`（8 个描述符 + 字段元数据 + 签名 + 校验 + 构建入口）、`plugins/host.rs`（`plan`/`boot`/`mark`/状态快照）、`GET /api/plugins`、`GET /api/config/schema`、`Engines::new` 改经 `PluginHost::boot`（校验前移 + **多问题一次汇总**）；注册表漂移护栏测试（字段 key 必须真实存在于 `Config`） |
| **P2 统一热切换** | ✅ | `llm: RwLock<Llm>`（对齐 TTS 模式）；`Engines::refresh_from_disk()` 覆盖 tts+llm；`PluginHost::set_config` + `Engines::live_config()` 会话级快照；热切换决策抽成**纯函数** `decide_refresh` 并单测；失败保留旧实例 + 记 `last_error`；TTS 切换时清空结果缓存（否则会播出上一个引擎的音频） |
| **P3 LLM 护栏** | ✅ | ✅ `LlmFailure` 具名分类、三层超时（建连 5s / 整请求 120s 兜底 / **流空闲 20s**）、`RetryingLlm`（≤2 次重试、8s 总预算、**已吐出文本不重试**）、空回复判定、`TokenUsage` + `UsageMeter` + `GET /api/usage` / `/api/session/{id}/usage`、LLM 失败**兜底话术**（不再静默）、配置 `revision` + 过期写入 409、**密钥打码 + presence（第二批，见 §0.6）**。❌ 仍未做：LLM 卡「高级」折叠区与「测试连接」动作（属 P5 UI） |
| **P4 配置约定统一** | ✅（配置约定部分） | `engine` 键 + `[<cap>.<id>]` 私有段 + `normalize()` 双读 + **旧客户端补丁重写**；删除 `TtsBackendKind`/中心化 `build_tts`/`engine_signature`（选择/构建/签名全归描述符）；未知 engine 值在 plan/boot/热刷新三处显式报错。❌ 仍未做：`voices`/`firmware` 的**统一动作模型**（`POST /api/plugins/{id}/actions/{name}` + job 轮询）——见 §0.10 偏差 8 |
| **P5 UI 重组** | ✅（主体） | 能力总览页（`GET /api/plugins`）、状态徽标**含原因**、三档生效文案（服务端判定 `hot`）、段级/字段级热生效提示（schema 驱动）、`ConfigSections` 常量表、409「重新加载」。**偏差**：统一动作模型未做（同 P4）、§5.4-2「已覆盖/恢复默认」未做、`AdminTheme` token 未整体重写——见 §0.11 |
| **P6 灵魂 / 记忆** | ✅ | `[soul]` 人格档案（有序 prompt 段）、`[memory]` 本地图记忆（SQLite + 词法召回 + 后台抽取）、`LlmProvider::chat_stream` 收 `&TurnPrompt`、`/api/memory/{status,recall,maintain,clear}`、管理页新增「灵魂」「记忆」两个 tab。**偏差**：不含 embedding/PPR/LPA/三元组（P2 范畴）、`profile_path` 未做、未新增测试台协议消息——见 §0.9 |

### 与设计的偏差（逐条记录，便于复核）

1. **AIUI 描述符 id 用 `fullchain.aiui`**（设计里写作 `aiui.fullchain`）：为让"id 前缀 == capability id"这条**可机器校验**的命名空间规则成立（注册表单测强制）；配置段仍是 `[aiui]`。
2. **新增 `warn_if` 钩子**（设计里没有）：区分"硬条件不满足（`validate` → 不可构建）"与"能构建但能力受限"（`warn_if` → `Degraded` + 原因）。首个用例：`[llm].api_key` 为空时服务照常启动，但 `/api/plugins` 会明确报 `Degraded`，而不是静默 `Active`。
3. **AIUI 保持 `RestartOnly`**（tts/llm/vad 是 `NextSession`）：它决定整条流水线走向，且启用时凭据缺失是**致命**错误；热启用会带来"半配置状态下切换链路"的风险，收益不值。
4. **`Built` 里没有 `Vad` 变体**：VAD 是每会话构造（`Engines::new_vad`），没有共享实例可构建；描述符只承担校验与上报。
5. **保留 `refresh_tts_from_disk`**：测试台 `tts_test` 需要"只看 `[tts]`"的语义（LLM 配置坏掉不该让 TTS 测试失败）；会话开始用的是统一入口 `refresh_from_disk`。
6. **`live_config()` 的边界**：`[server]` / `[audio]` / `[aiui]` 仍读**启动快照**（握手与 OTA 必须与启动值一致，属 restart-only）；`[vad]`、`[llm].max_history`、`[tts].lang|speaker|speed`、音色探测凭据改读 live 快照。
7. **密钥打码曾在第一批有意延后，已在第二批完成**（见 §0.6）。延后原因保留于此便于复核：它必须**同批**上线服务端打码、"空串 = 保留原值"语义与客户端 presence 模型（前端 `xfyunCredsReady()` 会在打码后误判"没填凭据"而隐藏音色区）；半途上线会造成比 bug-1 更严重的"保存一次清空密钥"故障（§7 风险 1）。第一批只做了无此风险的 `revision` + 409。

### 0.6 第二批：密钥打码 + presence 模型（§3.4 的落地）

| 层 | 改动 |
|---|---|
| 服务端 | `gateway/ws/config.rs` 新增 `SECRET_PATHS`（5 条：`llm.api_key`、`tts.xfyun.api_key`/`api_secret`、`aiui.api_key`/`api_secret`）、`redact_secrets`（GET **一律摘除**密钥字段，只回 `has_<leaf>` presence）、`sanitize_patch`（空串密钥从补丁删除、presence/`revision` 包装字段剔除）、`merge_client_patch`（**`put_config` 与单测共用的唯一管线**——测试里原先复刻的 `simulate_put` 已合并，避免漂移） |
| 语义 | 留空/缺失 = **保留原值**；显式 `null` = 清除；非空 = 覆盖。旧客户端把空串回传**不再清空密钥**（这正是"必须同批上线"的原因） |
| 指纹 | `config_revision` 改为基于**打码后**内容：密钥**值**不参与（避免指纹成为密钥的比对依据），密钥**有无**（presence）参与。既有测试 `revision_tracks_content` 的"改密钥必须改指纹"断言已按新语义改写 |
| 测试台 | `/api/tts/voices/test-credentials` 对留空字段**逐项回落到盘上凭据**（新增 `tts_creds_raw`），否则打码后"不重填密钥就测不了凭据" |
| 客户端 | `ConfigFormState`：5 个 `*Configured` presence observable、`xfyunCredsReady()` 改为「已填 **或** 已配置」、`buildConfigJson` 用 `putSecret`（留空不发送）、`fill()` 读 `has_*`（旧服务端无该键时回落到"值非空"）；`ConfigCards` 密钥框占位提示「已配置（留空则不修改）」/「未配置」 |
| 回归护栏 | 新增 4 个单测：`get_redacts_secrets_and_reports_presence`（明文不得作为 **JSON 值**出现 + 5 条路径 presence + 打码后仍是合法 `Config`）、`blank_secret_keeps_existing_and_null_clears`、`revision_ignores_secret_values_but_tracks_presence`、`presence_and_revision_keys_never_persist` |

⚠️ **遗留（有意取舍）**：客户端**没有**"清空密钥"的 UI 动作（清除需显式 `null`，目前只能改 TOML 或直接调 API）。语音场景里"清空密钥"≈停用该能力，用错的代价大于收益，故不暴露。
8. **`voices`/`firmware` 只挂了描述符**：统一动作模型（`POST /api/plugins/{id}/actions/{name}`）、job 轮询契约未做，现有 `/api/tts/voices/*` 端点保持原样（兼容优先）。

### 本次验证口径（实际执行的命令与结果）

```bash
# 1) 单元测试（无 feature：含注册表/宿主/热切换决策/配置部分更新/密钥打码/用量/SSE 用量捕获）
~/.cargo/bin/cargo test --config 'source.ustc.registry="sparse+https://mirrors.ustc.edu.cn/crates.io-index/"'
#    => 113 passed; 0 failed; 3 ignored   （第一批 109 → 第二批 +4 个密钥语义单测）

# 2) sherpa feature：类型检查 + **真实原生库下跑测试**（本机需 brew install opus +
#    PATH/PKG_CONFIG_PATH，见[服务端指引](<../../server/AGENTS.md>)的验证小节（原 agents.md §5.6）；DOCS_RS=1 仅做类型检查、跳过原生库）
DOCS_RS=1 ~/.cargo/bin/cargo check --features sherpa --config 'source.ustc.registry="..."'
#    => Finished（0 error）
~/.cargo/bin/cargo test --features sherpa --config 'source.ustc.registry="..."'
#    => 113 passed; 0 failed（feature 门控分支的断言**真的跑到了**，不只是类型检查）

# 3) 管理页产物重建（证明 Kotlin 改动可编译）
cd client && ./gradlew :apps:h5App:publishWeb -PwebSourceMap=false
#    => BUILD SUCCESSFUL in 35s（nativevue2.js 已含 presence 提示与 putSecret 路径）
```

⚠️ **环境备注（fmt 口径，重要）**：本机 rustup toolchain **原先缺 `rustfmt` 组件**（`cargo fmt` 直接报
`'cargo-fmt' is not installed`）——即 **第一批报告里"`cargo fmt --check` 通过"无法复现**；第二批实施时
用 `rustup component add rustfmt` 装回（rustfmt 1.9.0）。装回后实测：**全 crate 有 30 个文件存在既有 fmt 漂移**
（`device/voices.rs` 单文件就约 1190 行重排）。
本批次**刻意不做全库格式化**：`cargo fmt` 曾一次性重排 29 个与本功能无关的文件，已用
`git checkout --` 回退，只保留本批次真正修改的文件。**新增/修改的代码本身 fmt-clean**（已逐文件核对：
rustfmt 输出不触碰新增行）。全库统一格式应作为**独立提交**单独评审。
> 补充（§0.7 文件规模整改）：该次整改对**被拆分的 8 个文件**执行了 `rustfmt`（属改动范围本身），
> 其余文件仍保持原样；全库统一格式依旧留作独立提交。

⚠️ **本地未执行**：`server/tests/mock_client.py` 端到端联调需要真实模型（`/data/models`，本机不存在）并需 `--features sherpa` **运行**；该步骤属容器/真机验收，本次未覆盖。

---

## 改造前插件架构盘点


## 1. 现状盘点（本仓库事实）

> ⚠️ **本节是"改造前"的盘点快照**，用于定位缺口与论证设计。其中多项**已在 §0.5 / §0.6 落地**
> （配置 `revision` + 409、密钥打码 + presence、LLM 三层超时/重试/用量、`/api/plugins`、
> `/api/config/schema`、统一热切换等）。判断"现在是什么状态"请看 §0.5/§0.6 与源码，**不要只看本节**。

### 1.1 能力清单：五个能力 + 两个"体制外"能力

| 能力 | trait | 工厂 | 配置段 | 实现 |
|---|---|---|---|---|
| ASR | `plugins/asr/mod.rs:20` `AsrEngine` | `build_asr` `asr/mod.rs:68` | `[asr]` `asr/mod.rs:25` | SenseVoice（`asr/sensevoice.rs`） |
| VAD | `plugins/vad/mod.rs:14` `VadEngine` | **无工厂**，`Engines::new_vad` 内 `cfg(feature)` 直连 `runtime/mod.rs:177-188` | `[vad]` `vad/mod.rs:22` | Silero /（非 sherpa）Mock |
| TTS | `plugins/tts/mod.rs:173` `TtsEngine` | `build_tts` `tts/mod.rs:227`、`build_tts_with_lang` `:237` | `[tts]` + `[tts.xfyun]` `tts/mod.rs:24,55` | Kokoro（本地）/ 讯飞 AIUI（远程） |
| LLM | `plugins/llm/mod.rs:114` `LlmProvider` | `build_llm` `llm/mod.rs:130` | `[llm]` `llm/mod.rs:13` | OpenAI 兼容 Responses API |
| 全链路 | `plugins/aiui/mod.rs:138` `FullChainEngine` | **无工厂**，会话内直接 `AiuiSession::new(...)`（`workflow/session/mod.rs:590-603`） | `[aiui]` `aiui/mod.rs:25` | AIUI 云闭环 |
| 发音人目录 | — | — | — | `device/voices.rs`（148 条候选池探测 + `/data/voices.json` 缓存），路由 `ws/mod.rs:69` |
| 固件托管 | — | — | — | `device/firmware.rs`（多目标存储/匹配/版本比较：`<dir>/<功能>/<型号>/<板型\|_>/`）+ `device/firmware/http.rs`（上传/清单/下载/删除四个端点），路由 `ws/mod.rs:68` |

**观察**：`voices` 与 `firmware` 是"有 UI、有状态、有动作"的完整能力，却**不在 `plugins/` 目录、不进 `Config` 结构、不进装配器**——它们是"体制外能力"。统一插件化的第一个收益就是把它们纳入同一套描述与状态上报。

**已经写下的承诺**：`plugins/mod.rs:1-10` 的模块注释明确宣称「新增厂商：在对应能力目录加实现文件、实现 trait、**工厂加一行注册**即可，会话编排与装配器不动」。这与 §1.3 的实测扩展步骤（4~5 处 + 客户端）**不一致**——统一插件化本质上就是**让这句承诺变成真的**。

### 1.7 ⚠️ 盘点过程中发现的两个真 bug（与本次重构无关，建议独立修复）

> 这两条不是"设计不够优雅"，而是**当前就在损害用户**的缺陷。放到最前面，因为它们比插件化重构更紧急。

#### bug-1（高危）：从管理页保存一次配置，会静默重置三个配置段

**证据链**：`ConfigFormState.kt:389-446` 的 `save()` 只 `put` 了六个段（server/audio/asr/vad/tts/llm），其中：

- `audio` **漏 `downlink_lead_ms`** → 服务端 serde default 把它重置为 **240**（`config.rs:140-142`）；
- `tts` **漏 `cache_entries`** → 重置为默认 **256**（`plugins/tts/mod.rs:52-53`）；
- **整个 `[aiui]` 段没有任何 `put`** → 重置为 `enabled=false` + 三要素清空（`config.rs:46-50`）。客户端 `commonMain` 全目录 grep `aiui` **零命中**，`fill()`（`:480-540`）也不读它。

而服务端 `PUT/POST /api/config` 是**整份 `Config` 反序列化后写回整份 TOML**（`ws/config.rs:82-98`），因此"漏传"等于"重置"。**触发条件**：任何用户（或任何一次前端保存）在管理页点一次保存 → 手工在 TOML 里设的 `downlink_lead_ms` 回 240、TTS 缓存被改回 256、AIUI 全链路被关掉。用户感受是"我明明设过，怎么又变回去了 / AIUI 突然不工作了"。

**讽刺的旁证**：同一段代码对 `llm.stream` 写了明确警告注释（*"漏传会被重置为 true，手工在 TOML 里设的 stream=false 经 UI 保存一次就会丢"*，`ConfigFormState.kt:440-442`）——**同一个坑当时只补了一个字段**。`ws/config.rs:237-347` 的往返回归测试也照抄了这份不完整的 JSON 形状，所以**测不出来**。

**修法（二选一，建议都做）**：

1. **后端兜底（必做，1 小时）**：`POST /api/config` 改为**部分更新语义**——先读现盘配置，用请求 JSON 覆盖出现的字段，未出现的保持原值（等价于 DSH 的 *"Path edits preserve fields a client did not receive"*）。这是**结构性修复**：以后新增配置段不会再被旧客户端清空；
2. **前端补齐 + 加护栏（必做）**：`fill()`/`save()` 补 `downlink_lead_ms` / `cache_entries` / `aiui.*`；并**新增一条回归测试**：以"完整 `Config` 的 JSON"为基准，断言 `fill → save` 往返后**逐字段相等**（不是只断言被手写的那几个字段）——这样下次漏字段会被测试直接打红。

#### bug-2（中危）：TTS 热切换对**设备正式会话**根本没生效，但三处文档都说生效了

**证据链**：`Engines::refresh_tts_from_disk`（`runtime/mod.rs:139-166`）的调用点**只有一处**：

```
server/src/workflow/session/bench.rs:107   ← 测试台 tts_test 专用
```

`workflow/session/mod.rs`（ESP 设备流水线）**完全不调用它**（grep `refresh` 在 `session.rs` 零命中），只通过 `engines.tts()`（`:459`）取当前引擎。也就是说：

- 管理页改 `[tts].backend` 保存后，**设备会话仍然用旧引擎，直到进程重启**；
- 只有跑过一次 macApp 测试台的 `tts_test`（它 `refresh` 了同一个进程内的 `RwLock`），后续设备会话才会"顺带"用上新引擎——这解释了为什么"有时候好像生效了"。

**文档三处与现实矛盾**：`runtime/mod.rs:17` 模块注释称「在会话开始 / 测试台 tts_test」、`AGENTS.md:77` 称「改后保存即热切换（新会话/测试台 tts_test 读盘重建引擎）」、`AGENTS.md:124` 称「管理页保存后无需重启」，而 `gateway/ws/mod.rs:22-23` 又说「引擎相关参数需重启」。**代码是唯一权威：只有 tts_test 生效。**

**修法**：在 `session.rs` 的会话开始处（与测试台同一位置语义）调用统一的 `host.refresh_from_disk()`——这正是 §3.2 规则 3 要做的事；修完后**三处文档统一改成"新会话生效"**，并补一条"改配置 → 起新会话 → 断言引擎名变化"的测试。

### 1.8 一致性缺口汇总（按危害排序）

| # | 缺口 | 证据 | 危害 |
|---|---|---|---|
| 1 | 表单漏字段 + 全量覆盖保存 → 静默重置配置 | 见 §1.7 bug-1 | **高**（正在损害用户） |
| 2 | 热切换只有 TTS 有骨架，且设备路径未接 | 见 §1.7 bug-2；ASR `runtime/mod.rs:39`、LLM `:47` 启动即冻结 | **高**（文档承诺与行为不符） |
| 3 | 运行期失败**全部静默**，四种写法 | ASR `session.rs:141`、TTS `:499`、LLM `:544`、AIUI `:176` 都是 `warn!`+continue；只有测试台 `tts_test`/`llm_test` 回报 UI（`bench.rs:242-267,303-323`） | **高**（用户只感知"没反应"） |
| 4 | `asr_test` 把错误**当识别文本**塞进 `stt.text`（`bench.rs:43-56`），与 tts/llm 的 `state:"error"` 语义不同 | 同上 | 中 |
| 5 | 测试台三条路径读盘策略不一致：`tts_test` 读盘热切换、`llm_test` 读盘临时构建、`asr_test` 完全不读盘 | `bench.rs:104-133` / `:287-299` / `:41` | 中 |
| 6 | 构造失败语义不一致：ASR/TTS/VAD/AIUI fail-fast；**LLM 永不失败**（`build_llm` 返回非 `Result`，`llm/mod.rs:130`）→ 密钥错延迟到调用期 | 同上 | 中 |
| 7 | `voices` 探测与讯飞**硬耦合**：`voices.rs:114-127` 直读 `tts.xfyun`、`:277-288` 手写凭据结构、148 条候选池硬编码在 app 层（`:321-470`） | 无法复用于第二个供应商 | 中 |
| 8 | 无 `/api/plugins`（能力清单+状态）、无 `/api/config/schema`（字段元数据）、无用量端点 | 路由表 `ws/mod.rs:57-70` | 中（UI 只能猜后端状态） |
| 9 | **AIUI 在管理页完全不可见**：`renderForm` 只有 6 个 tab（Server/Audio/ASR/VAD/TTS/LLM），`commonMain` 全目录 grep `aiui` **零命中** | `ConfigCards.kt:22-34` | 中（全链路模式只能手改 TOML，且见 bug-1 会被保存清空） |
| 10 | 能力之间无依赖联动：AIUI 启用时应提示/禁用本地 ASR/TTS 卡，UI 无此逻辑 | `engines.rs` 侧有校验，UI 侧无 | 中 |

### 1.2 装配：启动期一次性构建，只有 TTS 可热切换

- `Engines::new`（`runtime/mod.rs:58-87`）按固定顺序构建 asr/tts/llm，全部 `Arc<dyn Trait>` 共享；
- 启动期校验**只有两处硬编码**：VAD 模型路径非空（`runtime/mod.rs:60-63`）、AIUI 三要素（`runtime/mod.rs:65-71`）；
- 热切换**只有 TTS**：`tts_sig`（`runtime/mod.rs:43,74`）+ `refresh_tts_from_disk`（`:139-166`），签名比较 → 重建 → 失败保持旧引擎；
- **ASR / LLM 是启动即冻结**：`pub asr: Arc<dyn AsrEngine>`（`:39`）、`pub llm: Llm`（`:47`）都是不可变字段 → 改 `[llm].model` 必须重启。这是当前最刺眼的不一致。
- VAD 是**每会话对象**（`new_vad` `:177`），天然"每次新建"——它在语义上等价于 DSH 的 per-scope 实例，只是没被表达出来。

### 1.3 配置：约定存在但只写在注释里

`plugins/tts/mod.rs:18-24` 的注释是**当前唯一的扩展规范**（原文）：

> 扩展新远程供应商 = ① `[tts].backend` 加可选 id；② 新增 `[tts.<id>]` 凭据段 + serde 结构；③ `build_tts` 加分支 + 引擎实现；④ 客户端 `ConfigFormState` 的 `ttsRemoteEngines` 注册表追加一项 + 卡片字段区按 id 追加 vif。

它是**好设计的口头版**：本体字段=本地引擎参数、`[<cap>.<id>]`=远程凭据段。问题是：
- 只有 TTS 遵守（`[tts.xfyun]`）；ASR/LLM 无"实现 id"概念（`[asr].backend`/`[llm].backend` 不存在，未知键被 serde 静默忽略，见 `config.rs:10-11`）；
- 第 ④ 步把客户端也拖进扩展流程——**同一件事要改两个仓库**；
- 没有 `enabled` 开关，只有 AIUI 的 `[aiui].enabled` 一个特例（`config.rs:46-50`）。

### 1.4 HTTP 与 LLM 调用的缺口（最该借 DSH 的地方）

| 缺口 | 证据 | 后果 |
|---|---|---|
| LLM 请求**无超时** | `plugins/llm/openai.rs:82-85` 的 builder 只 `.user_agent(...)`，无 `.timeout(...)` | 上游挂起 → 会话永久等待，只能靠设备 abort |
| **无重试/退避/可重试错误分类** | `openai.rs` 全文件无 `retry/backoff/StatusCode` 分支 | 502/429/瞬时断连 = 直接失败，语音场景体感"AI 没反应" |
| **无 token 用量观测** | 全局无 token/meter 相关代码 | 成本不可见；也无法自证"记忆让 prompt 变长了多少" |
| system prompt 不可拆分 | `openai.rs:130` 用 `cfg.system_prompt` 单字符串；`LlmProvider::chat_stream` 签名（`llm/mod.rs:114-124`）只收 `history/user_text/tools/on_event` | 灵魂/记忆只能往一个字符串里拼（见 `soul-and-graph-memory-plan.md` §4/§5） |
| 配置**无 revision、密钥明文往返** | `ws/config.rs` 无 revision/redact；`LlmConfig.api_key`（`llm/mod.rs:18`）被 `GET /api/config` 明文序列化；客户端再明文回传（`ConfigFormState.kt:437,533`） | 多标签页互相覆盖；密钥出现在浏览器/日志里 |

### 1.5 路由面：能力状态无处可查

`ws/mod.rs:57-70` 的 API 只有 health / ws / config / config-meta / ota / firmware / voices。**没有** `/api/plugins`（能力清单+状态）、`/api/config/schema`（字段元数据）、任何"用量"端点。管理页只能靠"保存后能不能用"猜后端状态。

### 1.6 已经接近 DSH 理念、值得保留推广的底子

| 现有机制 | 证据 | 为什么它是对的 |
|---|---|---|
| `plugins/` 按能力分目录 + 工厂一行注册 | `plugins/{asr,tts,llm,vad,aiui}/` | 与 DSH「一个能力一个包」同构，只差"注册表"这一层 |
| 引擎签名热切换 | `runtime/mod.rs:139-166` | 已实现 DSH 的 `signature → 重建 → 失败保留旧实例`，且**失败不致命**（正是我们想要的语义） |
| 每会话 VAD 工厂 | `runtime/mod.rs:177-188` | per-scope 实例的正确形态 |
| 能力状态字段 + 轮询交互 | `GET /api/tts/voices` 返回 `probing`（`voices.rs:225`）、`POST /api/tts/voices/refresh`（`:232`）触发后台探测，`PROBING` 单飞守卫（`:62,134-136`） | 已经是一套"长任务 + 状态轮询 + 单飞"契约，可泛化为统一 action 模型 |
| 目录是从**真实探测**得出的能力清单 | `voices.rs:62` 起的候选池逐个调 API 验证，通过才进目录 | 与 DSH「清单即能力声明」同源思想：**能力面应该被真实枚举出来**，而不是写死在 UI 里 |
| 流式契约 + 取消语义 | `TtsChunkCallback`（`tts/mod.rs:167-182`）：引擎只产增量 PCM，取消由回调返回 `false` 表达 | 新供应商只要遵守契约就自动获得低 TTFA；下游 pacing/编码与引擎解耦 |
| `blocking_result` 统一包装 | `workflow/session/mod.rs:30-43`：把取消/panic/内部错误收敛成 `Err(String)` | 引擎只需是同步 trait，无需 async-trait，错误面统一 |
| 派生缓存挂能力身份 | `tts_cache.rs:50-65` + `session.rs:433-441`：cache key 含引擎签名 | 换引擎自动失效，不需要手工清理逻辑 |
| UI 声明式页面注册表 | `AdminComponents.kt:129-166`（`TabUiState/TabPage/tabbedPanel`）+ `ConfigCards.kt:22-34` | 新增一个 tab 只改一处列表——服务端补上 schema 后就能由数据驱动 |
| 组件层与 Toast 已收口 | `AdminFormControls.kt:54-124`、`AdminWidgets.kt:45-161`、`ToastHost.kt:11-45` | UI 差异只剩"用哪些官方控件"，不需要各写一套样式 |
| 契约可测 | `ws/config.rs:225-373`：JSON→Config→TOML→Config 全链路回归 | 扩 schema 时有现成护栏（⚠️ 但需按 bug-1 补全字段与断言） |
| 配置原子写 + 单文件挂载回退 | `ws/config.rs:47-60` 注释与实现 | 生产环境真问题（`EBUSY`）已被解决，别动 |
| 显式"配置来源"端点 | `/api/config/meta` `ws/mod.rs:63` | 与 DSH 的 `profile`/`bundles` 溯源同思路 |
| `ToolSpec/ToolCall` 已预留 | `llm/mod.rs:68-110` | 借 `gm_search` 等工具时无需新协议 |

---

## 插件统一实施计划与风险


## 6. 分期实施计划

> 原则：**先止血、再统一、后增强**；每一步都能独立上线与回滚。

### P0 — 止血（0.5~1 人日，**建议立刻做，与插件化无关**）

1. `POST /api/config` 改**部分更新语义**（缺失字段保持原值）——修 §1.7 bug-1；
2. 前端 `fill()`/`save()` 补 `downlink_lead_ms` / `cache_entries` / `aiui.*`；
3. 往返测试改为**逐字段全量断言**（以完整 `Config` 为基准，而不是复制手写形状）；
4. 会话开始处接上 TTS 热刷新（或至少把 §1.7 bug-2 涉及的 3 处文档改成"需重启"）——消除文档与行为矛盾。

验收：管理页保存任意配置后，手工在 TOML 里设的 `downlink_lead_ms=cache_entries=aiui.*` **保持不变**；`cargo test` 全绿。

### P1 — 契约与注册表（2~3 人日）

`plugins/registry/`（`PluginDescriptor`/`Capability`/`FieldSchema`/`HotReload` 常量表）、`plugins/host.rs`（`validate`/`boot`/`snapshot`）、`GET /api/plugins`、`GET /api/config/schema`、required 校验前移与错误文案升级、`Engines::new` 改经 `PluginHost`。
**行为不变**（只报告，不接管热切换）。验收：`/api/plugins` 能列出全部能力 + 状态；`cargo test` 含"注册表 id 唯一 / 每个能力至少一个实现 / 默认 engine 存在"三条单测。

### P2 — 统一热切换（2 人日）

`llm: RwLock<Llm>`（对齐既有 `tts: RwLock` 模式）；`refresh_from_disk` 覆盖 tts/llm/记忆；`HotReload` 三档上报；失败保留旧实例 + 记 `last_error`；会话开始调用。
验收：改 `[llm].model` 保存后**新会话用新模型**（加断言测试）；保存一个坏配置 → 服务不受影响、`/api/plugins` 显示异常原因。

### P3 — LLM 护栏（2~3 人日）

`LlmFailure` 枚举 + 超时（connect/stream-idle）+ 重试装饰器（normal only、总预算 ≈8s、已出声不重试）+ 空回复判定；`TokenUsage` 从 Responses API usage 取值 + 每会话计量 + `GET /api/session/{id}/usage`；LLM 卡加「高级」折叠区 + 「测试连接/拉取模型」动作 + 密钥打码 + revision 409。
验收：拔网/造 429 场景下**用户仍能听到兜底话术**而非静默；用量端点能回答"记忆注入让 prompt 长了多少 token"。

### P4 — 配置约定统一 + 体制外能力纳管（2~3 人日）

`engine` 键 + `[<cap>.<id>]` 私有段 + `normalize()` 双读；`voices`/`firmware` 纳入注册表与统一 action 模型（**只做描述与状态，不重写业务逻辑**）。
验收：旧 `config.toml` 原样可读；新增一个 TTS 供应商的改动点从 19 处降到 ≤5 处（对照 §3.6）。

### P5 — UI 重组（3~4 人日）✅ 已完成（主体）

能力总览页 + 卡片规范 + 状态徽标（含原因）+ 三档生效文案 + 动作组件 + `AdminTheme` 常量补齐 + AIUI 卡。
验收：浏览器（web 产物）与 macApp 双端可见；`publishWeb` 产物可跑；状态/降级原因与实际后端一致。
落地清单与 5 条偏差见 **§0.11**。

### P6 — 灵魂 / 记忆接入（依赖 `soul-and-graph-memory-plan.md`）✅ 已完成

`PromptSection` 注册表（§4.4）+ `[soul]`/`[memory]` 作为段提供者与可选插件（`Phase::Degraded` 而非致命）。
落地清单与 9 条偏差见 **§0.9**（含"未做 embedding/PPR/三元组（P2）、`profile_path` 未做、协议未改"）。

**合计 ≈ 12~18 人日**（不含真机联调排队）。**若只做 P0，也能拿到"不再静默丢配置"这一条最大收益。**

> P4 的落地情况与偏差见 §0.10（配置约定已落地；统一动作模型未做）。
**当前进度**：**P0~P6 全部完成**（§0.5 状态快照；P5 见 §0.11）。剩余：P4/P5 共同欠着的**统一动作模型**（`POST /api/plugins/{id}/actions/{name}` + job 轮询），以及 §5.4-2 的「已覆盖/恢复默认」字段语义。

---

## 7. 风险与陷阱（按严重度）

1. **密钥打码与旧客户端不兼容（高危）**：一旦 `GET /api/config` 不再回传 `api_key`，旧客户端会把空串回传。→ **必须同批上线"空字符串 = 未提供 = 保持原值"**（§5.4 第 4 条），否则用户保存一次就把密钥清空——这会造成比 bug-1 更严重的故障。
2. **部分更新语义要防"永远删不掉"**：缺失=保持 之后，如何清空一个可选字段？→ 约定：**显式 `null` 表示清除，缺失表示保持**；`serde` 侧用 `Option<Option<T>>` 或在 JSON 层判定 `containsKey`（`ws/config.rs` 当前直接 `Json<Config>`，需改为先取 `serde_json::Value` 再逐段合并）。
3. **注册表用枚举而非泛型**：`Built` 必须是闭枚举（§3.1），否则退化成 `Box<dyn Any>` + 运行时 downcast panic。→ 用"每个 descriptor 的 `build` 单测"覆盖全部 id。
4. **热切换失败必须保留旧实例**：否则"保存一个坏配置 → 语音服务不可用"。既有 TTS 范式已正确（`runtime/mod.rs:152-165`），统一时**不要**改成"先拆后建"。
5. **配置搬家（P4）是唯一破坏性改动**：`[tts].model → [tts.kokoro].model` 双读 + 保存写新结构；必须写迁移日志，并保留"未知键静默忽略"的既有行为（`config.rs:10-11`），否则旧配置直接解析失败。→ **已按此落地（§0.10）**：双读 + 迁移日志 + `rewrite_legacy_paths` 重写旧客户端补丁 + 未知键行为不变；风险点从"待处理"变为"有回归测试守着"。
6. **锁与阻塞**：`RwLock<Llm>` 每请求一次读锁（可忽略）；但 `refresh_from_disk` 可能加载模型（TTS 数秒）→ **必须在 `spawn_blocking` 内**（现状已如此：`bench.rs:107`）；且不要持有写锁做 IO。
7. **不要为了统一而统一**：`voices`/`firmware` 只纳入"描述 + 状态 + 动作"，**不重写其探测/托管逻辑**；`[aiui]` 的 `FullChainEngine` 也不要强行塞进 `Engines` 共享池（它是每会话有状态长连接）。
8. **文档漂移是本仓库的既有病**：本次盘点发现 3 处文档与代码矛盾（§1.7 bug-2）。→ 落地时**把文档断言变成测试断言**（引擎名断言、字段往返全量断言），否则修好一次还会再漂。
9. **协议与前端联动**：本次改动**不触碰** `protocol.rs` 的消息枚举。若 P3 要新增 bench 消息，必须遵守[协议说明](<../../server/src/workflow/protocol/protocol/AGENTS.md>)的 serde 命名约定（原 `agents.md` §5.1），且**`cargo check` 发现不了**，必须跑 `mock_client.py`。
10. **构建验证**：`cargo` 命令必须带 USTC 镜像覆盖且用 `~/.cargo/bin/cargo`（依赖与镜像覆盖见[服务端指引](<../../server/AGENTS.md>)，原 `agents.md` §3）；客户端改动必须 `cd client && ./gradlew :apps:h5App:publishWeb` 才能在本 GUI/浏览器里看到。

---

## 8. 附：文件级改动清单

### 后端（`server/src/`）

| 文件 | 改动 |
|---|---|
| `plugins/registry/`（新） | `mod.rs`：`PluginDescriptor`/`Capability`/`FieldSchema`/`HotReload`/`Requirement`/`Built` + `REGISTRY` + 查找/JSON；`fields.rs`：字段元数据表；`impls.rs`：选择/签名/校验/构建 + 描述符常量；`tests.rs`：id 唯一性 + 字段漂移护栏 + schema 形状 |
| `plugins/host.rs`（新） | `PluginHost::{boot, validate, refresh_from_disk, snapshot}` + disposer 登记 |
| `plugins/mod.rs` | `pub mod registry; pub mod host;` + 文档注释更新（把 §1 的"承诺"改成真实规则） |
| `plugins/llm/mod.rs` | `engine`/`[llm.openai]` 段 + `LlmFailure` + `PromptSection` 注册表 + `TokenUsage` |
| `plugins/llm/openai.rs` | `:82-85` 加 `connect_timeout`/`stream_idle_timeout`；`:163-166` 的错误改为结构化 `LlmFailure`（不再拼字符串）；解析 usage |
| `plugins/llm/retry.rs`（新） | 重试装饰器（`RetryingLlm(Llm)`） |
| `plugins/llm/prompt.rs`（新） | `PromptSection` + `assemble()`（升序 + 同 order 按名 tie-break + 空段丢弃 + `\n\n` 连接 + 未知变量启动失败） |
| `plugins/tts/mod.rs` | `engine` 键（`backend` 保留 alias）；Kokoro 参数迁 `[tts.kokoro]` + `normalize()`；`signature` 挂到 descriptor |
| `plugins/asr/mod.rs` / `vad/mod.rs` / `aiui/mod.rs` | 补 `engine` 键与 descriptor；VAD 补工厂；AIUI 补 descriptor（不改 trait） |
| `runtime/mod.rs` | `llm: RwLock<Llm>`；`tts_sig` 由注册表 `signature` 取代；`Engine s::new` 经 `PluginHost`；校验前移 |
| `config.rs` | `Config` 段不变（保持 `soul`/`memory` 扩展位）；`normalize()` 调用点 |
| `gateway/ws/mod.rs` | 路由加 `/api/plugins`、`/api/config/schema`、`/api/session/{id}/usage` |
| `gateway/ws/plugins.rs`（新） | `/api/plugins`、action 触发与 job 轮询 |
| `gateway/ws/config.rs` | **部分更新语义**（缺失=保持、`null`=清除）+ 密钥打码 + `revision` 409 + 全量字段往返测试 |
| `workflow/session/mod.rs` | 会话开始调用 `host.refresh_from_disk()`；失败上报统一（不再只有 `warn!`）；`max_history` → token 预算 |
| `workflow/session/bench.rs` | 三条测试路径统一走 action/`refresh_from_disk`；`asr_test` 错误语义与 tts/llm 对齐 |
| `device/voices.rs` | 探测纳入 action 模型（保留旧端点做兼容） |
| `device/firmware.rs` | 纳入注册表（只加 descriptor + 状态） |
| `config.example.toml` | `engine`/`[<cap>.<id>]`/`[llm.retry]`/`[llm.context]`/`[soul]`/`[memory]` 示例与注释 |

### 前端（`client/shared/src/`）

| 文件 | 改动 |
|---|---|
| `commonMain/.../admin/ConfigFormState.kt` | 补 `downlink_lead_ms`/`cache_entries`/`aiui.*`；`save()` 改**部分更新**（只发变更字段）；`revision` 字段与 409 处理；密钥改 presence 模型 |
| `commonMain/.../admin/ConfigCards.kt` | `:22-34` 加「总览/AIUI/记忆/灵魂」；各卡按 §5.3/5.4 规范重排（高级折叠区、hint 讲后果、状态带原因） |
| `commonMain/.../admin/CapabilityOverview.kt`（新） | 能力总览页（计数摘要 + 分组卡片 + 状态徽标 + 动作） |
| `commonMain/.../admin/CapabilityDetail.kt`（新） | 单能力详情（`tabbedPanel` 配置/状态/日志） |
| `commonMain/.../admin/SectionRegistry.kt`（新） | `SectionDescriptor` 常量表 + 构建期校验 |
| `commonMain/.../admin/AdminTheme.kt` | 补 §5.8 的间距/字号/圆角/状态点常量 |
| `macosArm64Main/.../admin/AdminShell.kt` | 侧栏项改为读 `SectionRegistry` |
| `macosArm64Main/.../admin/TestBenchPages.kt` | 测试台动作与 action 模型对齐；状态/失败原因显示同规范 |

### 文档（本仓库）

| 文件 | 改动 |
|---|---|
| `AGENTS.md` §5.2b/§4 | 修正 TTS 热切换表述（→「新会话生效」，修完 bug-2 后成立）；补"密钥不回显"约定 |
| `docs/architecture.md` | 增「插件化架构」一节，指向本文件 |
| `docs/designs/soul-and-graph-memory-plan.md` | §5.4 的记忆注入位置改为 `after_history`（默认）+ 开关；`recall_max_chars` → token 预算 |
| 本文件 | 落地过程中把"设计"改写成"已实现"，并记录与计划的偏差 |

---

## 9. 一句话回答「怎么统一插件化 + 借 DSH」

> **统一** = 三张表：能力描述符注册表（可枚举 + 自带 `signature`/`build`/`fields`）、`enabled` 与 `phase` 分离的生命周期、`engine` + `[<cap>.<id>]` 的配置约定；五个引擎的 trait 一行不改，分阶段迁移、随时回滚。
> **借 DSH** = 借它的**护栏**（具名失败分类 + 有界退避 + `Retry-After`、有序 prompt 段与前缀稳定性、token/压力分解、密钥只存引用 + 表单只报 presence、revision 拒绝过期写入、单调 guard 与"拒绝也是工具结果"），**不借它的编排**（agent loop / subagent / PTC / 审批 / web search / `always` 重试）。
> **UI** = 借信息架构（写页与读页分工、声明式有序注册、只在偏离时打标、草稿 + 单一保存点、三档生效文案、AlertDialog 两段式强确认），**不借视觉实现**（modal 面板、hover/tooltip、自绘指示器与 loader、`color-mix` 动态淡色、运行时槽位机制）。
> **顺带**：盘点过程中发现两个真 bug（保存即静默重置三个配置段；TTS 热切换对设备会话根本没生效）——**它们比重构更紧急，建议先修**。
