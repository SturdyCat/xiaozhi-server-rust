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

### 智能上下文（标准行为，默认全部**开启**；不需要可显式关闭，关闭时零成本）

> 默认值口径（普通构建）：唤醒问候语、指令目录、技能目录、人格档案、记忆（含召回/抽取）、EchoMuse 入口、LLM 对话
> 一律 `enabled = true`；目录存在不等于模型已启用。**生产纯底座构建例外**：EchoMuse/Tuya 等业务配置、路由、worker 和静态 workflow 绑定强制失活，业务能力只能由运行期 WASM 插件提供。是否默认开启由「服务器代码默认值 + 管理台表单默认值 +
> 存量配置对齐」三层共同保证，改动其中一层必须同步另外两层（见[配置说明](<../config/AGENTS.md>)）。
> 需要凭据或模型文件的开关（`[aiui].enabled`、`[nlu].bert_enabled`）不在此列：它们缺依赖会启动/校验失败。

- **记忆多后端注册表**（`[memory].engine`）：
  - `graph`：内置图记忆——SQLite 原文事实源 + 辅助模型抽取摘要 + 汉字二元组倒排**词法召回**（零外网），注入原始 Q/A 作为背景资料；
  - `store`：纯存储——只存原文、不抽取、不召回，定位"先积累数据再决定"；
  - `rag`：本地文档 RAG——扫描 `[memory.rag].docs_dir`（默认 `/data/rag-docs`）对 txt/md 建词法索引（后台任务，重建不阻塞对话），设备可回答手册/备忘里的内容；全程零外网、零向量库。
  - 切换后端数据不丢（同一 `db_path`，graph/store/rag 各表独立）；`[memory.extractor]` 可为抽取单独路由更便宜的小模型。
- **灵魂人格档案**（`[soul]`）：内置预设（默认人格）打底 + 逐字段覆盖（留空 = 预设补）；`POST /api/soul/preview` 展开最终提示词（来源/预设/覆盖字段/字符数），保存前即可校验变量拼写。
- **唤醒问候语**（`[wakeup]`）：固件唤醒词命中后立刻上报 `listen state=detect`，服务器马上说 `greeting`（默认「我在」），**跳过 LLM**。`greeting` 支持换行多句，每次唤醒随机选一句（单行 = 单句；逐行 trim、空行丢弃、逐句限长）。EchoMuse 设备唤醒/按键由服务器代发 `detect`（`echomuse/routes.rs` 的 `start_turn`），同一问候机制覆盖两个承载。成品音频按音色三级复用：内存缓存 → 磁盘 `hot-audio/`（跨重启，存 TTS 原始 24k PCM）→ 真合成一次落盘，稳态零 LLM 零 TTS；换音色因缓存键含引擎签名/speaker/speed 自动重合成，多句池逐句各自缓存。`persist` 需要 `[tts].cache_entries > 0`。
- **模型唯一意图识别**（`[nlu]` + `[command]` 统一目录）：ASR 文本只由真实训练模型选择意图；不再通过关键词、模板规则、启发式评分或 LLM 工具重新选择技能。`[command].keywords` 是 `end_session` 的训练样例与目录来源，旧 `match_mode/max_chars`、`model_threshold/model_margin` 仅兼容读取，不参与识别。模型关闭时只允许普通聊天，缺失/加载/推理故障明确报告；`other`、低分、未知或未准入技能不能产生动作。模板只允许为模型已经接受的同一技能抽取槽位，缺参明确澄清，不编造参数。
- **配置驱动技能与稳定能力**：初始定义来自默认配置种子，显式清空技能不再被隐藏内置表加回；已有同名配置不会被偷偷替换成真实副作用。动作支持兼容的 `reply/end_session/mcp:<设备工具名>` 和引用受控能力的声明式 `plan`。条件、步骤、参数插值及任务服务见[技能系统](<../skills/AGENTS.md>)；使用已有能力新增技能无需改 Rust，新增模型标签仍须重新训练并验收。`end_session` 保留先告别再 MCP stop 及老固件/未知工具/超时降级关闭当前会话的交付兼容，这不是识别回退。MCP 工具名来自保存配置，不能由模型/槽位任意指定。模型质量与旧线上候选的拒识缺陷见[真实验证记录](<../../../docs/nlu-model-validation-20261101.md>)；构建候选不自动启用，见[构建说明](<../gateway/ws/nlu_build/AGENTS.md>)。
- **TTS 固定文本持久化缓存**（`hot-audio/`）：生产预取路径**逐句**登记完整身份（`HotAudioKey`），完整合成成功即落盘、跨进程复用——配置固定文本（问候语/告别语/技能应答/兜底话术）复用率最高，稳态零合成；测试台强制真合成不落盘；AIUI 云闭环无法避免云端已发生的合成。磁盘存的是**可直接播放的裸音频文件**：`<哈希>.pcm` = 24kHz mono S16LE 整句 PCM，无容器/头部/登记表（`ffplay -f s16le -ar 24000 -ch_layout mono` 即可播放）；下行采样率/帧长/编解码档不参与磁盘身份，改设备档位不再作废缓存，命中后按会话格式现场编码；上限 4096 文件 / 512MiB 总量 / 单句 8MiB，超限按 mtime 淘汰、损坏安全回退真合成。清理刻意最简：旧文案/旧音色的文件靠容量淘汰，旧格式遗留（`.hot`/`managed.json`）不读取也不自动清理，由使用者手动处理。
- **AIUI 全链路**（`[aiui]`，与本地级联互斥）：VAD 切段 → 讯飞云端 ASR + LLM + TTS 闭环；启用后本地引擎与 `[soul]`/`[memory]` 均不参与。

## WASM 插件平台（P0–P7g/P5b，feature 门控）

`wasm/` 与静态注册表并存（绞杀者迁移，P4 起逐域替换）：控制面插件化（注册/槽位/策略/效果），**模型推理与帧级计算铁律留原生**。契约唯一源是 `server/wit/plugin.wit`（宿主 bindgen 与 guest SDK 同源）；guest 侧统一走 [xiaozhi-plugin-sdk](<../../crates/xiaozhi-plugin-sdk/AGENTS.md>)（独立 crate，不进 server 依赖图）；改契约必须双侧同步重建测试夹具。测试夹具**按需本地构建、产物不入库**（`ensure_fixture`：缺失/源更新自动重建，git 忽略）：hello/trap/greeter-user 源在 [tools/wasm-fixtures](<../../../tools/wasm-fixtures/AGENTS.md>)（测试件），weather 源在 [plugins-src](<../../../plugins-src/AGENTS.md>)（真实插件源根）。生产分发：管理界面上传已构建组件（JSON `POST /api/plugins/upload` 或管理台桥 raw 通道 `POST /api/plugins/upload/raw?id=<id>`——共享 `adopt_bytes` 管线：大小上限→**[plugins] 配置门**（禁用 id 403）→隔离验证→采用→热装载→重启重装；重启经数据卷扫描重装即注册）。

- 宿主导入面：log / now-ms / **kv-get**（P1a；未声明 `kv` 权限 → deny-as-absent，温和拒绝）。
- 效果面：log / kv-put / **http-get**（两段式：宿主执行后**按步批量**回灌 `http.responses` 事件——一步的全部请求合成一个数组，深度按步计、上限 4；权限门校验 URL host ∈ `http-out:<host>` 白名单；HTTP 经 `HttpFetcher` 抽象，测试注入脚本实现零真实网络）。
- 扩展点替换演练已落地：官方默认 vs 用户自建同一扩展点，首选切换/卸载回退/注册-回收全链路有真实 wasm 测试（`usermade_plugin_replaces_official_and_falls_back_on_unload`）。
- 首个真实源根插件：[plugins-src/weather](<../../../plugins-src/weather/AGENTS.md>)（`skill.weather` 官方默认，纯决策瀑布 + 与原生共享 parse.rs 单源；回放测试 `wasm/tests/weather.rs`）。
- **生产接线（P2c）**：`[weather].backend = "native"|"wasm"`（默认 native，重启生效）——wasm 经 `wasm/bridge.rs` 事件桥（扩展点解析/geo 预填/事件派发/kv 结果通道），skills 层零改动；缺依赖明确报错不静默回退。
- **观察者挂钩（P2d）**：wasm 插件经 workflow 绑定进会话 hook 位（start/link.nlu.llm/end）——`try_dispatch` 单步派发不执行、`spawn_deferred` 异步执行效果（fire-and-forget 不回灌）；陷阱降级只告警，永不打断对话轮。
- **配置门（P3）**：`[plugins]` `enabled` 总闸 + `entries."<id>".enabled` 每插件启停（缺省启用；RestartOnly）——装配扫描与启动引导都按它过滤，禁用产物不装载不引导（落盘保留，重新启用即恢复）。`entries."<id>".artifact` 可选持久化 SHA-256 内容钉版；上传采用经 ConfigStore 回写，重启摘要不符即跳过，缺省 pin 兼容旧配置放行。
- **asr.provider 事件桥（P4a/P4b）**：utterance 级 `asr.transcribe` 事件（f32-le PCM base64 payload）→ 插件 kv `asr-result:<req_id>` 结果通道（JSON `{text}`）；原生适配器 `asr/wasm_engine.rs` 实现 `AsrEngine`，`[asr].engine = "wasm-remote"` 经 `asr.wasm-remote` 注册表描述符与 boot 装配接线；缺宿主/插件显式报错，不静默回退 SenseVoice。hello 夹具带探针事件与多扩展点声明。
- **tts.provider / llm.provider 控制策略（P4c）**：`tts.wasm-policy` 包装 native Kokoro，`tts.prepare` 只返回文本/速度/音色控制；`llm.wasm-policy` 包装 native OpenAI，`llm.prepare` 只接收受限元数据。WASM 不承载 TTS PCM、完整历史、HTTP/SSE 或模型响应；策略缺失/结果非法显式失败，默认 native 路径不变。
- **vad.provider 会话级控制策略（P4d）**：`vad.wasm-policy` 只在 `VadPool` 借出 native Silero 前执行一次 `vad.prepare`，返回受限阈值/时长控制；逐帧 `accept`/`flush`、PCM 和 segment callback 全部留 native。策略结果及插件 id 进入池签名，缺宿主/插件/非法结果显式失败，EchoMuse device-VAD 继续旁路；无 sherpa 时显式拒绝该策略，不构造 MockVad。
- **connector.tuya 控制策略（P5a）**：`[tuya].engine = "wasm-policy"` 经 `connector.tuya.prepare` 选择有限候选索引；WASM 不接收 local_key、IP、云凭证、原始 DP/报文或 socket。native `TuyaEngine` 继续负责 rustuya LAN 会话、set→status 回读、DP cache、互斥、超时和错误归类；策略缺失/结果非法 fail-closed，默认 `native` 零行为变化。
- **typed lan-socket effect（P5b）**：WIT 提供 `effect-lan-status`/`effect-lan-set-dp`，仅传不透明设备句柄、数字 DP 与有界 JSON；`lan-socket` 权限门 + `LanSocketQueue` 有界 drain，队列满/非法值显式失败；命令带仅宿主内部的 `plugin_id` 路由元数据（不跨 WIT），native Tuya consumer 已由 `device/tuya/worker.rs` drain，复用真实 rustuya 数据面并将有界结果回写插件 KV；原始 socket/密钥/报文不跨边界。
- **device.echomuse 会话入口策略（P6a）**：`[echomuse].engine = "wasm-policy"` 仅经 `device.echomuse.prepare` 选择受限欢迎文案；WS/keepalive/hub、bounded-turn、PCM、BLE、shell/PTY、鉴权和设备状态全部留 native，缺策略或非法结果显式终止入口，默认 `native` 零行为变化。
- **设备公共能力边界（P9 规划）**：重启、音量、设备状态等属于接入硬件提供的公共能力，不是服务器自身系统能力。跨 EchoMuse/Xiaozhi ESP/ESPHome 复用的 capability ID、参数校验和纯 helper 位于 `server/src/device/capabilities/`；业务插件/技能应复用 native 公共模块，不能重复实现。WASM 只在低频、可替换 control path 中可选介入，PCM/VAD/推理/WS/keepalive/PTY/Tuya LAN 等 hot path 不经 WASM。
- 验证：`cargo test --features wasm-plugins`（默认 feature 零影响）；阶段与证据见[WASM 插件平台](<../../../docs/designs/wasm-plugin-platform.md>) §9–10。


## 直属下级

- [registry](<registry/AGENTS.md>)：能力/实现/字段元数据、描述符与宿主、配置约定。
- [wasm](<wasm/mod.rs>)：WASM 插件平台 P0–P7g/P5b（契约 `server/wit/plugin.wit` + 沙箱宿主 + 扩展点解析/回退 + waterfall + kv/http 闭环 + typed lan-socket effect/有界 native command queue + weather 试点回放 + 配置门/摘要钉版 + ASR 远端适配器 + TTS/LLM/VAD 控制策略 + Tuya connector 控制策略 + EchoMuse 会话入口控制策略 + wakeup.strategy/prompt.soul/debug.policy/skill.policy/memory.policy/nlu.catalog 有界策略桥），默认 feature 关闭；guest 侧见 [crates/xiaozhi-plugin-sdk](<../../crates/xiaozhi-plugin-sdk/AGENTS.md>)，设计与阶段见[WASM 插件平台](<../../../docs/designs/wasm-plugin-platform.md>)。
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
