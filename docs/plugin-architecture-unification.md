# 统一插件化架构 —— 借鉴 DSH 插件理念 + 适合本项目的 LLM 插件 + 管理页 UI 设计

> 状态：**设计（未实现）**。所有「现状」结论均给出本仓库 `文件:行号`；所有 DSH 结论均给出 asar 内包名+文件
> （取证方式见 [`soul-and-graph-memory-plan.md`](./soul-and-graph-memory-plan.md) 文首说明：`app.asar` 是归档文件不是目录）。
> 目标读者：要动手实现这件事的工程师（含 AI 编码助手）。
> 关联：`AGENTS.md`（陷阱速查）、`docs/architecture.md`、`docs/soul-and-graph-memory-plan.md`（灵魂/记忆，本文件的 P4 依赖它）。

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
| 服务端 | `app/ws/config.rs` 新增 `SECRET_PATHS`（5 条：`llm.api_key`、`tts.xfyun.api_key`/`api_secret`、`aiui.api_key`/`api_secret`）、`redact_secrets`（GET **一律摘除**密钥字段，只回 `has_<leaf>` presence）、`sanitize_patch`（空串密钥从补丁删除、presence/`revision` 包装字段剔除）、`merge_client_patch`（**`put_config` 与单测共用的唯一管线**——测试里原先复刻的 `simulate_put` 已合并，避免漂移） |
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
#    PATH/PKG_CONFIG_PATH，见 agents.md §5.6；DOCS_RS=1 仅做类型检查、跳过原生库）
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
（`app/voices.rs` 单文件就约 1190 行重排）。
本批次**刻意不做全库格式化**：`cargo fmt` 曾一次性重排 29 个与本功能无关的文件，已用
`git checkout --` 回退，只保留本批次真正修改的文件。**新增/修改的代码本身 fmt-clean**（已逐文件核对：
rustfmt 输出不触碰新增行）。全库统一格式应作为**独立提交**单独评审。
> 补充（§0.7 文件规模整改）：该次整改对**被拆分的 8 个文件**执行了 `rustfmt`（属改动范围本身），
> 其余文件仍保持原样；全库统一格式依旧留作独立提交。

⚠️ **本地未执行**：`server/tests/mock_client.py` 端到端联调需要真实模型（`/data/models`，本机不存在）并需 `--features sherpa` **运行**；该步骤属容器/真机验收，本次未覆盖。

---

### 0.7 第三批：文件规模整改（逻辑文件 ≤ 600 行）

**背景**：新增规范 `AGENTS.md` §5.10 —— 任何 `.rs`/`.kt` 逻辑文件 ≤ **600 行**。审计发现 4 个文件超标，
其中 2 个（`registry.rs`、`ConfigFormState.kt`）是前几批自己写大的。

| 整改前 | 行数 | 整改后 | 行数 |
|---|---|---|---|
| `plugins/registry.rs` | 989 | `registry/{mod,fields,impls,tests}.rs` | 399 / 450 / 314 / 212 |
| `app/ws/config.rs`（逻辑 422 + 测试 553） | 975 | `config.rs` + `config/tests.rs` | 424 / 550 |
| `app/session.rs`（逻辑 695） | 733 | `session.rs` + `session/reply.rs` | 443 / 310 |
| `ConfigFormState.kt` | 652 | `ConfigFormState.kt` + `ConfigFormFill.kt` | 548 / 116 |

**拆分口径**：按**职责**而非按行数均分——注册表拆成「类型/注册表/JSON」「字段表」「实现接线」「测试」；
配置端点把 550 行测试外移；会话把「回复生成与下发」（`stream_response` + 两个 `speak_*`）独立；
Kotlin 把「服务端 JSON ⇄ 表单映射」（`fill` + `putSecret`）独立。

**可见性代价（仅 2 处，均为最小放宽）**：
- Rust：`session/reply.rs` 的 `stream_response` 标 `pub(super)`——**父模块看不到子模块私有项**，
  而调用方在父模块的消息循环里（子模块访问父模块私有项不受影响，故其余全部保持私有）。
- Kotlin：`ConfigFormState.deriveVoiceGroupAndType` 由 `private` → `internal`（`fill` 依赖它，
  同模块可见即可）。`putSecret` 同步为 `internal`（`buildConfigJson` 仍在原文件调用）。

**验证（行为不变的证据）**：

```bash
~/.cargo/bin/cargo test                                                    # => 113 passed; 0 failed; 3 ignored（与拆分前同数同结果）
~/.cargo/bin/cargo test --features sherpa                                  # => 113 passed; 0 failed
cd client && ./gradlew :apps:h5App:publishWeb -PwebSourceMap=false         # => BUILD SUCCESSFUL in 34s
find server/src client/shared \( -name '*.rs' -o -name '*.kt' \) -exec wc -l {} + | awk '$1>600 && $2!="total"'   # => 空
```

**过程记录（避免后人重踩）**：拆分用脚本按行区间搬移，工具链**不认行为等价**，因此脚本里对每个
切点都写了 `assert` 锚点校验；仍出现两类边界缺陷并已修复——① `stream_response` 的**文档注释**被留在原文件
（函数体走了、doc 没走，rustdoc 直接报 `expected item after doc comment`）；② 测试外移时多带了一层
`mod tests {` 包裹（未闭合分隔符）。**结论：脚本化搬移必须配锚点断言 + 全量编译，二者缺一不可。**

### 0.8 第四批：测试与逻辑彻底分离（`AGENTS.md` §5.10）

**背景**：§5.10 原口径是"内联测试**超过 ~150 行**才外移"。按项目要求升级为**测试一律独立文件（无例外）**——
把"测试与逻辑同文件"本身当成缺陷，即使只有 1 个断言也不内联。

**范围**：审计 `server/src` 全部 `.rs` 中 `#[cfg(test)]` 的用法——**19 个内联块 + 1 个空块**，另有 2 个已外移。

| 处置 | 数量 | 说明 |
|---|---|---|
| 内联测试外移 | 19 | `foo.rs` → `foo/tests.rs`；`foo/mod.rs` → `foo/tests.rs`；外层只留 `#[cfg(test)] mod tests;` |
| 空测试块删除 | 1 | `app/session.rs` 的 `mod tests {}`（零断言占位），连同多余空行一并删除 |
| 已合规 | 2 | `app/ws/config/tests.rs`、`plugins/registry/tests.rs`（第三批已外移） |

**规模对比**（行数取本批开始前的实测值；"测试"含外移后新增的 `//!` 文件头）：

| 文件 | 前（逻辑+测试） | 后（逻辑 / 测试） |
|---|---|---|
| `app/downlink.rs` | 591 | 331 / 264 |
| `app/voices.rs` | 576 | 508 / 72 |
| `plugins/aiui/mod.rs` | 498 | 335 / 167 |
| `plugins/host.rs` | 490 | 338 / 167 |
| `app/protocol.rs` | 441 | 341 / 103 |
| `engine.rs` | 433 | 340 / 101 |
| 其余 13 个 | 均 ≤ 356 | 均 ≤ 340 |
| **合计** | **6022** | **4326 / 1805** |

**验证（行为不变）**：

```bash
~/.cargo/bin/cargo test                       # => 113 passed; 0 failed; 3 ignored（与搬移前同数同结果）
~/.cargo/bin/cargo test --features sherpa     # => 113 passed; 0 failed; 3 ignored
grep -rnE 'mod[[:space:]]+tests[[:space:]]*\{' server/src --include=*.rs   # => 空（0 个内联块）
find server/src -name '*.rs' -exec wc -l {} + | awk '$1>600 && $2!="total"'  # => 空（0 个超标文件）
# ⚠️ 不能写 `xargs wc -l | awk '$1>600'`：wc 的 `total` 行（万级常量）会被误判为超标文件
find server/src -name tests.rs | wc -l                                     # => 21
```

**格式漂移的归属（逐文件证明不是本次引入的）**：新测试文件里仍有 rustfmt 漂移（`plugins/aiui/tests.rs` 8 处、
`app/downlink/tests.rs` 7 处等）。用 `git show HEAD:<file>` 重建"原始内联块"后逐文件对比漂移计数：
**14 个文件搬移前后完全一致**（7→7、4→4、3→3…，另有 `ws/config.rs` 由 3→0，因该测试文件第三批已格式化），
证明搬移是**纯位移**；剩下 5 个测试块是本项目前几批**新写的代码**，按"新增代码保持 fmt-clean"的既定口径
已对其执行 `rustfmt`。全库统一格式仍留作独立提交（本 crate 约 30 个文件有既有漂移，本批刻意不动）。

**过程记录（脚本化抽离的两个必备判定）**：
1. **不要用大括号计数找测试块结尾**：`firmware.rs` 里的字符串 `"{broken"` 会让 `{`/`}` 计数失衡
   （实测直接报 `unmatched braces` 并中断）。改用"**最后一个非空行必须是顶格 `}`，且其后无代码**"后一次通过。
2. `#[cfg(test)]` 之后可能是三种形态：`mod tests {`（待外移）、`mod tests;`（已外移）、单行 `mod tests {}`（空块）——
   三者都要单独识别，否则前者会被误报为"异常"（实测把 2 个已合规文件报成错误）。
   脚本先**全量分析、零异常才写入**，避免改到一半留下坏状态。

---

### 0.9 第五批：P6 灵魂 / 记忆接入（`soul-and-graph-memory-plan.md` 的 P0+P1）

> 设计与取舍的完整讨论见 [`soul-and-graph-memory-plan.md`](./soul-and-graph-memory-plan.md)（该文档已同步标注本次落地状态与偏差）。

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

### 0.10 第六批：P4 配置约定统一（本文件 §3.3 的落地）

> 这是本设计**唯一破坏性**的部分（§7 风险 5），因此实现上贯彻"**只增不删 + 双读 + 补丁重写**"。

**三条约定**（`server/src/config.rs` 模块注释为权威）：

| 约定 | 形式 | 为什么 |
|---|---|---|
| ① `engine` 键选实现 | `[tts] engine = "kokoro"` ↔ 描述符 `tts.kokoro`（id 前缀即能力） | 让"哪个实现被选中"成为**可枚举/可校验**的配置事实，而不是散落在代码分支里 |
| ② 私有字段进子段 | `[tts.kokoro]` / `[tts.xfyun]` / `[memory.extractor]` | 新增供应商不再需要往能力本体加字段（本体只留跨实现通用项） |
| ③ 跨实现参数留本体 | `[tts] speed/speaker/cache_entries` | 判定规则**可机械执行**：被 `Trait::method` 当入参用的留在本体，其余进私有段 |

**落地清单**：

| 关注点 | 落地形态 |
|---|---|
| 规范化 | `Config::normalize()`（幂等）在**三个入口**调用：`load()` / `Default` / `merge_client_patch()`；`persist_config()` 再兜一次（防手搓 `Config` 直接落盘写成空 `engine`） |
| 旧写法双读 | `[tts] backend="sherpa"` → `engine="kokoro"`（`EngineSpec.aliases`）；`[tts].model/voices/tokens/data_dir/dict_dir/lexicon/lang/num_threads` → `[tts.kokoro].*`（旧字段 `#[serde(default, skip_serializing)]`，**保存只写新结构**）；`[memory].backend` → `[memory].engine`（serde `alias`） |
| 新位置优先 | 同一字段两处并存时新位置胜出（避免旧键覆盖新键）；与默认值等价的旧值不产生迁移噪音 |
| 旧客户端补丁 | `app/ws/config.rs::rewrite_legacy_paths`（在 `sanitize_patch` 内、merge 之前）：`tts.backend→engine`、`tts.<旧扁平字段>→tts.kokoro.<字段>`、`memory.backend→engine`；**同一补丁里显式的新位置优先**。没有这一步，旧 macApp 改 `model` 会被"新位置优先"静默丢弃 |
| 未知值不静默回退 | `registry::{unknown_engine_of, unknown_engine_hint}`：`PluginHost::plan` 把该能力全部实现标 `Failed` + 可选值原因；`boot` 汇总报错；`decide_refresh` 返回 `Err`（否则保存一个新 engine 值后热刷新会静默保留旧引擎） |
| 去中心化 | **删除** `TtsBackendKind` 枚举、`TtsConfig::engine_signature()`、`build_tts()` 中心分发；选择走描述符 `selected`、构建走描述符 `build`、签名走描述符 `signature`（`engine.rs::refresh_tts` 改为按描述符构建） |
| `asr`/`vad`/`llm`/`memory` | 也补上 `engine` 键（`sensevoice`/`silero`/`openai`/`graph|none`）；`<CAP>_ENGINES` 表是配置层权威，`config/tests.rs` 守"声明 id ↔ 注册表实现"一致 |
| 字段元数据 | `registry/fields.rs`：`engine` 进入各能力**公共字段**（含 enum 可选值），TTS 的 `speed`/`speaker` 从 Kokoro 私有表移入公共表（与约定③一致）；漂移护栏测试当场抓到过这次不一致（`tts.kokoro 的字段 speaker 在 Config 路径中不存在`） |
| 客户端 | `[tts]` 写 `engine` + `kokoro` 子段；`fill()` 读 `engine`（回落 `backend`）与 `kokoro`（回落旧扁平位置）；本地引擎 id 统一为 `kokoro`（`canonicalTtsEngine()` 兼容旧值 `sherpa`）；记忆卡读 `engine`、状态卡显示"引擎" |
| 样例 | `config.example.toml` 改为新约定（`engine` 键 + `[tts.kokoro]`），并加"旧写法仍可读"的注释；新增测试 `example_config_parses_and_normalizes` 守住"样例必须能解析" |

**偏差（逐条）**：

1. **`llm.openai` / `asr.sensevoice` / `vad.silero` 的字段仍留在本体**（设计 §3.3 的示例把 LLM 字段画进了 `[llm.openai]`）：这三个能力目前**只有一个实现且无第二实现计划**，搬家的收益为零，却要动 `SECRET_PATHS` 路径、`[memory.extractor]` 的逐字段覆盖以及约 30 处测试。按 §7 风险 7「不要为了统一而统一」，本次只把**机制**（子结构 + 旧字段只读 + `normalize` 双读 + 补丁重写）在 TTS 上完整落地并验证——它正是验收指标（新增 TTS 供应商成本）所指的能力。出现第二个 LLM/ASR 实现时按 TTS 同一套搬即可（机制与先例已在）。
2. **`[memory].engine = "none"` 不是实现 id**：它是"显式停用"的哨兵值（保留配置但不生效），与 `enabled=false` 等价。`unknown_engine_of` 对它显式放行，否则会把合法配置报成"未知实现"。
3. **`speed`/`speaker` 留在 `[tts]` 本体而非 `[tts.kokoro]`**：它们是 `TtsEngine::synthesize_stream` 的入参（讯飞也吃 `speed`），属约定③。原先打算放进 Kokoro 子段，实施中按"可机械执行的判定规则"纠正，并同步了字段元数据表。
4. **AIUI 用 `enabled` 而非 `engine`**：`[aiui]` 是"整条流水线的模式开关"（能力 `fullchain`），不是多实现互斥选择，保留布尔开关更直白；描述符 id 仍是 `fullchain.aiui`（配置段名为历史的 `[aiui]`，`config/tests.rs` 里作为**唯一例外**显式列出）。
5. **`sherpa` 成为 `kokoro` 的别名而非新 id**：`engine="sherpa"`（旧 `backend` 取值）与 `engine="kokoro"` 等价；好处是 `backend = "sherpa"` 的老配置零改动，且 `engine` 的值空间与描述符 id 严格对应。
6. **`TtsConfig.engine` 默认为空字符串**（不是 `"kokoro"`）：空 = 未指定，才能让"文件里只有旧 `backend`"这条路径走别名解析；`normalize()` 负责补全，因此 GET `/api/config` 与落盘文件永远是具体 id。
7. **`GET /api/memory/status` 的 `backend` 键改名为 `engine`**（客户端同步）：一个概念不留两个名字；旧字段值本来就是 `graph`/`none`，语义没变。
8. **未做统一动作模型**（设计 §3.4 的 `POST /api/plugins/{id}/actions/{name}` + job 轮询）：P4 的另一半（`voices.probe` / `tts.test` / `llm.test` / `memory.*` 归一到一套"动作 + 状态 + 结果"契约）本质是**新增端点与契约**，与配置约定无耦合，且需要真机验证探测/合成的长任务语义。本次先把配置约定（唯一破坏性部分）做完并单独验证。

**验收证据**（P4 的两条验收指标）：

| 验收项 | 证据 |
|---|---|
| 旧 `config.toml` 原样可读 | `full_test_config()` 的 fixture **刻意写成旧位置形态**（`[tts] backend/model/…`、`[memory] backend`），`legacy_toml_fixture_migrates_to_convention` 断言迁移到规范位置且**序列化后旧键消失**；`legacy_config_with_listen_key_parses` 覆盖"未知键静默忽略 + 旧 `[tts]` 扁平写法"；`raw_legacy_toml` fixture 同时驱动**全部既有往返/部分更新测试**（说明旧写法不只是"能解析"，而是全链路可用） |
| 旧客户端补丁不丢改动 | `legacy_client_form_shape_no_longer_resets_omitted_sections`：补丁写 `tts.backend` + `tts.model=/m/k2.onnx`，断言 `engine=xfyun` **且** `kokoro.model` 真的变成 k2（不是被"新位置优先"丢弃） |
| 新增 TTS 供应商成本 | 服务端需要改的只剩：① 新实现文件；② `plugins/tts/mod.rs` 的凭据/参数结构 + `build_<id>()`；③ 注册表描述符 1 条（`fields` 进 `registry/fields.rs`）；④ `<CAP>_ENGINES` 表 1 行。**枚举/工厂分发/签名函数/`match` 分支已全部消失**（原 19 处 → 服务端 ≤5 处；客户端的 `ttsRemoteEngines` + 字段区待 P5 由 schema 驱动收口） |
| engine 写错的可见性 | `unknown_engine_marks_capability_failed_with_hint`（`/api/plugins` 标 `Failed` + 列出可选值）、`boot_rejects_unknown_engine_with_actionable_message`、`memory_engine_none_is_not_an_unknown_engine` |

**验证命令与实际结果**：`cargo test` 默认 **195 passed / 0 failed / 3 ignored**；`cargo test --features sherpa` **195 passed / 0 failed / 3 ignored**；`./gradlew :apps:h5App:publishWeb` **BUILD SUCCESSFUL**（产物含 `kokoro`/`"engine"`/「engine（引擎）」）；文件规模 ≤600 行与"0 个内联测试块"校验均无输出；被改的 21 个 Rust 文件经 `rustfmt --check --skip-children` **0 diff**（`--skip-children` 是必需的：不加会把子模块既有漂移算到本文件头上，我第一轮统计就被它误导过一次）。

⚠️ **未做**：真实模型下的端到端联调（本机无 `/data/models`，跑不了 `mock_client.py`）；`/api/config` 的运行时形状冒烟（无模型无法启动进程，形状由单测覆盖）；统一动作模型（见偏差 8）。

### 0.11 第七批：P5 UI 重组（本文件 §5 的落地）

| 层 | 改动 |
|---|---|
| 服务端 · 三档生效判定 | `POST /api/config` 响应新增 `hot` / `hot_hint` / `changed_sections`。判定在 `app/ws/config/impact.rs`：两侧先 `normalize()`（否则"旧位置搬到规范位置"会被误报成真实变更）→ **叶子级** diff → 逐级查注册表 `field_hot`（字段级优先）→ 回退 `section_hot` → 取**最严格**（`HotReload::strictest`）。新增 `registry::{field_hot, section_hot, section_hot_known}` + `HotReload::{rank, strictest}` |
| 服务端 · 非能力段 | `APP_SECTION_HOT`（`[server]`/`[audio]` → `restart_only`）+ `schema_json()` 新增 `app_sections` 投影；`every_config_section_has_declared_hot_semantics` 把"新增配置段忘登记档位"从**静默降级成需重启**变成硬失败 |
| 客户端 · 能力总览页 | 新 tab「总览」（`PluginMetaState.kt` + `ConfigOverviewCard.kt`）：`GET /api/plugins` → 必需/可选两组 + 计数摘要（**只显示非零项**）+ 状态徽标（**只在偏离时打标**）+ **降级/异常原因行内展开** + 「刷新」（失败 → 就地错误 + 可行动指引，不弹 toast） |
| 客户端 · 页签表 | `ConfigSections.kt`：`SectionDescriptor(id, order, label)` 常量表 + 构建期校验（id 唯一 / order 升序 / 非空）；`renderForm` 由表驱动，新增 tab 从"改一行并记住顺序"收敛为"改表 + 加一条 `when`" |
| 客户端 · 三档文案 | `save()` 读响应 `hot` → 「已保存并立即生效 / 已保存，新会话生效 / 已保存，下次启动生效」；服务端 message 里那句"对**新会话**生效"已删除（档位改由字段元数据 + `hot` 回答，不再由文案承诺） |
| 客户端 · 热生效提示 | `FormSchemaState.kt`：`GET /api/config/schema` → 段级 `sectionNote`（Server/Audio/AIUI 卡各一句）+ 字段级 `fieldNote`（`[tts].cache_entries` 这处"段内档位不一致"的例外）；**`live` 不标注**（默认预期，逐字段标注只会制造噪音） |
| 客户端 · 409 | `conflict` observable + 顶栏「重新加载」（**草稿保留**，与 §5.4-5 一致） |
| 新增组件 | `textNote(tone, micro)`（全站提示文字唯一入口）、`pluginStatusBadge(phase, enabled)`；`labeledField` 新增 `note` 参数 |
| 测试 | `config/tests/impact.rs`（7 条：字段级档位、段级回退、多段取最严格、无改动=live、旧写法迁移不算改动、数组元素变更可见、**每个配置段必须有档位**）；`host/tests.rs` 新增**管理页契约**测试（总览页读的每个键必须存在——客户端是字符串取键，改键名不会编译报错，只会让页面变空白） |

**验收（实跑）**：`cargo test` **206 passed / 0 failed / 3 ignored / 0 warning**（默认与 `--features sherpa` 同结果；⚠️ 这是 `cargo test` 的口径——`cargo check` 仍报 3 条**既有** dead_code，属预留 API，不在本批范围）；`./gradlew :apps:h5App:publishWeb` **BUILD SUCCESSFUL**，产物含 `总览`/`必需能力`/`已保存并立即生效`/`重新加载`/`未知配置页` 等全部新文案；`./gradlew :shared:compileKotlinMacosArm64` **BUILD SUCCESSFUL / 0 warning**。⚠️ 最后一步是必需的：`AdminShell`/`ConnectState` 在 `macosArm64Main`，`publishWeb` **不覆盖**它——只跑 web 会留下未验证的 macApp 路径。

**顺带清掉的历史噪音**：`aiui/tests.rs` 的未用导入 + 4 处 `data == null`（编译期恒假的死判断——`NMAllResponse` 的 `data` 是非空 `JSONObject`，这是照抄既有写法传染的）。清掉后两个 Kotlin 目标都 0 warning。

**与设计的偏差（5 条）**：

1. **统一动作模型仍未做**（`POST /api/plugins/{id}/actions/{name}` + job 轮询）：与 §0.10 偏差 8 是同一件事；UI 侧继续沿用 `probing` 轮询与测试台 `tts_test` 结果帧。
2. **§5.4-2「已覆盖 / 恢复默认」未做**：它要求服务端回答"这个字段是 `config.toml` 显式覆盖、还是回落到内置默认"，而 `GET /api/config` 返回的是**合并后的生效值**——要落地得先改服务端契约（区分"文件里有"与"内置默认"），属独立批次。
3. **`AdminTheme` token 未整体重写**：§5.8 的对齐表只被**新增**组件沿用（`textNote`/`pluginStatusBadge` 用既有 token）；既有组件的硬编码尺寸（44/48/64）未动——纯视觉回归风险，不该混在功能批次里。
4. **状态徽标文案硬编码在 `phase` 分支**（降级/异常/已关闭），没做成服务端 `PHASE_KEYS` 式映射表：这三个词不会随后端变化，且必须与 §5.3 的表格一致。
5. **总览页只有"清单 + 状态"，没有 §5.2 的二级页**（某能力 → 配置/状态/日志三页）：二级页需要"按能力取日志"的数据源，服务端目前没有该端点，硬做会变成假功能；当前用"总览行 → 对应配置 tab"的人工对应关系替代。

---

## 1. 现状盘点（本仓库事实）

> ⚠️ **本节是"改造前"的盘点快照**，用于定位缺口与论证设计。其中多项**已在 §0.5 / §0.6 落地**
> （配置 `revision` + 409、密钥打码 + presence、LLM 三层超时/重试/用量、`/api/plugins`、
> `/api/config/schema`、统一热切换等）。判断"现在是什么状态"请看 §0.5/§0.6 与源码，**不要只看本节**。

### 1.1 能力清单：五个能力 + 两个"体制外"能力

| 能力 | trait | 工厂 | 配置段 | 实现 |
|---|---|---|---|---|
| ASR | `plugins/asr/mod.rs:20` `AsrEngine` | `build_asr` `asr/mod.rs:68` | `[asr]` `asr/mod.rs:25` | SenseVoice（`asr/sensevoice.rs`） |
| VAD | `plugins/vad/mod.rs:14` `VadEngine` | **无工厂**，`Engines::new_vad` 内 `cfg(feature)` 直连 `engine.rs:177-188` | `[vad]` `vad/mod.rs:22` | Silero /（非 sherpa）Mock |
| TTS | `plugins/tts/mod.rs:173` `TtsEngine` | `build_tts` `tts/mod.rs:227`、`build_tts_with_lang` `:237` | `[tts]` + `[tts.xfyun]` `tts/mod.rs:24,55` | Kokoro（本地）/ 讯飞 AIUI（远程） |
| LLM | `plugins/llm/mod.rs:114` `LlmProvider` | `build_llm` `llm/mod.rs:130` | `[llm]` `llm/mod.rs:13` | OpenAI 兼容 Responses API |
| 全链路 | `plugins/aiui/mod.rs:138` `FullChainEngine` | **无工厂**，会话内直接 `AiuiSession::new(...)`（`app/session.rs:590-603`） | `[aiui]` `aiui/mod.rs:25` | AIUI 云闭环 |
| 发音人目录 | — | — | — | `app/voices.rs`（148 条候选池探测 + `/data/voices.json` 缓存），路由 `ws/mod.rs:69` |
| 固件托管 | — | — | — | `app/firmware.rs`（上传/下载/版本），路由 `ws/mod.rs:68` |

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

**证据链**：`Engines::refresh_tts_from_disk`（`engine.rs:139-166`）的调用点**只有一处**：

```
server/src/app/session/bench.rs:107   ← 测试台 tts_test 专用
```

`app/session.rs`（ESP 设备流水线）**完全不调用它**（grep `refresh` 在 `session.rs` 零命中），只通过 `engines.tts()`（`:459`）取当前引擎。也就是说：

- 管理页改 `[tts].backend` 保存后，**设备会话仍然用旧引擎，直到进程重启**；
- 只有跑过一次 macApp 测试台的 `tts_test`（它 `refresh` 了同一个进程内的 `RwLock`），后续设备会话才会"顺带"用上新引擎——这解释了为什么"有时候好像生效了"。

**文档三处与现实矛盾**：`engine.rs:17` 模块注释称「在会话开始 / 测试台 tts_test」、`AGENTS.md:77` 称「改后保存即热切换（新会话/测试台 tts_test 读盘重建引擎）」、`AGENTS.md:124` 称「管理页保存后无需重启」，而 `app/ws/mod.rs:22-23` 又说「引擎相关参数需重启」。**代码是唯一权威：只有 tts_test 生效。**

**修法**：在 `session.rs` 的会话开始处（与测试台同一位置语义）调用统一的 `host.refresh_from_disk()`——这正是 §3.2 规则 3 要做的事；修完后**三处文档统一改成"新会话生效"**，并补一条"改配置 → 起新会话 → 断言引擎名变化"的测试。

### 1.8 一致性缺口汇总（按危害排序）

| # | 缺口 | 证据 | 危害 |
|---|---|---|---|
| 1 | 表单漏字段 + 全量覆盖保存 → 静默重置配置 | 见 §1.7 bug-1 | **高**（正在损害用户） |
| 2 | 热切换只有 TTS 有骨架，且设备路径未接 | 见 §1.7 bug-2；ASR `engine.rs:39`、LLM `:47` 启动即冻结 | **高**（文档承诺与行为不符） |
| 3 | 运行期失败**全部静默**，四种写法 | ASR `session.rs:141`、TTS `:499`、LLM `:544`、AIUI `:176` 都是 `warn!`+continue；只有测试台 `tts_test`/`llm_test` 回报 UI（`bench.rs:242-267,303-323`） | **高**（用户只感知"没反应"） |
| 4 | `asr_test` 把错误**当识别文本**塞进 `stt.text`（`bench.rs:43-56`），与 tts/llm 的 `state:"error"` 语义不同 | 同上 | 中 |
| 5 | 测试台三条路径读盘策略不一致：`tts_test` 读盘热切换、`llm_test` 读盘临时构建、`asr_test` 完全不读盘 | `bench.rs:104-133` / `:287-299` / `:41` | 中 |
| 6 | 构造失败语义不一致：ASR/TTS/VAD/AIUI fail-fast；**LLM 永不失败**（`build_llm` 返回非 `Result`，`llm/mod.rs:130`）→ 密钥错延迟到调用期 | 同上 | 中 |
| 7 | `voices` 探测与讯飞**硬耦合**：`voices.rs:114-127` 直读 `tts.xfyun`、`:277-288` 手写凭据结构、148 条候选池硬编码在 app 层（`:321-470`） | 无法复用于第二个供应商 | 中 |
| 8 | 无 `/api/plugins`（能力清单+状态）、无 `/api/config/schema`（字段元数据）、无用量端点 | 路由表 `ws/mod.rs:57-70` | 中（UI 只能猜后端状态） |
| 9 | **AIUI 在管理页完全不可见**：`renderForm` 只有 6 个 tab（Server/Audio/ASR/VAD/TTS/LLM），`commonMain` 全目录 grep `aiui` **零命中** | `ConfigCards.kt:22-34` | 中（全链路模式只能手改 TOML，且见 bug-1 会被保存清空） |
| 10 | 能力之间无依赖联动：AIUI 启用时应提示/禁用本地 ASR/TTS 卡，UI 无此逻辑 | `engines.rs` 侧有校验，UI 侧无 | 中 |

### 1.2 装配：启动期一次性构建，只有 TTS 可热切换

- `Engines::new`（`engine.rs:58-87`）按固定顺序构建 asr/tts/llm，全部 `Arc<dyn Trait>` 共享；
- 启动期校验**只有两处硬编码**：VAD 模型路径非空（`engine.rs:60-63`）、AIUI 三要素（`engine.rs:65-71`）；
- 热切换**只有 TTS**：`tts_sig`（`engine.rs:43,74`）+ `refresh_tts_from_disk`（`:139-166`），签名比较 → 重建 → 失败保持旧引擎；
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
| 引擎签名热切换 | `engine.rs:139-166` | 已实现 DSH 的 `signature → 重建 → 失败保留旧实例`，且**失败不致命**（正是我们想要的语义） |
| 每会话 VAD 工厂 | `engine.rs:177-188` | per-scope 实例的正确形态 |
| 能力状态字段 + 轮询交互 | `GET /api/tts/voices` 返回 `probing`（`voices.rs:225`）、`POST /api/tts/voices/refresh`（`:232`）触发后台探测，`PROBING` 单飞守卫（`:62,134-136`） | 已经是一套"长任务 + 状态轮询 + 单飞"契约，可泛化为统一 action 模型 |
| 目录是从**真实探测**得出的能力清单 | `voices.rs:62` 起的候选池逐个调 API 验证，通过才进目录 | 与 DSH「清单即能力声明」同源思想：**能力面应该被真实枚举出来**，而不是写死在 UI 里 |
| 流式契约 + 取消语义 | `TtsChunkCallback`（`tts/mod.rs:167-182`）：引擎只产增量 PCM，取消由回调返回 `false` 表达 | 新供应商只要遵守契约就自动获得低 TTFA；下游 pacing/编码与引擎解耦 |
| `blocking_result` 统一包装 | `app/session.rs:30-43`：把取消/panic/内部错误收敛成 `Err(String)` | 引擎只需是同步 trait，无需 async-trait，错误面统一 |
| 派生缓存挂能力身份 | `tts_cache.rs:50-65` + `session.rs:433-441`：cache key 含引擎签名 | 换引擎自动失效，不需要手工清理逻辑 |
| UI 声明式页面注册表 | `AdminComponents.kt:129-166`（`TabUiState/TabPage/tabbedPanel`）+ `ConfigCards.kt:22-34` | 新增一个 tab 只改一处列表——服务端补上 schema 后就能由数据驱动 |
| 组件层与 Toast 已收口 | `AdminFormControls.kt:54-124`、`AdminWidgets.kt:45-161`、`ToastHost.kt:11-45` | UI 差异只剩"用哪些官方控件"，不需要各写一套样式 |
| 契约可测 | `ws/config.rs:225-373`：JSON→Config→TOML→Config 全链路回归 | 扩 schema 时有现成护栏（⚠️ 但需按 bug-1 补全字段与断言） |
| 配置原子写 + 单文件挂载回退 | `ws/config.rs:47-60` 注释与实现 | 生产环境真问题（`EBUSY`）已被解决，别动 |
| 显式"配置来源"端点 | `/api/config/meta` `ws/mod.rs:63` | 与 DSH 的 `profile`/`bundles` 溯源同思路 |
| `ToolSpec/ToolCall` 已预留 | `llm/mod.rs:68-110` | 借 `gm_search` 等工具时无需新协议 |

---

## 2. DSH 插件理念 → 我们要借的 6 条（机制层，非 Node 实现层）

> 完整 8 条与"不必照搬"清单见 `soul-and-graph-memory-plan.md` §11。这里只列**本项目能落地**的 6 条，并直接给出 Rust 形态。

| # | DSH 机制 | 证据（asar 内包/文件） | 本仓库落地形态 |
|---|---|---|---|
| 1 | **清单即能力声明**：宿主只读清单即可枚举插件，无需载入代码 | `dsh-plugin-manager` `reconcile()` 读包 `dsh.bundle` 并写入 `dsh.profile.bundles`；无 `dsh.bundle` 的包只警告"not a profile layer" | `plugins/registry/` 的 `&'static [PluginDescriptor]`：能力可枚举 → `GET /api/plugins` |
| 2 | **注册表 + 命名空间 + 重复即报错** | `cordis/src/registry.ts`、`dsh-llm` 重复路由 → `DUPLICATE_ADAPTER`；`dsh-persona` 全局挂载 *fails loud* | 注册表 `id` 唯一性单测（编译期常量表也能测）；同一能力同一 `engine` id 不可重复 |
| 3 | **声明式依赖（inject）**：全就绪才激活，任一缺失即失败（**而非静默降级**） | `graph-memory/dsh.ts` 的 `inject=[...]`；`fiber.ts resolveConfig()` 校验失败抛 `ValidationError` 拒绝加载 | `requires: &'static [Requirement]`，`PluginHost::validate()` 在构建前跑；错误信息必须可行动（指到缺失模型/环境变量） |
| 4 | **生命周期状态机 + `enabled` 与激活态分离** | `cordis/src/fiber.ts` `FiberState`：`PENDING→LOADING→ACTIVE/FAILED→UNLOADING→DISPOSED`；`dsh-settings` 分开上报两者 | `enum Phase { Disabled, Loading, Active, Degraded{reason}, Failed{error} }` + `PluginStatus.enabled`（配置意图）/`phase`（实际） |
| 5 | **资源统一登记为 effect → disposer**（逆序、幂等） | `cordis/src/events.ts`、`ctx.effect(execute) → Disposable` | `PluginHost` 持 `Vec<Box<dyn FnOnce() + Send>>`，卸载/重配/退出走同一条清理路径（`Drop` + `CancellationToken`） |
| 6 | **schema 驱动校验 + `.volatile()` 标记可热改字段** | `dsh-settings/src/schema.ts` 投影、`ValidationError`；⚠️ 但 `autoGenerate` **无随发布客户端使用**，表单是手写的 | `FieldSchema{key,label,kind,default,help,hot}` 常量表 → `GET /api/config/schema`（**用于校验/默认值/字段提示/密钥打码标记，不用于自动生成布局**，理由见 `soul-and-graph-memory-plan.md` §11.1） |

**明确不借**（Node/Cordis 特有，详见 `soul-and-graph-memory-plan.md` §11.3）：`!!js` 表达式、原型链 `ctx` 解析、`Symbol` realm、pnpm 安装器与 `dsh.bundle` 对账、typert 装饰器 RPC、客户端 HMR 版本戳。

---

## 3. 统一插件化架构设计

### 3.1 数据模型：三张表 + 一个宿主

```rust
// server/src/plugins/registry/{mod,fields,impls}.rs（新增，全部为 'static 常量，零分配、可单测）

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Capability { Asr, Vad, Tts, Llm, FullChain, Voices, Firmware, Memory, Soul }

/// 热生效语义（= DSH 的 .volatile()，但要区分三种粒度）
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HotReload {
    Live,        // 保存即对**新请求**生效（LLM/记忆/灵魂/TTS）
    NextSession, // 保存后对**新会话**生效（VAD 阈值等；当前会话保持旧值）
    RestartOnly, // 必须重启（server.port / worker_threads / 下行音频参数）
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Requirement { SherpaFeature, ModelFiles, Credentials, LlmUpstream, EmbeddingUpstream }

#[derive(Clone, Copy)]
pub enum FieldKind { Str, Password, Int, Float, Bool, Enum(&'static [&'static str]), Path, Dir, Text }

#[derive(Clone, Copy)]
pub struct FieldSchema {
    pub key: &'static str,          // TOML 路径片段："api_key"
    pub label: &'static str,        // UI 标签（中文）
    pub kind: FieldKind,
    pub default_hint: &'static str, // 展示用默认值（真实默认值仍在 serde default 函数里）
    pub help: &'static str,         // 一句话说明 + 坑（例："留空则按 X 降级"）
    pub hot: HotReload,
    pub required: bool,
}

/// 构建结果：类型擦除但**保持各 trait 不变**
pub enum Built {
    Asr(Arc<dyn AsrEngine>),
    Vad(Box<dyn VadEngine>),
    Tts(Arc<dyn TtsEngine>),
    Llm(Llm),
    FullChain(Arc<dyn FullChainEngine>),
}

pub struct PluginDescriptor {
    pub id: &'static str,              // "tts.kokoro"（<capability>.<impl>，namespace 唯一）
    pub capability: Capability,
    pub display: &'static str,         // "Kokoro（本地离线）"
    pub local: bool,                   // 本地 / 远程（UI 两级下拉的依据）
    pub hot: HotReload,
    pub requires: &'static [Requirement],
    pub fields: &'static [FieldSchema],          // 该实现的私有字段（含 secret）
    /// 热切换判定：由**该实现自己**声明哪些字段影响成败（取代 engine.rs 手写的 tts_sig）
    pub signature: fn(&Config) -> String,
    pub build: fn(&Config) -> Result<Built>,
}

pub static REGISTRY: &[PluginDescriptor] = &[
    asr::SENSEVOICE, tts::KOKORO, tts::XFYUN, llm::OPENAI, vad::SILERO, aiui::FULL_CHAIN,
    voices::CATALOG, firmware::HOST, /* P4: memory::GRAPH, soul::PROFILE */
];
```

**关键取舍**：`Built` 是一个枚举而不是 `Box<dyn Any>`。Rust 里类型擦除后向下转型会引入运行时 panic 面；枚举 + 按能力 `match` 保持 100% 编译期检查，且**五个引擎的 trait 定义一行都不用改**（这是本设计能低风险落地的根本原因）。

### 3.2 宿主：装配、校验、隔离、热切换、上报

```rust
// server/src/plugins/host.rs（新增）
pub struct PluginHost {
    cfg: Arc<Config>,
    states: RwLock<HashMap<&'static str, PluginState>>, // id -> {phase, last_error, built_at}
    disposers: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
}

impl PluginHost {
    /// 启动装配：required 能力失败 → 快速失败；optional 失败 → 记 Degraded/Failed 继续启动
    pub fn boot(cfg: &Config) -> Result<Arc<Self>>;
    /// 依赖校验（inject 语义）：构建前把"缺什么"说清楚，而不是让引擎 ctor 抛底层错误
    fn validate(d: &PluginDescriptor, cfg: &Config) -> Result<()>;
    /// 统一热切换：逐个 descriptor 比 signature，变了才重建；失败**保留旧实例**并记 last_error
    pub fn refresh_from_disk(&self) -> Vec<(&'static str, Result<(), String>)>;
    /// 状态快照（GET /api/plugins）
    pub fn snapshot(&self) -> Vec<PluginStatus>;
}
```

四条硬规则：

1. **required vs optional 分级**：`Asr/Vad/Tts/Llm` 属 required（对齐现状：`Engines::new` 失败即退出）；`Voices/Firmware/Memory/Soul` 属 optional → **一个可选插件坏掉不能拖垮语音服务**（DSH 的"插件失败不致命"）。例：记忆库文件损坏 → `Phase::Degraded{reason:"记忆库不可写，已禁用记忆"}`，语音对话照常。
2. **校验前移**：把现在散落的硬编码校验（`engine.rs:60-63` VAD 路径、`:65-71` AIUI 三要素）统一成 `required` 字段 + `Requirement::{ModelFiles, Credentials}`，错误文案统一含"怎么修"（例：`[vad].model 文件不存在：/data/models/silero_vad.onnx（容器内缺失模型时入口脚本会自动下载；离线环境请预置）`）。
3. **热切换覆盖 LLM/记忆**：`Engines.llm` 由 `pub llm: Llm` 改为 `RwLock<Llm>`（与既有 `tts: RwLock<...>` 同模式，**不引新依赖**），`refresh_from_disk` 在**会话开始 / 测试台请求前**调用（现有 TTS 的调用点），从此改 `[llm].model` 不再需要重启。
4. **清理登记**：记忆库句柄、探测任务、后台抽取任务在构建时 `register_disposer(...)`；热切换与进程退出共用同一路径（避免"换了引擎旧连接泄漏"）。

### 3.3 配置约定（统一后的唯一规范）

```toml
[tts]
enabled = true                 # 新增：所有能力段统一有（缺省 true，兼容旧配置）
engine  = "kokoro"             # 新增：实现 id；`backend` 保留为 deprecated alias（serde alias）
num_threads = 4                # 通用项（跨实现）留本体
cache_entries = 200

[tts.kokoro]                   # 实现私有段（本地实现也进子段——统一规则）
model = "/data/models/kokoro-int8-multi-lang-v1_1/model.int8.onnx"
# ...
[tts.xfyun]                    # 远程实现私有段（既有形态，不动）
app_id = "..."

[llm]
engine = "openai"
[llm.openai]
api_base = "https://.../v1/responses"
api_key_env = "XIAOZHI_LLM_API_KEY"   # 密钥引用优先；api_key 明文保留但 UI 打码
[llm.routing]                  # 用途路由（P3，见 §4.4）
chat = "openai"
extract = "openai-mini"
embedding = "bge-m3"
```

迁移策略（**这是本文件唯一"破坏性"的部分，故单独说明**）：

- **只增不删**：`engine` 新增、`backend` 保留 alias；`[tts.kokoro]` 与旧的 `[tts].model` 双读——`serde(alias)` 做不到"两种位置都认"，因此实现方式为：`TtsConfig::normalize()` 在加载后把本体里的 Kokoro 字段搬进 `kokoro` 子结构（若子结构为空），保存时只写新结构；
- 未知键继续静默忽略（`config.rs:10-11` 的既有行为，不改）；
- 保存后 `config.toml` 重排为规范形态（管理页保存本来就丢注释，见 `ws/config.rs:2`，行为无变化）；
- **回滚**：`engine` 缺省时行为与今天逐字节一致（`engine_signature` 也保持旧公式），因此 P0（只加表与端点）**零行为变化**。

### 3.4 端点：能力可枚举、可诊断、可操作

| 端点 | 语义 | 借用点 |
|---|---|---|
| `GET /api/plugins` | 能力清单 + `enabled`/`phase`/`active_impl`/`hot`/`last_error` | DSH `pluginInventory.list()`（inflight 只读清单） |
| `GET /api/config/schema` | 按能力+实现返回 `FieldSchema[]`（含 `secret:true` 标记、`hot` 标记、`required`） | DSH `dsh-settings` 的 Config 投影（**但只做元数据，不做自动布局**） |
| `GET /api/config` | **打码** secret（`has_api_key: true`）+ 返回 `revision` | DSH secrets → presence marker |
| `POST /api/config` | 带 `expected_revision`；不一致 409；省略的 secret 字段**保留原值** | DSH `form.mutate(ops, expectedRevision)` 拒绝过期写入 |
| `POST /api/plugins/{id}/actions/{name}` | 统一动作：`voices.probe` / `tts.test` / `llm.test` / `memory.clear` / `memory.recall` / `plugin.reload` | DSH `ctx.commands.register` → `{kind:'success'\|'error', text}`；本项目已有的 `voices.probing` 轮询契约 |
| `GET /api/plugins/{id}/actions/{job}` | 长任务状态轮询（`running/done/failed` + 结构化结果） | 同上（probe 全量约 1 分钟，必须异步） |

**为什么动作要统一**：现在"探测音色目录"是 `voices.rs` 私有实现（`probing` 字段 + 自定义轮询），"测试合成"是 bench 的 `tts_test` 协议消息，两者语义相同（长任务 + 状态 + 结果）却两套契约。统一后 UI 只需要一套"动作按钮 + 状态徽标 + 结果区"组件。

### 3.5 迁移路径（每步可独立上线、可回滚）

| 步 | 内容 | 行为变化 | 回滚 |
|---|---|---|---|
| M0 | 加 `registry/`（mod/fields/impls）/`host.rs`/`PluginStatus` + `GET /api/plugins`（**只读、只报告，不接管装配**） | 无（纯新增端点） | 删端点 |
| M1 | `Engines::new` 改为经 `PluginHost` 装配（required 校验前移，错误文案升级） | 错误信息更清楚；其余不变 | 保留旧 `Engines::new` 分支 |
| M2 | LLM/记忆纳入热切换（`RwLock<Llm>` + 统一 `refresh_from_disk`） | `[llm]` 改动免重启 | 关闭开关即回旧行为 |
| M3 | 配置统一（`engine` + `[<cap>.<id>]` 私有段；TTS Kokoro 参数搬家） | 保存后的 TOML 结构变化（旧文件仍可读） | alias + `normalize()` 双读 |
| M4 | `voices`/`firmware` 纳入注册表与统一动作模型 | 端点兼容保留旧路径 | 旧路由不动 |
| M5 | 客户端「能力总览」页 + 每能力卡片改造（§5） | UI 变化 | 旧卡片保留 |

### 3.6 收益量化：扩展成本对比（本设计的 ROI 证明）

| 场景 | 现状（实测步骤数） | 统一后（目标值） | 证据 |
|---|---|---|---|
| 新增一个 **TTS 供应商** | **19 处**：`plugins/tts/mod.rs` 9 处（`TtsBackendKind` 变体/`backend_kind`/`engine_signature`/`TtsConfig` 字段/`mod`+`pub use`/`Default`/`build_tts`/`build_tts_with_lang`/文档）+ `config.rs` re-export + `config.example.toml` + `voices.rs` 与凭据测试需**新写一套** + 客户端 4 处（observable/注册表/`save` put/`fill` opt）+ 卡片 `vif` 区整段抄 | **≤5 处**：① 新文件实现 `TtsEngine`；② 描述符表加 1 条（含 `fields`/`signature`/`build`）；③ 凭据结构体；④（可选）专用交互卡片 | §1.3、盘点 D.2 |
| 新增一个 **ASR 实现** | **15 处**，且**必须先自己补** `backend` 字段与热切换骨架（ASR 既无 `backend` 也无 `RwLock`） | **≤5 处**（同上；热切换由 `PluginHost` 统一提供，不再每个能力各写一遍） | 盘点 D.1 |
| 新增一张**能力卡**（例 AIUI） | **5~8 处**：tab 列表 + 卡片函数 + observable + `save` put + `fill` opt +（可选）注册表/按钮/尺寸常量 | **1~2 处**：`SectionRegistry` 加 1 行 + 卡片函数（纯字段驱动时 1 行） | 盘点 E.2 |
| **改一个引擎并生效** | 必须**重启**（除 TTS 经测试台间接生效，见 §1.7 bug-2） | 保存即对**新会话**生效，`/api/plugins` 显示结果与失败原因 | §1.7 |

> ⚠️ 「统一后」是**目标值**，需要 P1~P4 全部落地；P1 单独落地只带来「能力可枚举 + 可诊断」这一半收益。别把目标值当成一步到位。

---

## 4. 借鉴 DSH 的 LLM 插件：只借"护栏"，不借"编排"

> 本节所有 DSH 结论均来自 asar 内该包 `README.md` / `lib/*.js` 原文（引号内为原文），并对齐 §1.4 里本仓库的实测缺口。

### 4.0 先纠正一个容易想当然的前提：DSH **没有** provider 故障转移

这一点必须先说清楚，否则会"照抄一个不存在的东西"。三条独立证据：

1. `dsh-llm/README.md:74`：*"This service never re-runs a request: retrying is the job of `dsh-llm-retry` at the agent's failed-step extension point."*
2. `dsh-llm-retry/lib/index.js` 的 `recover()` 命中策略后只产出 `{kind:'retry'}`，让循环**在同一 provider/model 上重跑该 step**；文件内没有任何"换 provider/model"的分支。
3. 对 290 个 `@deepseek-ai/*` 包全量检索 `fallback`：命中的全是 `defaultContextWindow` 容量回退、图片 base64 inline 回退、标题兜底，**没有一例是 provider/model 故障转移**。

**DSH 的模型侧韧性 = 同路由重试 + 压力折叠**，不是"多 provider 降级"。对我们的意义：**`[llm.fallback]` 若要做，是我们自己的设计，不能记在"借鉴 DSH"账上**。建议默认**不做**（语音一次回复只打一次请求，多打一份 = 多一份 TTFA 与费用）；确需降级时也只对**可重试类错误**（连接失败 / 429 / 5xx）生效，绝不"永久失败也换"。

**顺带一条不变量（可作 §3.2 的外部依据）**：adapter 注册是**原子 + 单所有者 + 注册即固化策略**：

- `ctx.llm.registerAdapter(providers, adapter)` 是**全有或全无**的原子注册：任一路由已被占用 → 抛 `LlmError{code:"DUPLICATE_ADAPTER"}`（`dsh-llm/lib/index.js:1863`，消息 `an adapter for provider "..." is already registered`）；
- 注册时**捕获该路由的 retryPolicy**（`lib/index.js:1867`：`const retryPolicy = adapter.providerRetryPolicy(provider) ?? resolveRetryPolicy(...)`）；
- README 不变量原文：*"Registry mutations are atomic — route and directory registration validates the whole candidate set before anything moves, so a refused change leaves the previous state serving."*（**拒绝变更后旧状态继续服务**）。

**适合我们**：我们的 `[llm]` 是单后端，没有"路由冲突"问题，但**"注册即固化策略 + 拒绝变更不破坏当前状态"**这条不变量，正是 §3.2 里 `PluginHost::refresh_from_disk` 的语义（重建失败保留旧实例）。可直接引用作为该设计的外部依据。


### 4.1 命名化失败分类 + 有界重试（`dsh-llm-retry`）

**DSH 的做法**：重试策略**不在重试插件里**，而在每个 provider profile 上（`retryPolicy`），`dsh-llm-retry` 自称 *"It is the executor: the retry policy itself lives on each provider adapter's configuration, and this package has no configuration of its own."*

- 失败分类是**具名枚举**：`EMPTY_RESPONSE` / `RATE_LIMIT` / `SERVER` / `TIMEOUT` / `TRANSPORT`；
- **normal 模式默认**：上述 5 类各 5 次重试，指数退避 **500 ms → 10 s，10% 抖动**；
- **always 模式**：无次数上限，直到成功/取消/dispose（原文：*"always mode continues until success, cancellation, or disposal"*）；
- **`Retry-After` 优先**：*"A valid `Retry-After` from the provider replaces local backoff when it fits the policy bounds."*
- 取消语义：*"Cancellation or plugin disposal aborts active backoff … and makes a callback captured before disposal fail closed."*
- 成本与可见性：*"Each retry is another billed provider request"*、*"Nothing here is model-visible: no retry event, delay, provider error, or failed partial output reaches the model."*

**精确默认值（源码逐字，已核对 `dsh-llm/lib/types/retry-policy.js:12-21`）**：`DEFAULT_MAX_RETRIES=5`、`DEFAULT_INITIAL_DELAY_MS=500`、`DEFAULT_MAX_DELAY_MS=10_000`、`DEFAULT_JITTER_RATIO=0.1`、可重试码 = `[EMPTY_RESPONSE, RATE_LIMIT, SERVER, TIMEOUT, TRANSPORT]`。退避公式（`dsh-llm-retry/lib/index.js:44`）：`delay = min( min(initial × 2^(retry−1), max) × (1 − jitter + 2·jitter·rand()), max )`。校验是 fail-loud 的：未知键报错、`retryableCodes` 不得为空/重复、`initial ≤ max`。

**分类纪律：按 code 路由，绝不解析 message**（`dsh-llm/lib/types/error.js` 头部原文 *"route on this, never by parsing `message`"*）。码表：`AUTH(401/403)` / `QUOTA` / `RATE_LIMIT` / `CONTEXT_WINDOW_EXCEEDED` / `INVALID_REQUEST` / `SERVER` / `HTTP_<status>` / `TRANSPORT`（传输失败）/ `ABORTED`（取消）/ `TIMEOUT`（**stream-idle 到期**）/ `EMPTY_RESPONSE`（退化完成＝空回复，必须当失败）。

**超时与重试是两件事**：`retryPolicy` **不含超时字段**；模型侧超时在 adapter 上（`streamIdleTimeoutMs`，DeepSeek 默认 300,000 ms，语义是"单次流读取的最大空闲时间"）。`Retry-After` 若 ≤ `maxDelayMs` 则优先于本地退避；**normal 模式下超过上界就放弃重试**。

**我们的缺口（实测三处）**：

1. `openai.rs:82-85` 的 client 只设了 UA，**没有 `.timeout()` / `.connect_timeout()`** → 上游挂起即永久等待；
2. `openai.rs:163-166` 把状态码与响应体**拼成一个字符串** `bail!("LLM API 返回错误 {status}: {body}")` → 调用方**无法按状态分类**，重试与降级都无从谈起；
3. **没有"空回复"判定**（`EMPTY_RESPONSE`）——退化完成会被当成成功，用户侧表现就是"没回应"。

**落法**：把重试做成 trait **之上的装饰器**（照抄 DSH「服务不重试、执行器重试」的分层），`LlmProvider` 保持单次尝试语义：

```toml
[llm]                        # 超时属传输层，不属重试策略（对齐 DSH 的 streamIdleTimeoutMs）
connect_timeout_ms     = 3000
stream_idle_timeout_ms = 30000

[llm.retry]                  # 只提供 normal；**不提供 always**（语音不可无限重试）
max_retries       = 5
retryable         = ["rate_limit","server","timeout","transport","empty_response"]
initial_delay_ms  = 500
max_delay_ms      = 10000
jitter_ratio      = 0.1
honor_retry_after = true
```

Rust 侧新增 `LlmFailure` 枚举（`RateLimit / Server / Timeout / Transport / EmptyResponse / Auth / Quota / ContextWindowExceeded / InvalidRequest / Http(u16)`）；`openai.rs` 只负责「HTTP/传输 → code」的映射，重试循环放在装饰器里。**两条语音特有约束**：① 重试总预算要有上界（建议 ≈8 s，超了就放弃并走兜底话术）；② **已下发音频后不再重试**（见下）。

实现要点（**与 DSH 的关键差异，对我们更有利**）：DSH 明确承认 *"Provider HTTP status is unavailable — pi-ai error events do not expose a stable HTTP status across providers"*，所以它只能按抽象码分类；**我们用 `reqwest` 直接拿到 status**，因此分类可以直接建立在 429/5xx/连接错误上，并把 status 一起写进日志与 `/api/plugins` 的 `last_error`。

⚠️ **流式重试的边界（必须写进实现注释）**：一旦已经有 chunk 下发到设备，**不能再重试**（会重复播报）。规则：**首个 chunk 之前**的重试才允许；首 chunk 之后的失败只能向设备发 `tts:stop`/错误提示。这与 DSH 的 *"a raw stream cannot separate already-emitted chunks durably"* 是同一个约束。

### 4.2 路由级模型元数据与"写时校验"（`dsh-llm-pi-ai`）

**DSH 的做法**：一份 `providers` 字典描述所有端点，每条 profile 含 `apiKeyEnv` / `baseURL` / `api` / `models[]` / `retryPolicy`；每个 model 可声明 `contextWindow`（README 示例为 `200000`、`262144`）。校验语义尤其值得学：

- *"A route pi-ai does not ship needs `api`, `baseURL`, and a non-empty `models` list; an unserviceable profile is **refused where it is written, naming the route and model**."* → **写时就拒绝，并指名道姓**（而不是启动后才炸）。
- *"Config updates strictly validate **changed** providers. Initial loading retains stored catalog failures as editable provider diagnostics; unchanged failed providers do not block."* → **只校验被改动的条目**；历史遗留的错误条目不该阻塞启动，但要能被 UI 展示与编辑。
- `apiKeyEnv`：*"a credential reference resolved per request through the harness credential seam"* → 密钥是引用不是值。
- 已知坑（原文）：*"`headers` can carry a credential **the redactor never sees**"* → 我们的打码逻辑必须把自由 `headers` 字段视为可能含密钥。

**我们的缺口**：`[llm]` 是扁平的 `api_base/api_key/model/system_prompt/max_history/temperature/stream`（`llm/mod.rs:13-24` + Default impl），**没有 model 元数据、没有 context window**，因此 §4.3 的"占用率"与记忆注入预算都无处标定。

**落法**：见 §3.3 的 `[llm.<id>]` 段 + `[llm.routing]`；`/api/config` 的 POST 校验规则 = "只校验被改动的 profile，失败要指名 route + model"（对齐 DSH 语义），错误直接返回前端做行内提示。

### 4.3 Token 计量与"上下文压力"分解（`dsh-token-meter`）

**DSH 的做法**（`ctx.tokenMeter` 两个操作：`measure(session, requestHeader?)` / `estimateMessage(message)`）：

- 会话级读数：`totalTokens`（请求+响应压力）与 `surfaceTokens`（仅模型可见面）；
- 用量分解：`uncachedInputTokens` / `outputTokens` / `cacheReadTokens` / `cacheWriteTokens`（**缓存读写分开**）；
- 压力：`contextPressure{ pressureTokens, projectedTokens, contextWindow }`，其中 `contextWindow` 来自 *"`ctx.llm.resolveModelInfo().context`"* —— **容量属于 adapter/route，不属于计量器**；
- 组合分解：`contextBreakdown{ systemTokens, toolsTokens, messageTokens }`；
- 诚实声明局限：*"The `contextBreakdown` figures are estimates … **CJK text and JSON schemas underprice badly at four characters per token**"*，且 *"Occupancy is a reference figure, not a billing record"*。

**我们的缺口**：完全没有用量观测。这有两个后果：① 成本不可见；② **无法回答"记忆/灵魂到底让 prompt 长了多少"**——而这正是记忆功能上线后第一个会被问的问题。

**落法**：四桶命名与口径**逐条对齐** DSH（这样将来对账/对标不会各说各话）：

```toml
[llm.usage]
enabled      = true
log_per_turn = true          # 每轮 debug 日志：input/output/cache_read/tools/system/recall + ttfa_ms
```

- 每会话持一个 `TokenMeter`（随 `session.rs` 生命周期），四桶：`uncached_input` / `output` / `cache_read` / `cache_write`；**一次重试算另一次计费尝试**（对应 DSH 的 *"Each retry is another billed provider request"*）；
- **压力口径 = `input + cache_read + cache_write`（不含 output）**——与 DSH 的 `pressureFrom` 一致，且必须与 §4.5 折叠触发的口径**用同一函数**，否则"UI 说没满、折叠却触发了"；
- 只读端点 `GET /api/session/{id}/usage`（按会话，不按全局；DSH 也是 session 级 fold）+ `GET /api/llm/usage`（进程累计，便于运维）；
- **不做金额换算**：DSH 全包**没有**任何单价/金额字段（只有图片视觉 token 计价），README 亦声明 *"Occupancy is a reference figure, not a billing record"*。各种 OpenAI 兼容网关单价不同，硬编码会误导用户——**只报 token**；
- **中文启发式必须自校准**：不要照抄 `CHARS_PER_TOKEN = 4`（DSH 原文自陈 CJK 严重低估）。按字符区间分段：CJK 汉字/CJK 标点按 **≈1 token/字**，ASCII 按 **≈4 字符/token**，并在 UI 标注"估算值"；
- 记忆注入预算改成**以 token 为单位**（`recall_max_tokens` + `context_share`），与 `[llm.context].window_tokens` 联动——这直接改进 `soul-and-graph-memory-plan.md` §5.6 的 `recall_max_chars`。

### 4.4 有序 prompt 段：把"灵魂/记忆"做成并列 section（`dsh-system-prompt` + `dsh-persona`）

**DSH 的做法**：

- `ctx.systemPrompt` 是**注册表**，插件的贡献按 `order` **升序拼接**，且 *"equal orders use code-unit name order"*（**同级有确定性 tie-break**，避免顺序随机）；
- 实测内置 order：固定开场白 **−1000**、`personaPrefix` **0**、`personaSuffix` **10200**（`persona` 与"部署级人格"是**同一套 section 机制**里的条目，不是特例）；
- 变量：`{{name}}` 每次组装时求值，*"scoped variables shadow a same-named global for that agent"*；
- `interpolate: false` 可让某段**保持字面**（防 `{{}}` 被吃掉）——对"记忆原文里出现花括号"的场景是必需的保险；
- **Prefix 稳定性 = prompt cache 命中率**：*"Prefix-stable while identity, persona, variables, section text, and order render identically"*、*"reordering changes cache shape → may invalidate reuse"*、*"Schema tokens repeat on every request"*。

**DSH 的实际段号表是"稀疏编号"**（`dsh-system-prompt/lib/index.js:10`，可直接借用这个口径）：`HARNESS_IDENTITY -1000` / `DEPLOYMENT_PERSONA_PREFIX 0` / `PLAN_POLICY 500` / 各工具 `1000–3100` / `TOOLS_SDK 5000` / `DELIVERABLE_FILE_REFERENCES 9000` / `STRUCTURED_OUTPUT 9900` / `HARNESS_SOURCE 10000` / `WEB_SURFACE 10100` / `DEPLOYMENT_PERSONA_SUFFIX 10200`（运行时事实另有 `CONTEXT_ORDERS`：sandbox 110 / approval 115 / subagent 120）。**用稀疏数字（间隔 100~500）而不是连续序号**，是为了以后往任意位置插一段而不必重编号——我们的 `[soul]`/`[memory]`/技能/运行时事实也应照此留空档。

**插值的纪律是"宁可吵闹地失败，也不要畸形的 prompt"**：DSH 对未知引用、已注册但值为 `undefined`、畸形 `{{a}b}}` **一律抛错**（原文 *"a malformed prompt is worse than a loud failure"*）；孤立 `{{` 当普通文本；**替换结果不再二次扫描**；空段丢弃、`\n\n` 连接；要保留字面花括号必须 `interpolate: false`。→ 我们的 `assemble()` 同样应在**启动期**校验未知变量（而不是渲染出 `{{memory}}` 塞给模型）；记忆原文里可能出现花括号，因此记忆段应显式 `interpolate: false`。

**落法（也是对本项目既有设计的一处修正）**：

`[soul]` / `[memory]` 都做成 `systemPrompt` 注册表的**段提供者**（`order` 从描述符里声明），而非往一个字符串里拼。**顺序建议**：

```
order 0     : 身份与人格（[soul] 前缀，**稳定**，只随配置变化）
order 100   : 能力/风格约束（[soul] 后缀：句长、口语化、拒答边界）
order 900   : 工具说明（若有）
order 1000  : 历史（append-only 前缀，缓存友好）
order 1100  : 记忆召回（**易变**，放在历史之后 → 保住前面整段的 prefix cache）
order 1200  : 当前用户输入（由 input[] 承载）
```

⚠️ **权衡要说清楚**：`soul-and-graph-memory-plan.md` §4.3 原设计把记忆放在历史**之前**（该节已加修订批注）。改为之后有得有失：**得** = 前缀缓存命中（`max_history` 越大越值钱，DeepSeek/OpenAI 的 prefix cache 都按前缀命中计费）；**失** = 召回内容离用户问题更远、recency 稍弱。因此给一个显式开关（`[memory].inject_position = "after_history" | "before_history"`，默认 `after_history`），并在文档里写明：`max_history` 调大时才考虑 `before_history` 的收益反转。**不要靠猜，用 §4.3 的 `cache_read` 读数验证。**

⚠️ **别高估 prefix cache 的收益（本项目特有）**：`session.rs:239-241,568-570` 的历史截断是**从头丢弃**（`history.remove(0)`），一旦发生截断，整个历史前缀都移位 → 缓存失效。也就是说 prefix 稳定性只对"还没触发截断的对话"有效。**结论：先把 §4.1 的重试与 §4.3 的计量做出来，再谈段序优化**；段序按上面给的默认值即可，不要提前做缓存微调。

### 4.5 折叠（compaction）：**先别做，用观测决定**（对调研结论的一处修正）

**DSH 机制**（`dsh-compaction-basic`）：
- 触发公式（`lib/index.js:128-133`）：`messageBudget = W − O`；`threshold = floor(min(W × thresholdRatio, W − O − headroom))`；默认 `thresholdRatio=0.8`、`retainRatio=0.16`、`headroomTokens=65536`；
- 压力口径 = **输入 + cache 读写（不含输出）**，与 `dsh-token-meter` 的 `pressureFrom` 一致；
- 模型看到的东西：选中区间被**一条 user-role 消息**替换 = preamble + `<compacted-summary>` 摘要 `</compacted-summary>`，后接**原样保留的近期 tail**；preamble 明确要求"当作既定背景、不要复述、不要确认这个 checkpoint"；
- 摘要请求复用 `system node 0 + 上次 tools + 被 shadow 的消息（逐字节）`，把指令作为**最后一条 user 消息** → 目的是**吃前缀缓存**；
- 选区从"第一个非 `system/message` 节点"开始，必须 tool-pairing 平衡；**`system` 段永不被折叠**。

**判断（这里与调研报告有分歧，给出取舍）**：调研把折叠列为"当前最大功能空缺"，我认为**对本项目应降级为条件项**：

- 语音历史本来就短（`max_history` 默认 10 条 ≈ 5 轮），而**记忆召回（P4）已经覆盖"跨会话事实"**这一类需求；
- 折叠的真实收益（连续对话超过 ~5 轮仍记得细节）在语音场景出现频率低，成本是"一次额外 LLM 调用 + 一整套复杂度"；
- 真正该修的是**硬截断这个语义**（见 `soul-and-graph-memory-plan.md` §1.2；本仓库证据 `session.rs:239-241,568-570`：按条目从最旧开始丢），而它有更便宜的解法：把 `max_history` 换成**按 token 预算保留最近 N 轮原文**。

**落法**：P4 先做「记忆召回 + `[llm.context]` 预算化截断 + 用量观测」；**只有观测证明截断造成实际遗忘**（`contextBreakdown.messageTokens` 频繁触顶、且有用户抱怨"忘事"）才上折叠，最小实现 = `[llm.context].summarize=true` + 折叠成一条 checkpoint 消息 + 复用同一端点。**不要**引入 DSH 的 pruner / spill / 事务日志锁整套。中文估算必须用 `chars/1.6` 而非 DSH 的 `chars/4`（后者对中文低估约 2.5×）。

### 4.6 工具管线：若将来做 MCP 闭环，借两条纪律（`dsh-tools`）

- 固定管线：`pre-execute`(allow/deny/ask) → **单调 guard** → `execute` → `post-execute` → `finalizeContent` → 只读 `result`；
- **`pre-execute` 故意不能修改 `exec.arguments`**——原文理由：否则 *"logged and rendered args would desync from what ran"*（审计一致性优先于灵活）；
- **guard 单调不可翻转**：拒绝是硬事实，后续 listener 不能改回允许；
- **拒绝 = 工具结果，不是异常**：产出 `Error: <reason>` + `isError:true` 回填给模型，让模型自己说"这个我做不到"，**不中断 WebSocket**；
- `ask` 需要审批通道，无通道时**降级为 deny** → 我们**不实现 `ask`**（ESP32 无 UI 可审）。

落法：`ToolSpec` 增加 `guard: fn(&ToolCall) -> Option<String>`（只读参数、同步、单调），拒绝回填 `function_call_output` 的 `Error:`。这也是 `gm_search` 上线时的安全底座。

### 4.7 LLM 卡的信息架构（`dsh-client-ui-settings-models` 的三条原则）

1. **高频字段在卡上，低频/高级字段折叠或干脆不上 UI**。DSH 明确把 retry/timeout 排除在设置页之外（README：*"Retry policy, timeouts ... remain in `cordis.patch.yml`"*）——**这条对我们直接适用**：§4.1/§4.5 新增的 `[llm.retry]`/`[llm.context]` 字段**不要平铺到卡片**，放"高级"折叠区（默认收起）；主卡只留 `api_base` / `api_key` / `model` / `stream`。
2. **密钥只写不读**。DSH 的 provider 行主字段只有一个 API key 输入，write-only，页面**从不询问环境变量名**，凭据状态用绿点/红点表示。我们的现状相反（`GET /api/config` 明文回显 `api_key`，`ConfigFormState.kt:437/533` 明文往返）→ 配合 §3.4 的打码 + "留空即不改"语义改造。
3. **写前校验 + 字段级错误**。DSH 要求 key **全为可打印 ASCII**（`[\x21-\x7E]`，因为它最终要进 HTTP header）、拒绝 `NAME=value` 粘贴形式、容量非法即拒、写入带 `revision` 且冲突返回 `settings/conflict`。→ 我们的 `POST /api/config` 应对 `api_base`（http(s) URL）、`model`（非空）、`api_key`（可打印 ASCII）做写前校验并给字段级提示（`ConfigFormState` 已有 `dirty`/`vif` 机制，改动面小）。

**附带可借**：DSH 的「Fetch available models」会**真去问端点**再让用户选择，而不是手打 model 字符串 → 我们的「测试连接」动作（§3.4 的 action 模型）应同时返回**可用模型列表**，卡片上用官方 AlertDialog 下拉选择。

### 4.8 明确**不借鉴**（语音单轮场景不划算）

| DSH 包 | 为什么不用 |
|---|---|
| `dsh-agent-loop` / `dsh-subagent*` / `dsh-workflow*` / `dsh-tool-ralph` / `dsh-experimental-agent-team*` | 语音是一问一答 + 可打断；多步 agent 循环会把 TTFA 变成多跳模型调用，且设备端没有"计划/审批"交互面 |
| `dsh-ptc-runtime*`（`run_code` + TS/Python SDK 生成） | 依赖 Node/Python 运行时；其自陈 *"intermediate values are execution-local and unbounded by bytes — may exhaust process or worker memory"*；我们也不该有脚本执行面 |
| `dsh-tool-*`（bash / fs / pwsh / web / workflow…） | 语音终端**不应该**有任意工具执行面（安全与延迟双重理由）。若要做，只暴露只读白名单 + §4.6 的单调 guard，并沿用已有 `ToolSpec` |
| 交互式审批（`ask` / `dsh-user-approval` / `sandbox_permissions` 提权） | ESP32 无 UI 可审；DSH 自己也定义「无审批通道 → 降级 deny」 |
| `dsh-web-search-deepseek` / `dsh-tool-web` | 拉长 TTFA；要联网应是**显式开关的独立能力**，不是默认注入 |
| `always` 重试模式 | 认证/配额/非法请求会被无限重试且持续计费；语音必须有限预算 |
| 图片/文件/视觉预算体系（Files API、`IMAGE_OFFLOAD_REQUIRED`、`*-image-offload`） | 我们上行只有 Opus 音频，无图片输入；整套投影/上传/配额是纯复杂度 |
| `dsh-deepseek-llm-api-extensions` 字段注册表 | 我们只有一处请求构造点，`serde_json::json!` 直接写即可 |
| `dsh-agent-instructions`（AGENTS.md 链加载 / digest / touch 刷新） | 依赖文件系统与工作区概念，我们没有。**只借它的"字节预算 + 更具体优先"裁剪顺序**用于记忆段限长（broad 先丢、最具体才截断） |
| `dsh-session-title-llm` 等"每轮额外 LLM 调用"插件 | 每轮多一次计费调用；语音端只保留**抽取**这一处必要调用（且放后台，见 soul 文档 §5.2） |
| session-projections / checkpoint / 回放式确定性 fold | 我们不持久化会话日志（会话即连接），引入 fold 引擎复杂度收益为零 |
| pi-ai 多协议目录 / OAuth 登录 / 跨进程刷新锁 | 无 Node、无凭据存储组件；我们只需一个 OpenAI 兼容端点 + `api_key` |
| MCP 客户端的完整实现（`dsh-mcp-*`） | 设备端虽有 `mcp` 消息类型预留，但服务端做完整 MCP 客户端会放大延迟与安全面；本期不做（只做 §4.6 的 guard 底座） |

### 4.9 我们比 DSH 更容易做对的三件事（别浪费）

1. **有真实 HTTP status**：重试分类可建立在 status 上（§4.1），比 DSH 的抽象码更准；
2. **单进程、单入口**：不需要 DSH 的 session log / 投影 / 复盘机制来保证"重试后历史一致"——我们只需保证 `history` 是**不可变事实源**、模型可见面**每次重新组装**（这点 `session.rs` 已经是这么做的）；
3. **中文优先**：token 启发式必须为 CJK 校准（§4.3），而 DSH 的默认启发式在中文上是已知低估的。

---

## 5. 管理页 UI：借"信息架构"，不借"视觉实现"

> DSH 证据来自 asar 内客户端包（`lib/client.js` / `README.md`）。**Kuikly 约束**（`AGENTS.md` §5.8）：只能用官方组件，禁止自造样式。

### 5.1 DSH 的骨架与两个关键机制（实测）

**顶层骨架**（`dsh-client-ui-layout/lib/client.js` 注册 `root`，`dsh-client-ui-sidebar` 注册其子座）：

```
root (AppFrame)
├─ sidebar          single   ← 侧栏；其下再声明 sidebar.panellist(list) / sidebar.settings(single) ...
├─ main             keyed    ← 主区；会话、插件管理页各占一个 key
├─ rightbar         single
├─ shell.overlay    list     ← toast / 浮层
└─ shell.leading    single
```

**设置面板 4 层**（`dsh-client-ui-settings-general/lib/client.js`）：

```
sidebar.settings [single]  ← 面板本体（800×800 居中 modal）
├─ settings.action   list    ← 标题栏右侧动作
├─ settings.section  list    ← ★ 左侧导航 = 这个 ledger 的投影
└─ settings.onboarding list
settings.section [list]  ← 每个 section 再声明自己的子座
├─ general (order 0)  → settings.general.item [list]
├─ models  (order 10)
└─ plugins (order 15) → settings.plugins.tab [list]
```

实测几何（可作我们排版口径的参考）：面板 800×800、左导航 **188px**（gap 18 / padding 22 12 0）、导航项高 **40px**、标题栏高 **54px**、内容列 `padding: 0 24px 24px` 且**自己滚动**。

**两个关键机制**：

1. **「声明即授权」**（`dsh-client-ui-slots/README.md:46` 原文）：*"Declaring a slot is claiming it: the registering entry becomes the only entry allowed to render that key, and registering into an undeclared slot … throws at load."* → **未声明就注册 = 加载期报错**，不静默丢弃。
2. **排序确定**：`list` 按 `order` 升序、同值按注册顺序；`chain` 按 `priority` 升序、首个非 null 胜出。实测 order：`settings.section` = general 0 / models 10 / plugins 15；`settings.general.item` = developer-tools 15 / **current-version 100（版本号永远在底部）**。

**写页与读页的分工**（这条是我们的直接模板）：
- **侧栏 Plugins 页 = 写**（安装/启停/卸载/配置该插件自己的设置）；
- **Settings → Built-in plugins = 只读清单 + 运行状态**；
- 两页**互相指路**（写页的说明里写"内置插件列表与运行状态见 设置 → 内置插件"）。

### 5.2 我们的落地骨架（Kuikly 官方组件）

DSH 的"800×800 居中 modal + 遮罩 + 焦点陷阱"在 Kuikly 下**不可实现**（官方只有 `AlertDialog`，无尺寸可定制的模态容器，`AGENTS.md` §5.8）。因此**用整页 + 官方 Tabs+PageList**：

```
┌ 侧栏（AdminShell，macosApp 已有「测试台 / 配置」两项）─┬─ 内容区 ────────────────────────┐
│ 测试台                                              │ 能力总览            [刷新]      │
│ 配置                                                │ 共 9 个 · 5 运行中 · 1 降级 · 1 异常 │
│  ├ 总览           ← 新增                            │                               │
│  ├ 语音识别 ASR                                     │ 官方能力                      │
│  ├ 语音合成 TTS                                     │ ┌───────────────────────────┐ │
│  ├ 大模型 LLM                                       │ │ ASR  SenseVoice   运行中 ●│ │
│  ├ 全链路 AIUI    ← 现在完全缺失                    │ │      本地离线 · 新会话生效│ │
│  └ 记忆 / 灵魂    ← soul 文档 P0/P1                 │ ├───────────────────────────┤ │
│                                                     │ │ LLM  OpenAI 兼容  降级 ⚠ │ │
│                                                     │ │      未配 api_key，调用会 401│
│                                                     │ └───────────────────────────┘ │
└─────────────────────────────────────────────────────┴───────────────────────────────┘
一级：总览（清单）→ 二级：某能力 = tabbedPanel(配置 / 状态 / 日志)
```

- **总览页**：`largeTitleBar` + 计数摘要 + `groupedCard("官方能力"/"外部能力")` + `cardRow` 列表；
- **能力详情**：官方 `Tabs + PageList`（`AdminComponents.kt:tabbedPanel`，**必须显式传 `pageItemWidth/Height`**）分「配置 / 状态 / 日志」三个 `TabPage`；
- **导航表**：Kotlin 侧 `data class SectionDescriptor(id, order, label)` 常量表，集中定义、启动期校验 id 唯一与 order 稳定（**不模拟运行时槽位**，见 §5.7）。

### 5.3 状态呈现规范（照抄 DSH 的"只在偏离时打标"，但**补上原因**）

DSH 的两条规则（`ui-settings-plugin-inventory/README.md:36` 原文）：

> *"A small enablement tag marks only the states that differ from plainly enabled: disabled, conditional, provided by presets, and failed; a plainly enabled row carries no tag."*
> *"A colored root-fiber status dot marks the live phases no tag states: pending maps to idle, while loading and unloading map to ongoing; an active or failed fiber shows no dot."*

实测映射表（`ui-plugin-manager/lib/client.js`）：`PHASE_KEYS = {pending:"等待依赖", loading:"加载中", active:"运行中", failed:"异常", unloading:"卸载中"}`；`phase === null → "未运行"`；`!enabled → "已关闭"`；`PHASE_STATES = {pending:"idle", loading:"ongoing", active:"done", failed:"error", unloading:"ongoing"}`。

**我们的映射（§3.1 的 `Phase` → UI）**：

| 配置意图 | 实际 phase | 徽标 | 点 | 是否显示原因 |
|---|---|---|---|---|
| `enabled=true` | `Active` | 无（正常不打标） | 无 | — |
| `enabled=true` | `Degraded{reason}` | **「降级」** | 黄/`warn` | **必须显示 reason**（如"未配 embedding → 词法模式"） |
| `enabled=true` | `Failed{error}` | **「异常」** | 红/`error` | **必须显示 error + 日志入口** |
| `enabled=true` | `Loading` | 「加载中」 | 蓝/`ongoing` | — |
| `enabled=false` | `Disabled` | 「已关闭」 | 无 | — |
| `enabled=false` | 任何 | 「已关闭」优先 | 无 | — |

⚠️ **这里要明确超过 DSH**：DSH 的 `ui-plugin-manager/README.md:150` 自陈 *"Rows show a phase, not a reason — a failed row reads as failed without the Host's error text; the Host log has it."*（**只有阶段，没有原因**）。我们的 `PluginStatus` 里**已经带上 `last_error` 与 `degraded_reason`**，没有理由不显示——用户看到"异常"却查不到原因，是最容易招致"这功能没用"的形态。
另注意 DSH 的「运行中」与「已启用」是两种信息：**`enabled` 是保存的选择，phase 是实际状态**——这正是 §3.1 把两者分开上报的原因。

### 5.4 表单规范（六条，全部有 DSH 出处）

1. **草稿 + 单一保存点**：`blocked = !dirty || invalid || saving`；DSH 在卸载时**丢弃草稿且不提供"放弃"按钮**（`ui-primitives/lib/index.js` 注释：*"Leaving the page drops every staged edit, so the form discards on unmount and offers no discard control"*）。我们的 `ConfigPage.kt` 已是"顶栏保存按钮"形态，保持即可。
2. **覆盖语义按"存在性"而非"值比较"**（原文：*"That presence, not a value comparison, is what marks a field overridden: an override equal to the composition default is still an override."*）→ 字段旁给「已覆盖」+「恢复默认」；**空输入 = 清除 = 恢复继承**。这正好对应我们「`config.toml` 覆盖内置默认」的真实语义。
3. **hint 讲后果，help 放展开区**。DSH 文案范例（中文，直接可用）：*"单条命令允许运行多久，超时即终止。"*、*"超出部分会转存到临时文件，而不是被丢弃。"*、*"请填数字；留空表示使用默认值。"*、*"本部署的设置为只读。"*、*"该插件当前未加载，暂时无法配置。"*、*"本部署没有接受这些值，已保留供你修改。"* → **不要复述标签**（"超时时间"→"单条命令允许运行多久"）。Kuikly 无 tooltip，用 `vif` 展开的 `Text`（12px 次级色）作为 help 区。
4. **密钥字段：只报 presence，空草稿保留原值**。DSH 原文：*"The value never rides a response, so the control reports only whether one is configured and starts blank; a blank draft writes nothing, which keeps the stored key rather than clearing it."*；文案 *"不写入设置文件。留空表示保持当前密钥。"* / *"已配置密钥。"* / *"未配置密钥"*；校验 = trim 后非空且**全为可打印 ASCII**（`[\x21-\x7E]`），拒绝 `NAME=value` 与成对引号。服务端侧 *"Secret values cross in this direction only: no method here returns one"* + `redactSecrets: true`。
   → **与 §1.7 bug-1 / §3.4 完全一致**：`GET /api/config` 只回 `has_api_key: true`；`POST` 里**空字符串视为"未提供"**（这条必须显式写进契约，否则旧客户端回传空串会把密钥清掉）。
5. **revision 冲突**：DSH 的 code 是 `settings/conflict`，客户端文案 *"Someone else changed these settings while this card was open. Close it and reopen to edit the current values."*，且**保留草稿**。→ 我们 `POST /api/config` 冲突返回 409 + 该文案花色的中文（"配置在别处被改过，请重新加载后再保存"）+ 保留草稿 + 「重新加载」按钮。
6. **保存后的三档生效文案**（DSH 有 profile 级 HMR 判定与 toast：*"A profile with HMR recomposes before the operation completes; one without HMR, and a bundle a higher layer overrides, say so in a toast."*，文案 `restartNotice="更改将在下次启动生效"`、`overriddenNotice="{name} 已保存，但被更高优先级的配置覆盖，当前未生效"`）：
   - **「已保存并立即生效」** ← 对应 `HotReload::Live`（LLM/TTS/记忆/灵魂）；
   - **「已保存，新会话生效」** ← 对应 `HotReload::NextSession`（VAD 阈值等）；
   - **「已保存，下次启动生效」** ← 对应 `HotReload::RestartOnly`（`server.port` / `worker_threads` / 下行音频参数）。

   ⚠️ **诚实标注**：DSH **没有**字段级"需重启"标注（只有 profile 级 HMR 与 toast），所以 **§3.1 的 `FieldSchema.hot` 字段级三档标记是我们自己的设计**，不是借鉴结果——而它恰好修复了 §1.7 bug-2 那类"文档说生效、实际没生效"的问题。

### 5.5 动作与长任务（统一现有两套契约）

现状是**两套语义相同却互不兼容**的契约：`voices.rs` 的 `probing` 字段 + 轮询（HTTP），与 bench 的 `tts_test`/`llm_test` 结果帧（WS）。统一为 §3.4 的 action 模型后，UI 只需要一套组件：

| 动作 | 现状 | 统一后 |
|---|---|---|
| 探测音色目录 | `POST /api/tts/voices/refresh` + `GET /api/tts/voices` 轮询 `probing` | `POST /api/plugins/tts/actions/probe-voices` → job id；`GET .../actions/{job}` |
| 测试合成 / 测试识别 / 测试 LLM | WS 的 `tts_test`/`asr_test`/`llm_test`（**且 `asr_test` 把错误当识别文本**，§1.8#4） | `.../actions/test`，结果统一 `{state:"ok"\|"error", message, elapsed_ms, sample?}` |
| 拉取可用模型列表 | 无 | `.../actions/list-models`（§4.7） |
| 清空/导出记忆 | 无 | `.../actions/clear-memory` / `export` |

UI 呈现口径（照 DSH）：手动触发时按钮**禁用 + `ActivityIndicator`，且至少 400ms 防止闪烁**；成功**不弹 toast**；失败保留原内容 + toast；首次加载失败 → 就地错误 + 「重试」；计数摘要**只显示非零项**（`共 N 个 · X 运行中 · Y 异常`，0 的项不出现）。

### 5.6 破坏性操作

DSH 两档（都在 Kuikly 官方 `AlertDialog` 能力内）：
- **普通确认**：问句标题 + 一句后果 + 危险主按钮（例：*"卸载「{name}」？"* / *"卸载后它提供的功能会消失。"*）。危险色**不是新组件**，而是重绑主按钮的填充色 token；
- **强确认**（`RiskConfirmation`）：标题 + 说明 + **复选框闸门**（未勾选则确认按钮 disabled）。

⚠️ **Kuikly 差异**：官方 `Switch` 是"状态开关"，语义上不能当"我已阅读"的确认控件（会被误读为设置项）。→ 强确认降级为 **AlertDialog 两段式**（第一段讲后果 → 第二段确认），或对极端危险操作要求**输入资源名**匹配（后者是我们的设计，DSH 未找到证据）。
删除文案还要**分叉说明后果**（DSH：删除 provider 时区分"凭据另存所以保留"vs"同时删除已存密钥"）→ 我们"清空记忆"必须明确：**是否同时删除 `db_path` 文件、是否保留导出**。

### 5.7 明确**不照搬**（Kuikly 官方组件约束）

| DSH 做法 | 为什么不照搬 | 官方等价物 |
|---|---|---|
| 800×800 居中 modal + 遮罩 + 焦点陷阱 | 官方只有 `AlertDialog`，无尺寸可定制模态 | **整页 + `tabbedPanel`**（Tabs+PageList） |
| hover 态 / tooltip / HoverCard / 悬停延迟 | 无官方 tooltip，触屏也无 hover 语义 | **DSH 自己的 help 展开区**（`vif` + `Text`）——最接近的等价物 |
| 自绘滑动指示器 / 选中胶囊 | `AGENTS.md` §5.8 明令禁止 | 官方 `Tabs` + `indicatorInTabItem`（`tabbedPanel` 已封装） |
| 自绘旋转 loader / shimmer 骨架屏 | 无官方 shimmer 与相位控制 | 官方 `ActivityIndicator`；接受相位不对齐 |
| `Tag`/`Pill` 8 套 tone + `color-mix()` 动态淡色 | Kuikly 无 `color-mix` | 预置 tint 常量（`statusBadge` 已这么做）+ **限制 tone 数量** |
| `RiskConfirmation` 的复选框闸门 | 官方 `Switch` 语义不符 | AlertDialog 两段式 / 输入资源名 |
| pnpm 安装日志、registry 探测、包管理细节 | 我们没有 npm/pnpm 域，照搬会**引入无数据来源的 UI** | 不实现（我们的清单来自 `/api/plugins`） |
| 键盘漫游 / aria / `focus-visible` / `inert` / `role="img"`+`title` | 官方组件不暴露这些属性 | **一切信息必须有可见文字**：状态点必须跟文字（如"已配置/未配置"） |
| 0.5px 边框 / `backdrop-filter` / `corner-shape` / `-webkit-line-clamp` | Kuikly 无对应能力 | 固定 1~2 行 + 显式截断策略 |
| **槽位机制本身**（`slots.register/inject`、四类 kind、`declare module` 类型合并、`chain` 竞选） | 本项目 Kuikly 侧**没有运行时 UI 插件加载** | 只借"声明 + 有序注册表 + 加载期校验"的思想 → **Kotlin 编译期常量表**，不模拟 keyed/chain |
| `settings-models` 的模型目录编辑器（Fetch models / input types 复选 / Restore defaults） | 我们配置是 TOML 固定字段，照搬会产生**无法保存的字段** | 只借「测试连接 → 返回模型列表 → 下拉选择」 |

### 5.8 设计 token 口径（补进 `AdminTheme.kt` 的常量）

DSH 全部走语义 token（`--dsw-alias-*` / `--dsw-radius-*` / `--dsw-elevation-*`，focus ring 2px）。实测可对齐的间距/字号口径：

| 项 | DSH 口径 |
|---|---|
| 字段行 | `padding: 12px 0`，相邻字段 `border-top: .5px` |
| 输入框 | 高 `34px`、`padding: 0 12px`、圆角 `md` |
| 卡片头 | `gap: 14px`、`padding: 8px`；图标 `48px` |
| 标题 / 描述 | 标题 `14/20/500`（单行省略）；描述 `13/18`（1 行截断，三级色） |
| 开关 | `36×20`（thumb 16，位移 16） |
| Tag / Pill | Tag `11/17`、`padding 1px 8px`、radius 999；Pill 高 `24px` |
| 状态点 | 槽 `10px`、实心核 `6px` |
| 左导航 | 宽 `188px`、项高 `40px` |

`AdminTheme.kt` 已有 `AdminColors/AdminShape/AdminType/AdminSpace` 同类抽象 → 按上表补齐常量即可，**不新增组件**。

### 5.9 兼容与迁移（UI 侧）

- 新增「总览 / AIUI / 记忆 / 灵魂」页面只改两处：`ConfigCards.kt:22-34` 的 `pages` 列表 + 新增卡片函数（若沿用现有 `renderForm` 结构）；有 `SectionDescriptor` 表后收敛为一处；
- **`[aiui]` 卡必须一并补上**（当前 `commonMain` grep `aiui` 零命中，配置段完全不可见，见 §1.8#9）；
- 保存语义改为"部分更新"后（bug-1），**旧客户端（回传完整 JSON 但缺字段）依然安全**；但**密钥打码会破坏旧客户端**（它会把空串回传 → 按 §5.4 规则"空串=未提供"处理即可兼容），因此**打码与空串约定必须同批上线**。

---

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
4. **热切换失败必须保留旧实例**：否则"保存一个坏配置 → 语音服务不可用"。既有 TTS 范式已正确（`engine.rs:152-165`），统一时**不要**改成"先拆后建"。
5. **配置搬家（P4）是唯一破坏性改动**：`[tts].model → [tts.kokoro].model` 双读 + 保存写新结构；必须写迁移日志，并保留"未知键静默忽略"的既有行为（`config.rs:10-11`），否则旧配置直接解析失败。→ **已按此落地（§0.10）**：双读 + 迁移日志 + `rewrite_legacy_paths` 重写旧客户端补丁 + 未知键行为不变；风险点从"待处理"变为"有回归测试守着"。
6. **锁与阻塞**：`RwLock<Llm>` 每请求一次读锁（可忽略）；但 `refresh_from_disk` 可能加载模型（TTS 数秒）→ **必须在 `spawn_blocking` 内**（现状已如此：`bench.rs:107`）；且不要持有写锁做 IO。
7. **不要为了统一而统一**：`voices`/`firmware` 只纳入"描述 + 状态 + 动作"，**不重写其探测/托管逻辑**；`[aiui]` 的 `FullChainEngine` 也不要强行塞进 `Engines` 共享池（它是每会话有状态长连接）。
8. **文档漂移是本仓库的既有病**：本次盘点发现 3 处文档与代码矛盾（§1.7 bug-2）。→ 落地时**把文档断言变成测试断言**（引擎名断言、字段往返全量断言），否则修好一次还会再漂。
9. **协议与前端联动**：本次改动**不触碰** `protocol.rs` 的消息枚举。若 P3 要新增 bench 消息，必须遵守 `agents.md` §5.1（`ClientMessage` snake_case / `ServerMessage` lowercase + 显式 rename），且**`cargo check` 发现不了**，必须跑 `mock_client.py`。
10. **构建验证**：`cargo` 命令必须带 USTC 镜像覆盖且用 `~/.cargo/bin/cargo`（`agents.md` §3）；客户端改动必须 `cd client && ./gradlew :apps:h5App:publishWeb` 才能在本 GUI/浏览器里看到。

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
| `engine.rs` | `llm: RwLock<Llm>`；`tts_sig` 由注册表 `signature` 取代；`Engine s::new` 经 `PluginHost`；校验前移 |
| `config.rs` | `Config` 段不变（保持 `soul`/`memory` 扩展位）；`normalize()` 调用点 |
| `app/ws/mod.rs` | 路由加 `/api/plugins`、`/api/config/schema`、`/api/session/{id}/usage` |
| `app/ws/plugins.rs`（新） | `/api/plugins`、action 触发与 job 轮询 |
| `app/ws/config.rs` | **部分更新语义**（缺失=保持、`null`=清除）+ 密钥打码 + `revision` 409 + 全量字段往返测试 |
| `app/session.rs` | 会话开始调用 `host.refresh_from_disk()`；失败上报统一（不再只有 `warn!`）；`max_history` → token 预算 |
| `app/session/bench.rs` | 三条测试路径统一走 action/`refresh_from_disk`；`asr_test` 错误语义与 tts/llm 对齐 |
| `app/voices.rs` | 探测纳入 action 模型（保留旧端点做兼容） |
| `app/firmware.rs` | 纳入注册表（只加 descriptor + 状态） |
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
| `docs/soul-and-graph-memory-plan.md` | §5.4 的记忆注入位置改为 `after_history`（默认）+ 开关；`recall_max_chars` → token 预算 |
| 本文件 | 落地过程中把"设计"改写成"已实现"，并记录与计划的偏差 |

---

## 9. 一句话回答「怎么统一插件化 + 借 DSH」

> **统一** = 三张表：能力描述符注册表（可枚举 + 自带 `signature`/`build`/`fields`）、`enabled` 与 `phase` 分离的生命周期、`engine` + `[<cap>.<id>]` 的配置约定；五个引擎的 trait 一行不改，分阶段迁移、随时回滚。
> **借 DSH** = 借它的**护栏**（具名失败分类 + 有界退避 + `Retry-After`、有序 prompt 段与前缀稳定性、token/压力分解、密钥只存引用 + 表单只报 presence、revision 拒绝过期写入、单调 guard 与"拒绝也是工具结果"），**不借它的编排**（agent loop / subagent / PTC / 审批 / web search / `always` 重试）。
> **UI** = 借信息架构（写页与读页分工、声明式有序注册、只在偏离时打标、草稿 + 单一保存点、三档生效文案、AlertDialog 两段式强确认），**不借视觉实现**（modal 面板、hover/tooltip、自绘指示器与 loader、`color-mix` 动态淡色、运行时槽位机制）。
> **顺带**：盘点过程中发现两个真 bug（保存即静默重置三个配置段；TTS 热切换对设备会话根本没生效）——**它们比重构更紧急，建议先修**。


