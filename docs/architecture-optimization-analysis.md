# xiaozhi-server-rust 功能与 UI 分模块优化分析

> 方法：参考 Golang 版 [`xiaozhi-esp32-server-golang`](https://github.com/hackers365/xiaozhi-esp32-server-golang) 的优秀设计，对 Rust 版 `xiaozhi-server-rust` 做分模块优化分析。
> 团队：软件开发专家团（主理人 齐活林 / 架构师 高见远 / 产品经理 许清楚）。
> 日期：2026-10-12。定位前提：Rust 版 = 离线 ASR/TTS(sherpa-onnx) + 云端 HTTP LLM 的**单二进制 Docker** 小智服务端，固件协议严格对齐(v1/v2/v3)，N5105 低功耗主机。Golang 版 = 全功能多引擎多协议重型实现。

---

## 0. TL;DR

- **Rust 已显著领先 Go 的三处**：① 配置系统（schema 驱动 + 热生效三粒度 + 密钥闭环）；② 流式下行（DownlinkPacer + 水库 + 下载预算三段缓冲，针对 ESP ~20 包硬约束的精确解）；③ 离线引擎编排（闭枚举注册表、零类型擦除）。
- **真正值得借鉴的差距**：配置向导化/就绪度、配置内联测试 + 首包延迟可视化、全局延迟监控大盘、设备消息注入、文本/控制侧优先级变换管线、VAD 实例池化、轻量事件总线解耦、记忆/MCP 多后端可插拔。
- **一律不搬（违背离线单二进制定位）**：MQTT-UDP 传输、OpenClaw 云端智能体编排、第三方 MCP 市场、Dify 云端 RAG、激活/OTP 流程。
- **P0（立即做，低成本高收益）**：配置向导化+就绪度+改动影响高亮；配置内联测试 + 首包延迟 HTTP 端点；VAD 实例池化；文本/控制侧优先级变换管线；轻量事件总线解耦；场景化联调 + registry 漂移护栏。

---

## 1. 总判断

### Rust 已领先 Go 的设计（不必抄）

| 维度 | Rust 现状 | 为何领先 |
|------|-----------|----------|
| 配置系统 | `config.rs` 单一权威字段元数据 + `registry/` 描述符含 `hot/signature/validate/warn_if/selected` + `GET /api/config/schema` 暴露；`app/ws/config.rs` 的 `SECRET_PATHS` 三处同改（服务端 redact 只留 `has_*`、空串=保留、buildConfigJson 用 `putSecret`） | Go 用 interface+工厂按 engine 字符串选、build-tag 多默认值变体、`redis/*` 外部持久化；Rust 编译期闭枚举无 panic、密钥语义闭合 |
| 流式下行 | `downlink/sender.rs`(DownlinkPacer) + `reservoir.rs`(水库) + `framer.rs`(跨分片复用重采样/Opus) + `prefetch.rs`(DownloadBudget 背压) | Go 用 `output_segmenter` 优先级插件管线切句；音频侧 Rust 专用 Pacer 是针对 ESP 20 包硬约束的精确解，更优 |
| 离线引擎编排 | `registry/mod.rs` 的 `PluginDescriptor` + `Built` 闭枚举 + `Capability` 枚举，新增实现=加 1 条描述符+字段表+漂移护栏测试 | Go 宽接口 + 运行时反射/DI(Eino)；Rust 闭枚举无类型擦除、更稳 |

### Rust 真正缺的（Go 有借鉴价值）

- 横切解耦（事件总线）——统计/历史内嵌 session，新增埋点要改 session。
- 规模化运维——VAD 模型每次会话 `create` 加载（README 自承待优化），无资源池。
- 可插拔后端抽象——记忆为单实现 SQLite 图，未抽象成多后端 provider；MCP 仅 server→device 硬编码 `tools/call`。
- **UI**：首登引导/配置就绪度、配置内联测试 + 首包延迟、**全局延迟监控大盘**、设备消息注入写操作。
- 高级特性（声纹/动态TTS、本地RAG、音乐）空白。

---

## 2. 功能与架构分模块分析

### ① 配置系统
- **现状(领先)**：`Config`/`EngineSpec`/`canonical_engine()`（未知 engine 报错非静默回退）+ `normalize()` 旧键兼容；`registry/mod.rs` 的 `HotReload{Live,NextSession,RestartOnly}` + `field_hot/section_hot`（取最严格）+ `FieldSchema` + `schema_json()`；`app/ws/config.rs` 的 `SECRET_PATHS`/`rewrite_legacy_paths`/`impact{}`。
- **Go 参考**：`UserConfigProvider`（角色/设备维度切换）、`defaults_manager.go`(build-tag 多默认值)、`redis/*` 外部持久化、5 步向导 + 逐模块启用/禁用 + 设为默认。
- **可借鉴**：「逐模块启用/禁用 + 一键恢复默认」运维心智；5 步向导形态——但 Rust 的 `schema_json()` 已数据驱动，前端可**自动生成向导式分步表单**（比 Go 手写更优）。
- **不适合照搬**：build-tag 多默认值变体、把激活/OTP/角色塞进 provider、Redis 后端。
- **建议**：**P0** 基于 `schema_json()` 做向导式分步表单 + 每 capability tab + 一键恢复默认 + 改动影响高亮（`impact` 已就绪）。**P1** 加配置预设(preset) + 每设备 overlay（`Config::load` 增 device overlay），不引 Redis。
- **风险**：部分更新 + 空串保留密钥极易回归"静默清空密钥"，须复用 `SECRET_PATHS` 三处同改护栏 + 回归测试。

### ② 引擎插件架构
- **现状(领先)**：`PluginDescriptor{id,capability,local,hot,requires,fields_path,fields,signature,validate,build,warn_if,selected}` + `REGISTRY` 静态表 + `Built` 闭枚举 + `Capability`(Asr/Vad/Tts/Llm/FullChain/Voices/Firmware/Memory/Soul/Command/Wake)；五引擎 trait 一行未改。
- **Go 参考**：interface+工厂按 engine 字符串选；宽接口 + 运行时反射/DI。
- **可借鉴**：把非引擎能力(Voices/Firmware/Memory/Soul)显式纳入统一描述符动作模型（Rust 已列可强化）。**不**改回 trait object 路由。
- **建议**：**P0** `PluginDescriptor` 加 `degrade_when_unavailable`/`per_session` 元数据，为⑤⑥提供支撑。**P1** 补全 Memory/Soul/Command/Wake 描述符 + `check_all(cfg)->Vec<Phase>` 健康自检端点。
- **风险**：闭枚举扩展需全量 match（已有护栏测试制衡）。

### ③ 流式下行与音频节奏
- **现状(领先)**：DownlinkPacer 实时节拍护 ESP ~20 包队列；ReservoirSink/Source 水库(多秒缓冲)；StreamingFramer 跨分片复用重采样/Opus 编码器(无爆音)；DownloadBudget 按秒预算 + 背压。三段缓冲"各管一事勿互相替代"。
- **Go 参考**：`output_segmenter.go`(priority=200) + `streamtransform/pipeline.go` 的 `Registry` 按优先级排序的 `Transformer` 管线。
- **可借鉴**：Go "优先级 Transformer 管线"用于 **文本/控制侧**（LLM 字流→断句→指令闸门/唤醒/记忆注入），抽成有序管线（指令闸门优先级最高先于 LLM）。**音频侧 Pacer 已最优，不动**。
- **建议**：**P0** 文本/控制侧引入轻量优先级变换管线（复用 prefetch 的 abort 语义），新增 `app/pipeline.rs`。**P2** 音频侧无需改动。
- **风险**：早退逻辑(command/wakeup 当前直接 return)塞进 transformer 的 Stop 信号须保证语义不丢。

### ④ 记忆系统
- **现状(领先)**：`plugins/memory/mod.rs` 的 `MemoryProvider` trait + `MemoryBackend{Graph,None}` + `NoopMemory` 默认零成本；`graph.rs` 的 `GraphMemory`(SQLite 事实源+摘要+倒排索引+后台抽取，`recall_budget_ms=300` 静默降级)。
- **Go 参考**：`memory/base.go` 的 `MemoryProvider` 接口 + nomemo/mem0/memobase/memos 多后端可插拔。
- **可借鉴**：多后端可插拔**接口形态**（Rust 已有 trait，把 Graph 作为其一）；`GetProviderByType` 按 type 选 + 默认 None → 映射 `MemoryBackend` 扩展。**不**引入 mem0/memobase 等**云端**后端（违离线）。
- **建议**：**P1** 将 `GraphMemory` 挂到 `MemoryProvider` 实现注册，引入 `memory_backends` 注册表（类比引擎 registry）。**P1** 新增"纯存储后端"(仅 record 不抽取)。**P2** 可选本地 embedding 语义后端。
- **风险**：保证默认 None/Graph 零成本契约；切换后端旧库迁移(SQLite schema 版本)。

### ⑤ 事件总线与可观测性解耦
- **现状**：会话状态机 + 直接调用，无事件总线；prefetch/reservoir 有 per-round stats 仅供日志。
- **Go 参考**：`eventbus/eventbus.go` 的 `asaskevich/EventBus` 单例 `eventbus.Get()`，解耦统计/历史/消息持久化。
- **可借鉴**：横切关注点(统计/历史落盘/对话记录/会话清理)通过事件解耦——Rust 明显短板。**不**照搬 asaskevich/EventBus(字符串主题+反射类型安全差)，用 `tokio::sync::broadcast` 零外部依赖。订阅者应是本地统计/日志/可选 webhook，不引 Redis。
- **建议**：**P0** 引入轻量事件层：定义 `ServerEvent`(SessionStart/End/MessageAdded/TtsRound/Error) 走 broadcast；session 只 emit，统计/历史/可观测作独立 subscriber，新增 `app/events.rs`。**P1** 基于事件汇总 `/api/metrics`(QPS、每轮音频吞吐比、水库峰值、VAD/ASR 耗时)；历史落盘作可选 subscriber(默认关，本地 SQLite/文件)。
- **风险**：broadcast 无缓冲丢早期事件，注意订阅者注册时序；迁移保留每轮日志口径。

### ⑥ 资源池 / VAD 池化与性能
- **现状**：README 自承"VAD 模型每次会话 `VoiceActivityDetector::create` 会加载，连接数大时建议池化(待优化)"；ASR 离线逐段；未启 sherpa 时下行空帧。
- **Go 参考**：`util/resource_pool.go` 通用 `ResourcePool`(Resource/ResourceFactory；PoolConfig{MaxSize,MinSize,MaxIdle,AcquireTimeout,IdleTimeout,ValidateOnBorrow/Return})。
- **可借鉴**：VAD 模型加载贵 → 专用 VAD 实例池(固定容量 Acquire/Release + 空闲超时回收)，直接回应 README 待优化项。**不**照搬通用 ResourcePool 全功能，用更简 `VecDeque<Arc<Vad>>`+semaphore 或 deadpool 风格仅限模型实例。
- **建议**：**P0** 实现 VAD 实例池(固定上限+空闲回收)，session 借/还，落点 `plugins/vad/` 加 `VadPool`；ASR 若每会话也 reload 同理。**P1** 池 Stats 接入⑤ metrics。
- **风险**：ONNX 模型实例是否线程安全(多线程共享 vs 每线程一个)——池化前须验证 sherpa-onnx 并发约束，错误共享致推理错乱。

### ⑦ MCP
- **现状(已落地)**：`app/mcp.rs` 的 MCP **Client**，`tools/call`(含 `self.audio_listener.stop` 命令闸门、wakeup 问候、错误识别老固件并降级)；`app/session/mcp.rs`、`app/ws/mcp.rs`。
- **Go 参考**：MCP **市场**(第三方聚合/导入/热更新)；设备/智能体维度远程调用调试。
- **可借鉴**：「设备/智能体维度 MCP 远程调用」——握手阶段设备上报可用 tools，server 据上报能力做工具发现(而非硬编码 `audio_listener.stop`)。「已导入服务启用/禁用」映射到 Rust 的 capability/command/wakeup 开关模型。**不**搬 MCP 市场(多租户运维平台，离线单二进制不需要且依赖外网)。
- **建议**：**P1** 引入"设备 MCP 工具清单"发现：握手上报可用 tools，server 缓存按需调用(替代硬编码常量)，`app/mcp.rs` 加 device-tools registry + session 存 per-device tools。**P1** command 闸门/wakeup 问候统一到"MCP 工具调用结果驱动"动作模型。**P2** 可选本地 MCP 服务目录(自托管，非第三方市场)。
- **风险**：设备工具清单需固件配合(旧固件无上报)，须保留硬编码常量降级；工具调用超时必须严格，避免阻塞对话。

### ⑧ 传输协议（MQTT-UDP）
- **现状**：仅 WebSocket(固件协议严格对齐，`tests/mock_client.py` 联调)，二进制 v1/v2/v3。
- **Go 参考**：`app/server/mqtt_udp/` + `app/mqtt_server/` 给低功耗设备用 MQTT+UDP 省电。
- **判断**：**基本不适合**离线家庭/桌面场景。Rust 定位(N5105 Docker + 桌面/开发板固件)WebSocket 已足，引入 MQTT broker+UDP 会显著扩大二进制体积与运维面，违背"单二进制离线简单"原则。
- **建议**：**P2(暂不投入)** 保持 WS 单一传输；如确有低功耗需求先评估"WS+长连接保活+服务端主动推送"能否覆盖，再决定是否引入 MQTT-UDP 适配层(仅可选 feature)。强化现有 WS：基于 `mock_client.py` 固化 v1/v2/v3 协议回归。

### ⑨ 高级特性路线图
逐项判断(契合离线单二进制定位)：
- **声纹识别 + 动态 TTS 切换**：部分契合。离线声纹(本地说话人聚类)可行；Rust 已有多 TTS(kokoro/xfyun)，可做按说话人切音色。**P1**(多用户场景)。
- **OpenClaw 智能体(关键词路由)**：不契合。云端多智能体编排，离线无此需求；Rust command 闸门已覆盖关键词拦截刚需。
- **声音复刻**：部分契合(本地 TTS 音色微调/克隆需模型，离线可行但重)。**P2**。
- **RAG(Dify/RAGFlow/WeKnora)**：部分契合但选本地。Rust 记忆已是本地知识沉淀；真正文档 RAG 需本地 embedding+向量库(离线)。把 `GraphMemory` 召回作 LLM 上下文增强(已实现 `GetContext`)。**P1** 做本地文档索引可选后端(复用记忆 trait)。**不对接 Dify 云端**。
- **音乐播放/MCP Audio Server**：不契合核心语音，属增值；离线可做本地音频播放 MCP 工具。**P2**。
- **激活/OTP**：本期明确不做(README)，未来对接官方固件激活再开。**P2**。
- **汇总**：优先把"记忆召回→RAG 前置"做扎实(已在④)；声纹+TTS 切换作差异化；其余云端特性不搬。

### ⑩ 测试与联调体系
- **现状(领先点)**：`tests/mock_client.py` 固件协议联调覆盖 v1/v2/v3；各模块 `#[cfg(test)]` 单元(framer/reservoir/sender/registry/memory/prefetch)；macApp 测试台(README `hello.test=true` 走 asr_test/tts_test/llm_test)。
- **Go 参考**：大量 auto_test 覆盖 session 生命周期/TTS/设备检测/MCP 重初始化。
- **可借鉴**：Go "会话生命周期端到端测试"→ Rust 用 `mock_client.py` 已覆盖协议层，应补"多轮+打断+唤醒+指令闸门"端到端；MCP 重初始化/设备维度测试→ Rust 补 MCP 降级(老固件无工具)测试。**不**照搬 Go auto_test 依赖完整多引擎/Redis/外部服务；Rust 坚持"无外部依赖单测+协议联调脚本"路线。
- **建议**：**P0** 扩充 `mock_client.py` 为场景化联调(多轮+abort+wakeup+command gate+记忆召回)，CI 必跑。**P0** 强化 registry 漂移护栏(新增能力必须有 descriptor、字段 key 在 Config 存在；已有 tests.rs，补"新增 capability 必须注册 descriptor"断言)。**P1** 引入"回放测试"：录真实对话音频/LLM 响应离线回放验证下行连续性(水库/Pacer 不溢出)，防真机断音回归。
- **风险**：端到端需稳定 mock LLM/TTS(Rust 已有 test 端点)，sherpa feature 下真实引擎不应进 CI。

---

## 3. 管理后台 UI 分模块分析

> 前提：Rust 后台是 Kuikly 跨端公共层(`commonMain`) + macOS 原生壳；Web 产物由 server 同域静态托管；**语音链路测试（麦克风/原生音频）只能跑在 macApp**，Web 端不可行。这一约束决定了对 Go 独立 Web 控制台设计「哪些该抄、哪些不该照搬」。

### ① 配置页向导化（首登引导、分步而非单张大表单）
- **现状**：`ConfigPage.kt`/`AdminPage.kt` 用 `renderForm` → `tabbedPanel` + `ConfigSections` 常量表(14 tab) + `GET /api/config/schema`，schema 驱动、跨端复用；`aiuiConfigCard` 已用 `switchRow` 做模块级启用。无首登引导、无就绪度提示。
- **Go 参考**：首次登录自动进入 5 步向导(OTA→VAD→ASR→LLM→TTS)。
- **可借鉴**：「首登引导 + 配置就绪度」心智，**不是**其 5 步向导 UI 形态（会牺牲 schema 驱动与跨端一致性，且 Rust 配置项远多于 Go 5 类）。
- **建议**：**P1** 新增 `ConfigReadinessState`(commonMain)，按 schema 校验 OTA/VAD/ASR/LLM/TTS 是否就绪，在概览页与配置 tab 顶部渲染 `readinessBanner`(未就绪模块 + 一键跳转)；首登(本地标记)弹非阻断 `Coachmark` 浮层(复用 `ToastHost` 绝对定位范式)，可跳过。
- **风险**：引导浮层绝对定位/遮挡处理(参考 `ToastHost` 已验证)；避免与 `form.conflict` 409 提示视觉冲突。

### ② 逐模块启用/禁用 + 默认配置开关
- **现状**：仅 `aiuiConfigCard` 有 `switchRow("enabled")`；VAD/ASR/LLM/TTS 在离线优先设计里恒启用单引擎，无启用开关、无默认配置概念；全局单配置，无按设备分档。
- **Go 参考**：每个模块独立启用/禁用 + 设为默认配置。
- **可借鉴**：可选模块启用开关范式(`switchRow` 已具备)扩展到真正「可选」的：AIUI(已有)、记忆(`memory`)、灵魂(`soul`)、指令闸门(`command`)、MCP。**不**搬「按设备设多套默认配置档」(Rust 单二进制单全局配置，`GET/POST /api/config` 部分更新语义，无多设备 profile 体系)。
- **建议**：**P2** 为 `command`/`memory`/`soul`/`mcp` 各卡加 `switchRow` 启用开关；`ConfigSections` 增加 `optional` 标记，渲染时可选模块标题右侧统一显示开关位。

### ③ 配置测试与首包延迟可视化（P0，Go 最强项）
- **现状(关键)**：服务端测量能力**已存在**——`server/src/app/session/bench.rs` 已实现 `asr_test`/`tts_test`/`llm_test` 并用 `Instant` 测耗时(`tts` 回 `text=耗时ms`、`llm` 回 `elapsed_ms`)，但**仅 `is_test` 会话受理**且当前只经 **macApp `XiaoZhiModule`** 触发；Web 端无麦克风/WS 音频受限，故语音链路测试整体在 macApp。但 **LLM 测试只需 prompt、TTS 测试只需文本合成(播放可省)**，Web 可测「延迟」而非「声音」。`ttsConfigCard` 已有 `form.testXfyunCreds(ctx)`(真实短合成验证，commonMain 可用)。
- **Go 参考**：每个配置项右侧「测试」按钮(状态/首包延迟/错误)；「测试全部」批量测试。
- **可借鉴**：「配置项内联测试 + 首包延迟徽标」交互范式(per-field 右侧按钮 + 行内结果)，概念完全可移植。**关键改造**：把 bench 三类测试**新增为服务端 HTTP 端点**，Web/commonMain 直接调用，**不依赖原生麦克风/WS**。
- **建议**：**P0** 服务端在 `ws/config.rs` 旁新增 `app/ws/test.rs`(或 `app/test.rs`)HTTP 端点(`POST /api/test/llm|tts|vad`)，复用 `bench.rs` 测量逻辑，返回 `{state, first_packet_ms, error}`；客户端 `ConfigFormState` 增 `lastTest*` 状态，在 `llm/tts/vad/asr` 卡加 `actionRow{secondaryButton("测试", loading)}` + 行内 `textNote`(状态+延迟+错误)；配置 tab 顶栏加「测试全部」→ `testSummaryCard`(模块×状态×延迟)。Web 上 VAD/ASR 按钮禁用 + 灰字提示「需在 macOS/iOS/Android 测试台(需麦克风)执行」。
- **风险**：需新增服务端端点(后端中等工作量)；VAD/ASR 无法纯 Web 测试属平台硬限制，需提示降级而非硬上。

### ④ 延迟监控大盘（全局可观测性页）
- **现状**：服务端**已埋点但未聚合暴露**——`observe.rs` 的 `note_assistant_turn(text, elapsed_ms)` 记整轮耗时，`bench.rs` 记各模块首包耗时；`StatsState`(来自 `/api/registry`)只暴露 `devicesOnline/sessionsLive/turns/usageTurns/uptime`，**无模块级首包延迟**；`UsageState`(`/api/usage`)只有 token 计量。即「数据在产生，但管理台看不到延迟」。
- **Go 参考**：独立「延迟监控」页，各模块首包延迟统计 + 优化建议。
- **可借鉴**：完全值得抄「全局延迟大盘」这一页，Rust 测量原语现成，主要补聚合读取端点。**不**必抄其优化建议文案表——Rust 瓶颈更多在下行水库/上游吞吐，建议应基于 Rust 实际参数(`downlink_lead_ms`/`tts.speed`/`llm` 模型选择)生成。
- **建议**：**P1** 服务端新增 `GET /api/metrics`(或扩展 `/api/registry`)，聚合每模块近 N 轮首包延迟(VAD/ASR/LLM/TTS)+整轮 `elapsed_ms` 分位(p50/p95)+错误率，内存态重启清零；客户端新增 `MetricsState` + `renderOpsLatency`(`AdminNav` 加 `latency` 或并入概览)，复用 `OpsPages.kt` 的 `metricRow`/`groupedCard`/`opsHeader` 范式，画每模块首包延迟数字 + 状态色阈值 + 一句服务端字段驱动的建议。
- **风险**：环形缓冲采样需确认采样窗口与重启清零口径，页面必须标注「内存态、重启清零」。

### ⑤ 设备管理与消息注入交互
- **现状**：`renderOpsDevices` + `DevicesState` **刻意只读**——展示设备/在线/协商参数/上次异常(`row.lastError` 显示「设备为什么突然不出声」第一现场)。无绑定/解绑/注入写操作(文件头明确本期不做)。
- **Go 参考**：设备管理含设备消息注入、OTA 验证等写操作。
- **可借鉴**：「**消息注入**」——Rust 架构**反而更优**：server 持有所有设备 WS 连接(`app/ws/registry.rs`)，Web 后台与服务端同域，可经 `POST /api/devices/{id}/inject`(server 代发文本/指令给指定设备)。这是 Go 单 Web 控制台难以做的(Go 设备走 MQTT 另外通道)。**不**搬「绑定智能体/解绑」等 Go 多租户操作。
- **建议**：**P1** 服务端在 `ws/registry.rs` 增加「代设备发消息」能力(server→device 文本/指令，协议本就支持下行文本)；客户端在 `renderOpsDevices` 设备卡加 `actionRow{primaryButton("注入消息")}` → `AlertDialog` 弹输入框 → 调端点，结果走 `ToastHost`；设备卡加「触发该设备 OTA 检查」按钮(复用 OTA 匹配逻辑)。
- **风险**：注入需活跃 WS，离线设备禁用按钮并提示「设备离线，无法注入」；避免滥用，可加确认。

### ⑥ OTA 体验强化
- **现状(已较强)**：`OtaCard.kt` + `OtaState` + `OtaFilePicker`——包内元数据自动识别、匹配规则可见、`matchNote` 写清「谁会被升级」、行内二次确认删除、上传/刷新 toast。
- **Go 参考**：OTA 是向导 Step 1，配 MQTT/UDP 地址下发 + 测试连通性。
- **判断**：Rust「匹配规则可见 + 谁会被升级」**已领先 Go**，保留。Go 的 MQTT/UDP 连通性测试**不适用于 Rust**(服务端即 OTA 源，设备主动拉，无 MQTT/UDP 下发)。
- **建议**：**P2** `OtaState` 增「同功能+型号历史版本」列表 → `otaVersionBlock`(当前生效 vs 可回滚，行内确认)；上传增加进度反馈；设备卡(area ⑤)加「通知该设备检查 OTA」按钮闭环。
- **风险**：多版本保留增加服务端存储管理，需确认固件目录策略。

### ⑦ 跨端一致性（web/Android/iOS/OHOS/macOS 对齐）
- **现状(平台拆分)**：commonMain(共享)= AdminPage/ConfigPage/OpsPages/所有 admin 组件/AdminTheme/ToastHost/OtaCard/McpToolsCard；macosArm64Main(仅 macOS)= AdminShell/ConnectState/TestBenchState+TestBenchPages(依赖 `XiaoZhiModule` 原生麦克风/音频)。即配置+只读运维全平台共享；测试台+切换服务器仅 macOS。Web 是唯一不能做语音链路测试的平台；iOS/Android/OHOS 原生都有麦克风与音频能力，可承载测试台。
- **可借鉴**：把测试台状态/逻辑(`TestBenchState` 的 connect/asr/tts/llm 流程)**抽到可跨原生平台复用**，让 iOS/Android/OHOS 也拥有语音链路测试(各自实现原生 `XiaoZhiModule` 等价物)；Web 仅展示「请使用桌面端/移动端测试台」入口 + area ③ 的 HTTP 轻量测试(LLM/TTS 延迟)。`OtaFilePicker`(原生文件对话框)确认 Android/iOS 有等价实现。
- **建议**：**P1** commonMain 测试台渲染对 Web 做 `expect`/平台分支(Web 显示「测试台不可用，原因… + 前往 macOS/iOS/Android」卡片；原生平台渲染完整 TestBench)；统一配置/运维两端空态/错误态(复用 `AdminTheme` token)。
- **风险**：原生模块移植工作量(iOS/OHOS 各需实现麦克风/音频/WS 桥)；Kuikly 各端渲染差异需实机回归。

### ⑧ 导航与信息架构（侧边栏、空态、加载态、错误态）
- **现状(已较好)**：`AdminNav` 常量表(概览/设备/会话/对话记录/用量/配置，默认落地概览)、`sidebarItem`(高 44、选中胶囊+竖条、dirty 橙点)、`largeTitleBar`、`opsHeader`/`opsError`/`textNote` 统一空/错态、`ToastHost`；五路数据并行 load 各自 loading/error。
- **差距**：缺窄屏(手机)响应式(侧边栏固定 240 挤压内容)、加载骨架屏(仅「加载中…」)、全局搜索/命令、侧边栏分组、键盘导航。
- **建议**：**P1** 响应式：`AdminPage.body` 对 `pagerData.pageViewWidth < 768` 渲染**抽屉式侧边栏**(汉堡 + 绝对定位浮层，复用 `ToastHost` 范式)；加载态新增 `ShimmerBlock`/`SkeletonCard`(复用 `cardBg`/`insetBg` token)；`AdminNav` 加 `group` 字段(运维组 vs 配置组)渲染分组标题。
- **风险**：Kuikly 响应式断点需实机验证；抽屉动画受限可用 vif 显隐替代过渡。

### ⑨ 可访问性与暗色主题统一
- **现状(已较强)**：`AdminTheme.kt` 完整暗色 token(`AdminColors`/`AdminType`/`AdminSpace`/`AdminShape`)、品牌绿 `#07C160`、暗底选中用更亮 `#4CD964` 保对比；触屏目标 ≥44(`sidebarItem`)；`focusRing` token 已定义(疑似未全量使用)。
- **差距**：仅暗色，无浅色/系统跟随；`textTertiary(#8E8E93)` 在 `#1C1C1E` 上对比约 4.0:1，小字(micro=13)临界不达标；无语义角色/无障碍标签(Kuikly 有限)；`focusRing` 未系统应用；无「减少动效」选项。
- **建议**：**P2** 新增浅色 token 集(或 `isDark` 开关)，`AdminColors` 按主题取色(低风险，token 已集中)；提升 `textTertiary` 对比至 ≥4.5:1；`focusRing` 在 `labeledField`/`dropdownField`/`primaryButton` 聚焦时应用；关键元素加 `contentDescription` 类语义。
- **风险**：浅色主题需全量审查对比度；Kuikly 各端无障碍支持不一致。

### ⑩ MCP 工具可视化
- **现状(已较强，领先)**：`McpToolsCard.kt`——按设备投影(`mcp.devices`)，每台显示 name/signature/description 原文；`userOnly` 用 warn 色徽标(「仅管理员」，即「AI 为何不用此工具」的答案)；错误行内展开 + 「显示上次成功结果」；空态解释「设备唤醒才探测」。数据来自 `GET /api/mcp/tools`。
- **Go 参考**：Go 文档未专门描述 MCP 可视化；Rust 此块已自成体系。
- **可借鉴**：主要补工具分组/分类、按能力筛选搜索、调用历史(若服务端记 tool call)、长描述折叠。
- **建议**：**P2** `mcpToolsCard` 加搜索/筛选输入(`labeledField` 复用) + 按 `userOnly`/`category` 分组折叠(长 description 默认折叠)；若服务端新增工具调用日志，在工具块加「最近调用 N 次 / 上次结果」行。
- **风险**：工具分类标准需与服务端 schema 对齐；调用历史需新增埋点。

---

## 4. 合并优先级总表（功能 + UI）

| 模块 | 维度 | 优先级 | 一句话动作 |
|------|------|--------|-----------|
| 配置向导化 + 就绪度 + 影响高亮 | 功能+UI | **P0** | 基于 `schema_json()` 做向导式分步表单 + `readinessBanner` + 改动影响高亮（不替换 schema tab） |
| 配置内联测试 + 首包延迟 | UI/后端 | **P0** | 服务端新增 `/api/test/{llm,tts,vad}` HTTP 端点，配置卡加内联测试 + 延迟徽标（VAD/ASR Web 提示转测试台） |
| VAD 实例池化 | 功能 | **P0** | 实现 VAD 实例池（先验证 sherpa 实例并发约束），session 借/还，落点 `plugins/vad/VadPool` |
| 文本/控制侧优先级变换管线 | 功能 | **P0** | 引入轻量优先级 Transformer 管线（指令闸门先于 LLM），音频侧 Pacer 不动，新增 `app/pipeline.rs` |
| 轻量事件总线解耦 | 功能 | **P0** | 定义 `ServerEvent` + broadcast 解耦统计/历史/可观测，新增 `app/events.rs` |
| 场景化联调 + registry 护栏 | 功能 | **P0** | 扩充 `mock_client.py` 为场景化（多轮+abort+wakeup+gate+记忆）；强化「新增 capability 必须注册 descriptor」断言 |
| 延迟监控大盘 | UI/后端 | P1 | 服务端聚合首包延迟端点 `GET /api/metrics` + 新增「延迟」运维页 |
| 设备消息注入 | UI/后端 | P1 | 服务端代发消息端点 + 设备卡「注入消息/触发 OTA 检查」按钮 |
| 跨端一致性 / 移动端测试台 | UI | P1 | 测试台逻辑抽跨原生平台复用（iOS/Android/OHOS 承载），Web 仅展示入口 + HTTP 轻测 |
| 导航响应式 / 骨架屏 / 分组 | UI | P1 | 窄屏抽屉侧边栏 + 加载骨架屏 + 侧边栏分组 |
| 配置预设 + 每设备 overlay | 功能 | P1 | `Config::load` 增 device overlay + preset，不引 Redis |
| 引擎 descriptor 健康自检 | 功能 | P1 | 补全非引擎能力 descriptor + `check_all(cfg)` 自检端点 |
| 记忆多后端注册表 | 功能 | P1 | `GraphMemory` 挂 `MemoryProvider` 注册表，支持本地后端即插 |
| 声纹 + 动态 TTS 切换 | 功能 | P1 | 离线声纹 + 按说话人切音色 |
| 本地 RAG 后端 | 功能 | P1 | 本地文档索引作记忆 trait 可选后端（不对接 Dify 云端） |
| MCP 设备工具清单发现 | 功能 | P1 | 握手上报可用 tools → server 缓存按需调用（替代硬编码 `audio_listener.stop`） |
| 启用开关扩展（command/memory/soul/mcp） | UI | P2 | 为可选模块加 `switchRow` 启用开关 |
| OTA 体验强化（版本/回滚/进度） | UI | P2 | 加版本对比/回滚视图、上传进度、矩阵视图（保留现有匹配规则可见） |
| 可访问性 / 浅色主题 | UI | P2 | 补浅色主题、提升 tertiary 对比、系统应用 focusRing |
| MCP 可视化增强 | UI | P2 | 加搜索/筛选/分组折叠、可选调用历史 |
| MQTT-UDP 传输 | 功能 | P2 | 暂不引入；保持 WS + v1/v2/v3 回归 |
| OpenClaw / 音乐 / 激活/OTP | 功能 | P2 | 不搬（云端编排/增值/本期不做） |
| 第三方 MCP 市场 | 功能 | P2 | 不搬（多租户运维平台，离线单二进制不需要） |

---

## 5. 建议实施路线

1. **P0 六件事（建议作为一个迭代，约 1~2 周）**：配置向导化+就绪度 → 配置内联测试 HTTP 端点+延迟徽标 → VAD 实例池 → 文本/控制侧优先级管线 → 轻量事件总线 → 场景化联调+registry 护栏。这六项均**低成本、高收益、契合离线单二进制定位**，且多数复用现有代码（`schema_json()`、`bench.rs`、`prefetch` 的 abort 语义、`mock_client.py`）。
2. **P1（第二个迭代）**：补齐可观测（延迟大盘/metrics）、设备消息注入、跨端测试台、导航响应式、记忆/插件/MCP 可插拔、声纹+本地 RAG。
3. **P2（按需/路线图）**：一律是超离线定位的云端重特性或体验打磨，默认不投入，待明确需求再评估。

> 原则：**取其架构思想，不搬其重实现**；凡新增服务端能力，优先复用现有 `registry`/`schema`/`bench`/`observe` 基础设施，避免另起炉灶。
