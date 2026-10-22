# 插件层指引

继承[服务端指引](<../../AGENTS.md>)。能力 trait、配置和实现按能力就近组织，会话层只依赖抽象；具体能力约束看下方直属下级入口，本级不重复维护。

- 新实现遵守[注册表规范](<registry/AGENTS.md>)，同步 engine 声明、描述符、字段元数据和一致性测试；不复活已删除的中心 TTS 枚举/match 分发。
- enabled 是配置意图，phase 是运行状态，两者不能合并。必需能力构建失败快速失败，可选能力降级；热重建失败保留旧实例并报告原因。
- 历史设计见[插件统一导航](<../../../docs/designs/plugin-architecture-unification.md>)；设计中的统一 action/job、动态 UI 等未实现提案不当作扩展要求。

## 插件装配与扩展

[registry](<registry/AGENTS.md>)声明能力/实现/字段，[host](<host.rs>)校验与上报，[Engines](<../runtime/mod.rs>)持共享实例与刷新逻辑。不要把三者职责合为一个中心分发器。

- 本地模型跨会话共享，有状态 VAD 每会话借独立实例并通过[实例池](<vad/pool.rs>)复用；“每会话新建”不等于每次都重新加载模型。
- 描述符负责 selected/signature/validate/build；签名含凭据，绝不打印。先构建成功再替换，不能先拆旧实例。
- 必需/可选分级以注册表和 host 实现为准；AIUI 全链路开关是流水线模式，不硬塞入普通共享 TTS/LLM 模型池。
- hot 字段/段定义只在注册表，刷新与 UI schema 消费同一来源。字段元数据的 default_hint 是展示，不是运行默认值。
- capability 聚合供 plugins 与 health 同源使用，纯配置 plan 不混入模型构造或运行探测。

## 可选上下文功能

> 本文为当前功能使用说明；实现细节以邻近源码、配置样例与测试为准。

### 智能上下文（可选能力，默认全部关闭、零成本）

- **记忆多后端注册表**（`[memory].engine`）：
  - `graph`：内置图记忆——SQLite 原文事实源 + 辅助模型抽取摘要 + 汉字二元组倒排**词法召回**（零外网），注入原始 Q/A 作为背景资料；
  - `store`：纯存储——只存原文、不抽取、不召回，定位"先积累数据再决定"；
  - `rag`：本地文档 RAG——扫描 `[memory.rag].docs_dir`（默认 `/data/rag-docs`）对 txt/md 建词法索引（后台任务，重建不阻塞对话），设备可回答手册/备忘里的内容；全程零外网、零向量库。
  - 切换后端数据不丢（同一 `db_path`，graph/store/rag 各表独立）；`[memory.extractor]` 可为抽取单独路由更便宜的小模型。
- **灵魂人格档案**（`[soul]`）：内置预设（默认人格）打底 + 逐字段覆盖（留空 = 预设补）；`POST /api/soul/preview` 展开最终提示词（来源/预设/覆盖字段/字符数），保存前即可校验变量拼写。
- **唤醒问候语**（`[wakeup]`）：固件唤醒词命中后立刻上报 `listen state=detect`，服务器马上说 `greeting`（默认「我在」），**跳过 LLM**。`greeting` 支持换行多句，每次唤醒随机选一句（单行 = 单句；逐行 trim、空行丢弃、逐句限长）。EchoMuse 设备唤醒/按键由服务器代发 `detect`（`echomuse/routes.rs` 的 `start_turn`），同一问候机制覆盖两个承载。成品音频按音色三级复用：内存缓存 → 磁盘 `hot-audio/`（跨重启，存 TTS 原始 24k PCM）→ 真合成一次落盘，稳态零 LLM 零 TTS；换音色因缓存键含引擎签名/speaker/speed 自动重合成，多句池逐句各自缓存。`persist` 需要 `[tts].cache_entries > 0`。
- **技能级联**（`[nlu]` + `[command]`，统一编译进 `skill/`）：ASR 文本按三级处理——①规则模板命中即本地执行；②规则未命中走本地轻量模型打分（零外网），达 `model_threshold` 且领先 `model_margin` 也直接执行（封闭槽位容忍"包含唯一值"的口语化说法，含否定词/多值歧义一律不执行、交 LLM）；③都未命中才调云 LLM，LLM 若识别为技能会以 `execute_skill` 工具调用回到目录校验执行（槽位/动作白名单由服务端配置决定，LLM 不可指定工具），否则正常聊天走 TTS。技能 `action.kind`：`end_session`（见下）/ `reply`（只应答）/ `mcp:<设备工具名>`（应答后把槽位派发到设备声明的同名 MCP 工具，清单明确不含则零调用跳过）。`end_session` 沿用指令闸门语义：先说告别语再**用 MCP 让设备结束聆听**（`tools/call` → `self.audio_listener.stop`：设备回待唤醒、音频通道保留、下次唤醒不重连）。**调用决策由每设备 MCP 工具清单驱动**：清单含 stop 才调用；不含（老固件）→ 零次调用直接降级断开（管理台 MCP 工具页有「指令闸门兼容」标记）；清单未知 → 试探调用 + `-32602` 降级。`[command]` 自身仍是 `match_mode="exact"`（整句相等、去标点剥语气词，不会误伤「关闭闹钟」）+ `max_chars=5` 长度闸门。
- **TTS 固定文本持久化缓存**（`hot-audio/`）：生产预取路径**逐句**登记完整身份（`HotAudioKey`），完整合成成功即落盘、跨进程复用——配置固定文本（问候语/告别语/技能应答/兜底话术）复用率最高，稳态零合成；测试台强制真合成不落盘；AIUI 云闭环无法避免云端已发生的合成。磁盘存的是**可直接播放的裸音频文件**：`<哈希>.pcm` = 24kHz mono S16LE 整句 PCM，无容器/头部/登记表（`ffplay -f s16le -ar 24000 -ch_layout mono` 即可播放）；下行采样率/帧长/编解码档不参与磁盘身份，改设备档位不再作废缓存，命中后按会话格式现场编码；上限 4096 文件 / 512MiB 总量 / 单句 8MiB，超限按 mtime 淘汰、损坏安全回退真合成。清理刻意最简：旧文案/旧音色的文件靠容量淘汰，旧格式遗留（`.hot`/`managed.json`）不读取也不自动清理，由使用者手动处理。
- **AIUI 全链路**（`[aiui]`，与本地级联互斥）：VAD 切段 → 讯飞云端 ASR + LLM + TTS 闭环；启用后本地引擎与 `[soul]`/`[memory]` 均不参与。

## 直属下级

- [registry](<registry/AGENTS.md>)：能力/实现/字段元数据、描述符与宿主、配置约定。
- [llm](<llm/AGENTS.md>)：Responses API 请求、失败分类与重试、用量口径。
- [memory](<memory/AGENTS.md>)：graph/store/rag 后端、召回预算与保留策略；集成、尽调与路线见[人格与记忆集成](<../../../docs/designs/soul-and-graph-memory-plan.md>)。
- [soul](<soul/AGENTS.md>)：人格档案、预设覆盖与 preview 语义。
- [tts](<tts/AGENTS.md>)：流式 PCM 契约、Kokoro 与讯飞 AIUI。
- [health](<health/AGENTS.md>)：运行期探测与能力状态聚合。
- [prompt](<prompt/AGENTS.md>)：公共提示词段组装与 DSH 机制调研边界。

## 历史设计与实施资料

- [统一插件化背景、改造前盘点与实施风险](<../../../docs/designs/plugin-architecture-unification.md>)
- [P1 健康与记忆接口、实施及共享护栏](<../../../docs/plans/p1-round1-design.md>)
- [文件规模与测试分离记录](<AGENTS.md#文件规模与测试分离记录历史>)

以上历史记录保留原章节编号与当时结论；当前维护约束以各能力目录 `AGENTS.md`、源码与测试为准。

---

## 文件规模与测试分离记录（历史）

> **历史设计 / 实施记录**：由原《插件架构统一》§0.7–0.8 并入；文中 `AGENTS.md` §N 指重组前的旧长文，现行约定见[仓库指引](<../../AGENTS.md>)。

### 0.7 第三批：文件规模整改（逻辑文件 ≤ 600 行）

**背景**：新增规范 `AGENTS.md` §5.10 —— 任何 `.rs`/`.kt` 逻辑文件 ≤ **600 行**。审计发现 4 个文件超标，
其中 2 个（`registry.rs`、`ConfigFormState.kt`）是前几批自己写大的。

| 整改前 | 行数 | 整改后 | 行数 |
|---|---|---|---|
| `plugins/registry.rs` | 989 | `registry/{mod,fields,impls,tests}.rs` | 399 / 450 / 314 / 212 |
| `gateway/ws/config.rs`（逻辑 422 + 测试 553） | 975 | `config.rs` + `config/tests.rs` | 424 / 550 |
| `workflow/session/mod.rs`（逻辑 695） | 733 | `session.rs` + `session/reply.rs` | 443 / 310 |
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
| 空测试块删除 | 1 | `workflow/session/mod.rs` 的 `mod tests {}`（零断言占位），连同多余空行一并删除 |
| 已合规 | 2 | `gateway/ws/config/tests.rs`、`plugins/registry/tests.rs`（第三批已外移） |

**规模对比**（行数取本批开始前的实测值；"测试"含外移后新增的 `//!` 文件头）：

| 文件 | 前（逻辑+测试） | 后（逻辑 / 测试） |
|---|---|---|
| `workflow/downlink/mod.rs` | 591 | 331 / 264 |
| `device/voices.rs` | 576 | 508 / 72 |
| `plugins/aiui/mod.rs` | 498 | 335 / 167 |
| `plugins/host.rs` | 490 | 338 / 167 |
| `workflow/protocol/protocol.rs` | 441 | 341 / 103 |
| `runtime/mod.rs` | 433 | 340 / 101 |
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
`workflow/downlink/tests.rs` 7 处等）。用 `git show HEAD:<file>` 重建"原始内联块"后逐文件对比漂移计数：
**14 个文件搬移前后完全一致**（7→7、4→4、3→3…，另有 `ws/config.rs` 由 3→0，因该测试文件第三批已格式化），
证明搬移是**纯位移**；剩下 5 个测试块是本项目前几批**新写的代码**，按"新增代码保持 fmt-clean"的既定口径
已对其执行 `rustfmt`。全库统一格式仍留作独立提交（本 crate 约 30 个文件有既有漂移，本批刻意不动）。

**过程记录（脚本化抽离的两个必备判定）**：
1. **不要用大括号计数找测试块结尾**：`firmware.rs` 里的字符串 `"{broken"` 会让 `{`/`}` 计数失衡
   （实测直接报 `unmatched braces` 并中断）。改用"**最后一个非空行必须是顶格 `}`，且其后无代码**"后一次通过。
2. `#[cfg(test)]` 之后可能是三种形态：`mod tests {`（待外移）、`mod tests;`（已外移）、单行 `mod tests {}`（空块）——
   三者都要单独识别，否则前者会被误报为"异常"（实测把 2 个已合规文件报成错误）。
   脚本先**全量分析、零异常才写入**，避免改到一半留下坏状态。
