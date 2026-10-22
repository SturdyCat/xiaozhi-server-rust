# 架构提案评估：差距核实 + 价值审查（合并归档）

> 状态：**过期评估记录（2026-10-14），提案未作为实施清单采纳**。评估对象《xiaozhi-server-rust 架构优化建议》原文未入库（依据主理人转述条目核实）；总体结论为「可作设计思想参考，不作为实施清单」。其中部分真缺口（如 VAD 池化）此后已有对应实现，**当前状态一律以源码、测试与各模块 `AGENTS.md` 为准**；文中 `文件:行号` 为记录时点快照。
> 来源：两份同日期、同评估人的文档合并——架构师 高见远《架构提案 × 代码现状 差距核实报告》（Gap Audit v2，282 行，逐条核实提案 vs 代码现状）+ 《技术方案价值评估 + 过度设计审查》（313 行，站在本仓库真实约束上判必要 vs 过度设计）。
> 阅读顺序：先读「结论速览」；逐条差距与改动量查第一部分；价值取舍、过度设计与替代优先级查第二部分。
> 相关：[MQTT+UDP 承载评估](<mqtt-udp-transport-assessment.md>)（提案 §3 的专项展开）、[Wyoming 调研与评估](<wyoming-provider.md>)（提案 §12 的专项展开）。

---

## 结论速览（合并）

| 维度 | 结论 |
|---|---|
| 差距核实 | 13 个可核实条目：**5 已实现 / 4 部分实现 / 0 完全未实现 / 3 不需要或伪需求**；没有一条需要大重构，实际工程量约为提案暗示的十分之一 |
| 提案首要风险 | 把大量**已存在**的能力（`trait Transport`、`EventBus`、三大 Provider trait、TTFA/TTFT/TTFS 分段指标、三层背压）当成待建项 |
| 必须驳回 | Audio Focus（职责错位，播放在设备侧）／设备能力静态表（拿不到数据 + 与 MCP 双源真相）／WebRTC（固件零实现，无客户端）／trait 改 async（与仓库既有结论直接冲突） |
| 真缺口性价比序 | VAD 池化 > 取消契约收敛 > turn 级 id + metrics 维度 > 熔断 + 自动 Fallback > MQTT+UDP 承载（先技术验证） |
| 方法论教训 | BSD/zsh 版 `grep "A\|B"` 会静默吞掉交替语法且退出码为 0——所有「零命中」结论必须用 `grep -E` 或专用工具复核 |

---

# 第一部分 · 差距核实（Gap Audit v2）

> 核实人：架构师 高见远（bob / software-architect）
> 核实对象：《xiaozhi-server-rust 架构优化建议》§3–§21（提案原文未入库，本报告依据主理人转述的条目描述逐条核实）
> 核实方式：逐文件精读 + `grep -E` 计数，**所有判定均带 `文件:行号` 证据**；未读到的一律标注「未验证」。

---

## 0. 修订记录（v2，重要）

**v1 存在方法论错误，已全部修正。**

**根因**：本机`grep` 是 BSD/zsh 版本，**双引号内的 `\|` BRE 交替语法被静默吞掉**——不报错、不匹配、退出码 0（实测：`grep -n "pub struct\|pub enum" src/workflow/protocol/protocol.rs` 返回**空**，而 `grep -nE "pub struct|pub enum"` 正确返回 5 行）。v1 报告中所有形如 `grep -rn "A\|B\|C"` 的"零命中"结论**全部无效**。

**受影响并已修正的判定**：

| 条目 | v1 结论（错） | v2 结论（已核实） |
|------|--------------|------------------|
| §14 Provider Health | ❌ 未实现（"Phase/warn_if 零命中"） | ⚠️ **部分实现**——`Phase` 四态 + `warn_if` 早已实现于 `plugins/host.rs`；缺的是熔断/自动 Fallback |
| §5 VoiceSession 状态机 | ⚠️ "抽象已有，**状态机没有**" | ✅ **状态机已存在**——`device/registry/types.rs:11-20` 四态枚举 + 三处状态迁移点 |
| §11 Device Capability | 🚫 "硬件分支零命中" | 🚫 **结论不变但理由修正**——型号字面量零命中（真），但 `board`/`model` 在固件 OTA 路径大量使用 |
| §10 Audio Focus | 🚫 不需要 | 🚫 **结论不变**（`media_player/ducking/audio_focus/music` 重跑后确为 0 命中） |
| §8 敏感词过滤 | 🚫 零命中 | 🚫 **结论不变**，但发现 `session.rs:375` 已预留扩展位 |
| §6 Cancellation | 🚫 "零命中" | ⚠️ **前提部分修正**——`is_cancelled`/`stop_flag` 确为零命中，但存在**第二套独立取消语义**（见下） |
| §3 MQTT/UDP | "零命中" | ⚠️ 修正为**仅注释/文档提及**，无实现代码 |

**v1 唯一保留的"零命中"结论**：`trace_id`/`turn_id`/`opentelemetry`/`prometheus` 重跑后确为 0 命中（`usage.rs:95` 仅一处"供后续 Prometheus 类导出"的注释性提及）。

---

## 1. 结论摘要（v2）

1. **提案 13 个可核实条目中，5 条已实现（✅）、4 条部分实现（⚠️）、0 条完全未实现、3 条不需要/伪需求（🚫）**。v1 说"6 已实现 / 3 部分 / 3 未实现"是把"部分实现"误记为"未实现"。
2. **最严重的错误：提案把已有的 `trait Transport` 当作"待建抽象"提**。`workflow/protocol/transport.rs:40-65` 已定义 `trait Transport`，`workflow/session/mod.rs:108` 的 `Session<'a, T: Transport>` 已泛型化消费，模块文档 `transport.rs:1-3` 甚至已论证"未来 MQTT+UDP 实现一份 trait 即可"（正是提案 §3 的理由）。
3. **第二个错误：§14 假设健康状态机需要新建**。实际 `plugins/host.rs:33-42` 已有 `Phase { Disabled, Active, Degraded, Failed }` 四态 + `:54-56` `is_deviation()` + `:123-140` 完整决策链（`warn_if`→`Degraded`、`validate` 失败→`Failed`/`Degraded`）+ `:377-380` 聚合端点。**提案把已有基建重提为待建项。**
4. **第三个错误：§5 假设状态机需要新建**。实际 `device/registry/types.rs:11-20` 已有 `Phase { Idle, Listening, Thinking, Speaking }`，且有**三处活跃迁移点**（`session.rs:422` Listening、`device/registry/mod.rs:264` Thinking、`session/reply.rs:188` Speaking）。提案点名的"Recognizing"未单列（识别合并进Listening→Thinking 之间）。
5. **提案没讲清的关键分层（v2 新增，高价值发现）**：§14 把**「状态上报」与「故障自愈」混为一谈**。前者（`Phase` + `warn_if` + 聚合端点）已完备；后者（熔断/自动 Fallback）完全没有。`Phase::Degraded` 只**如实上报**当前实现不可用，**不会自动切换到另一个实现**——这两件事之间隔着一个完整的熔断器 + fallback 编排层。
6. **§6 的前提部分成立、结论要改**：v1 说"多套并行取消机制不存在"是错的。`is_cancelled`/`stop_flag` 确实零命中，但存在**两套语义不同、作用域不同的取消**：`session.rs:130` 的会话级 `abort: Arc<AtomicBool>`（打断/barge-in），与 `plugins/llm/openai.rs:268`+`plugins/tts/kokoro.rs:71` 的引擎内局部 `cancelled: bool`（流读取终止）。提案设想的"统一"确有价值，但**风险点不是"多套并存"，而是"两套作用域边界不清"**。
7. **MQTT+UDP 是真实缺口**（v1 结论修正）：`grep -rniE "mqtt|webrtc|udp" src/` 重跑后 7 处命中，**但全部是注释/文档**（`transport.rs:2/11/54/60`、`session.rs:107`、`mcp.rs:10`、`ws/ota.rs:20`）——**零实现代码**。且 `ws/ota.rs:20` 显示固件 OTA 已把 `mqtt` 作为传输选项之一，**说明设备侧有 MQTT 需求，服务端侧完全空白**。这是 WAKE_DASHBOARD 类重构的前置依赖。
8. **双重取消源是真 bug 风险（认可主理人判断，已在 §3.6 展开）**：`abort`（会话级意图）与引擎内 `cancelled`（读取终止）之间**没有统一的取消契约**——LLM 流被 `abort` 打断时，引擎内 `cancelled` 只终止 HTTP 流读取，**不保证已送出的 chunk 被丢弃**。`retry.rs` 的"已交付文本就不重试"是唯一防线。
9. **§21 八条原则：仓库当前 8/9 满足**（详见 §6），Transport≠Session 已由泛型落实，Device≠Hardware Model 因**型号字面量零命中**而"更彻底"地满足。
10. **改动量总体极小**：13 条里 9 条为 0～2 文件，2 条"改 3–8 处"，**没有一条需要大重构**。实际工程量约为提案暗示的**十分之一**。

---

## 2. 逐条判定表（核心交付物，v2）

| # | 提案条目 | 判定 | 证据（文件:行号） | 实际差距 |
|---|---------|------|------------------|---------|
| 1 | **§3 Transport 抽象**（`trait Transport { send_audio / send_event / close }`，实现 WS/MQTT+UDP/WebRTC/Local） | ✅ **已实现（抽象层）/ ❌ 仅 WS 一种承载** | `workflow/protocol/transport.rs:40-65` trait 定义；`:30-37` `IncomingFrame`；`:79-155` `WsTransport`；`:62-64` `close()` 带设计说明；`workflow/session/mod.rs:108-109` `Session<'a, T: Transport>`；`workflow/session/start.rs:30` `run_session<T: Transport>`<br>`grep -rniE "mqtt\|webrtc\|udp" src/` = **7 处命中，全是注释**：`transport.rs:2/11/54/60`、`session.rs:107`、`mcp.rs:10`、`ws/ota.rs:20` | **抽象层零差距**（方法名不同但语义等价）。<br>⚠️ **v1 修正**：v1 说"MQTT/UDP 零命中"，实为**注释命中、代码零命中**。`ws/ota.rs:20` 表明固件 OTA 已把 `mqtt` 列为传输选项→ **设备侧有需求、服务端侧完全空白**。这是真实的第二承载缺口（详见 §5 G5）。 |
| 2 | **§4 Provider 统一为 async trait** | ⚠️ **部分实现** | 已 async：`plugins/llm/mod.rs:163-174`、`plugins/memory/mod.rs:61`（`BoxFut`）+ `:69-95`。<br>仍同步：`plugins/asr/mod.rs:29-31`、`plugins/tts/mod.rs:315-326`、`plugins/vad/mod.rs:26-31`（流式喂帧，设计上就该同步） | 调用点实测：`.recognize(`（排除 tests）= **1 处**（`workflow/session/mod.rs:318`）；`synthesize_stream(` 真实调用 3 处（`benchmark/mod.rs:227`、`workflow/pipeline/prefetch.rs:372`、`workflow/session/bench.rs:219`）。<br>→ 改 `asr/mod.rs` 签名 + 1 个调用点加 `.await`。**VAD 不建议改**（改 async 引入缓冲语义变化=倒退）。 |
| 3 | **§5 VoiceSession 抽象 + 状态机（Idle→Listening→Recognizing→Thinking→Speaking）** | ✅ **已实现（状态机已存在，v1 判断错误）** | 抽象：`workflow/session/mod.rs:101-165` `Session<'a, T: Transport>` 聚合 transport/engines/params/session_id/device_id/eff_config/history/vad/uplink_decoder/abort/aiui/pacer/commands/mcp_*/wake/inject_rx（`:103-107` 文档："原本要传 7~11 个离散参数…统一用 `&mut self`"）。<br>**状态机（v2 新增证据）**：`device/registry/types.rs:11-20` `pub enum Phase { Idle, Listening, Thinking, Speaking }`；`:24-31` `as_str()` JSON 契约；`:34-41` `display()` 中文文案。<br>**三处活跃迁移点**：`workflow/session/mod.rs:422` `note_phase(Phase::Listening)`、`device/registry/mod.rs:264` `s.phase = Phase::Thinking`、`workflow/session/reply.rs:188` `note_phase(Phase::Speaking)`；归位 `device/registry/mod.rs:174/304` | **v1 说"状态机没有"是错的**（因 grep 失效）。实际四态枚举 + 迁移点全部在用，界面上"这台设备在干什么"已可答。<br>⚠️ 唯一差距：提案的 **`Recognizing` 未单列**（识别阶段被合并在 `Listening→Thinking` 之间）；且`Phase` 定义在 `device/registry/types.rs`（数据模型模块）而非 `workflow/session/`，归属略偏。 |
| 4 | **§6 统一 Cancellation（`CancellationToken`）** | ⚠️ **前提部分成立，结论修正** | **第一套（会话级）**：`workflow/session/mod.rs:130` `abort: Arc<AtomicBool>`（`:128-129` 注释"打断标志（barge-in）"），置位 `:440`，复位 `:144`（reply 入口）/`:67`（command 刻意）。clone 传递链 6 处消费：`session/producer.rs:46,66,87`→`workflow/pipeline/splitter.rs:114,124,146`→`workflow/pipeline/prefetch.rs:233,310`→`workflow/downlink/reservoir.rs:355,376,401`→`workflow/downlink/sender.rs:55,153`→`workflow/pipeline/inference.rs:119,186`。<br>**第二套（引擎内局部，v2 新增）**：`plugins/llm/openai.rs:268,286,292,304,307` `let mut cancelled = false`；`plugins/tts/kokoro.rs:71,96,105,107,109` `cancelled: bool`。<br>`grep -rnE "is_cancelled\|stop_flag" src/` = **0 命中**（该结论有效）。<br>`Cargo.toml` 无 `tokio-util`。 | **v1 说"不存在多套并行机制"不准确**。实为**两套作用域不同的取消**：① 会话级意图标志；② 引擎内流读取终止标志。二者**无统一契约**（详见 §3.6 bug 风险）。<br>但 `is_cancelled`/`stop_flag` 确为零命中，且 `inference.rs:20-27` 记载的**不对称语义**（ASR 侧刻意不查 abort 以免stale abort 吞掉 barge-in 首句）`CancellationToken` 的广播语义**无法表达**。<br>→ **部分成立**：值得统一的是"取消契约"，**不是**简单换类型。建议只做 G3（见 §5）。 |
| 5 | **§7 Typed Event Bus**（18 变体） | ⚠️ **机制完备，变体数少于提案** | `observability/events.rs:158-206` `ServerEvent` **6 变体**：`SessionStart`(`:160`)/`SessionEnd`(`:168`)/`MessageAdded`(`:175`)/`TtsRound`(`:183`)/`Error`(`:192`)/`TurnLatency`(`:200`)。<br>`:212-232` `EventBus` 基于 `tokio::sync::broadcast`（容量 1024，`:19` 论述取舍），`subscribe()`(`:224`)/`emit()`(`:229` 忽略所有失败)。<br>emit 点7 处：`session/start.rs:91`、`session.rs:242,392`、`session/reply.rs:216,228,240,250,267,282`。 | 传输机制（broadcast 多订阅者扇出）**完备**。提案 18 变体是 HA 场景完备性列举，本仓库只需当前 6 个。<br>⚠️ **实际只有 1 个消费者**（详见 §3.9）：全仓 `subscribe()` 唯一调用点为 `main.rs:69`→`observability/metrics::subscriber`。**基础设施已在，缺的是多消费者用例**，不是缺基础设施。 |
| 6 | **§8 Pipeline Middleware** | 🚫 **大部分不需要；真实缺口是文本侧管线化** | 已有：句子切分 `workflow/pipeline/splitter.rs:28-96` + `:111-165` watchdog；Resample `media/resample.rs:28-39`；Opus `media/opus.rs`；VAD `plugins/vad/mod.rs:26-31` + `plugins/vad/pool.rs`（**已池化**）；AEC 能力位`workflow/protocol/protocol.rs:72-77` `ClientFeatures{mcp,aec}`；Stop 信号范式 `workflow/session/mod.rs:37-45` `enum UtteranceDecision{Respond,CommandStop}`。<br>`workflow/pipeline.rs` **不存在**（提案自己建议新增）。<br>`grep -rniE "media_player\|ducking\|audio_focus\|music"` = 0 命中（**重跑确认**）。<br>`grep -rniE "敏感词\|sensitive\|moderat\|profanity"` = **1 处**：`workflow/session/mod.rs:375` 注释"Stage 顺序与语义（新增文本侧变换——记忆注入/**敏感词**/指令扩展——在此序列中按需…）"。 | 音频变换与句子切分**已全覆盖**，不需要 middleware 链这层间接。<br>敏感词/语言检测/Markdown 过滤**业务上不需要**（自用离线助手，不过审）。注意 `session.rs:375` 已**预留扩展位**——若将来真要做，扩展点已指定。<br>→ **真实缺口只有一个**：文本侧变换尚未管线化（command/wakeup 仍是早退 return，见 §6 原则 #6）。 |
| 7 | **§9 Streaming First + TTFA/TTFT/TTFS** | ✅ **已实现（五阶段分位齐备）** | `observability/events.rs:44-58` `TurnStages`：`asr_end_ms`(`:49`)/`llm_first_token_ms`(`:51`)/`sentence_ready_ms`(`:53`)/`tts_first_pcm_ms`(`:55`)/`first_audio_ms`(`:57`)。<br>`:78-143` `TurnTrace`（first-only mark，`Instant` 单调钟）。<br>`observability/metrics.rs:39-46` 五窗口；`:49-70` `stage_stats_json`(p50/p90/p95/p99/max)；`:132-157` 聚合（`:137` 按anchor 过滤）；`:199-206` `latency` 节。<br>路由 `gateway/ws/mod.rs:190`；投影 `gateway/ws/metrics.rs:21-30`。首帧真实时刻由 `downlink/sender.rs` 的 `first_frame_at` out-param 提供（`events.rs:124-137` 详述为何不能用"现在"）。 | **零代码差距**。TTFA（`first_audio_ms`）、TTFT（`llm_first_token_ms`）、TTFS（`tts_first_pcm_ms`）全部可测。 |
| 8 | **§10 Audio Session / Audio Focus** | 🚫 **不需要（伪需求，从 HA 照搬）** | `grep -rniE "media_player\|ducking\|audio_focus\|music\|duck" src/`（排除 tests）= **0 命中**（重跑确认）。<br>`plugins/` 11 个子模块无 media/player 类能力；`workflow/mcp/mod.rs` 是**设备侧**MCP 原语（服务器是 Client，见 `session.rs:13-20`），工具由固件决定。<br>服务端音频出口唯一：`workflow/downlink/mod.rs:1-2` "合成产物 → 水库 → 按实时节拍下发"。 | 服务端**不存在并发音频源**，无冲突可仲裁——`Alarm > TTS > Music` 三级优先级在只有一路 TTS 的服务端**无处施加**。音乐播放由固件本地负责，服务端连通道都拿不到。 |
| 9 | **§11 Device Capability 表** | 🚫 **型号分支零命中（结论不变，理由修正）** | **型号字面量零命中**：`grep -rniE "esp32_s3\|esp32_p4\|chip" src/` = 0（`protocol.rs:1,4` 的 "esp32" 是仓库名/文档路径，非型号分支）。<br>**但 board/model 大量使用（v1 遗漏）**：`device/firmware/http.rs:9,41,65,76,94,111,114,155-163,203-243` 全套固件 OTA 的 `project/model/board/version` 四元组。<br>**能力信息在 MCP 通道**：`workflow/mcp/registry.rs:45-59` `ProbeResult{device_id,board,firmware_version,protocol_version,tools,error}` + `:63-74` `DeviceTools`。<br>**真实缺口**：`workflow/protocol/protocol.rs:80-90` `ClientHello{version,features,transport}` 整个结构体标`#[allow(dead_code)]`，注释 `:80` 自承"当前流水线未消费"。 | 提案要替代的型号分支**不存在**（此结论有效）。<br>⚠️ **v1 修正**：board/model 在 OTA 路径是**大量使用的真实概念**，不是零命中——只是**不用于运行时行为分支**（仅选固件包）。<br>→ 真实缺口 = `ClientHello.transport`/`features.aec` 是死字段。**不要新建 capability 表**，扩展已有 `mcp/registry.rs:63-74` 即可。 |
| 10 | **§12 Wyoming Provider** | 🚫 **已评估完毕，不重复展开** | 见 [wyoming-provider.md](<wyoming-provider.md>)（合并归档：PM 调研 637 行 + 架构师评估 597 行）。 | 按主理人指示不重复。结论一句话见 §3.7。 |
| 11 | **§13 Observability**（四级 id + otel/prometheus） | ⚠️ **两级 id 已有，四级 id 缺**（结论不变，证据重跑） | 已有：`session_id`/`device_id` 在 `events.rs:160-205` 每个变体成对携带；`TurnTrace`(`events.rs:78-79`) 承担 turn 级锚点。另有 `device/registry/types.rs:11-20` `Phase` 提供**会话阶段维度**。<br>`grep -rniE "trace_id\|turn_id\|opentelemetry\|prometheus"` = **1 处**：`observability/usage.rs:95` 注释"供测试与**后续 Prometheus 类导出**用"（仅提及，未实现）。<br>`Cargo.toml` 有 `tracing`/`tracing-subscriber`，**无** otel/prometheus。<br>另 `workflow/session/observe.rs:27-48` 四个 `note_*` 埋点写有状态观测面（`events.rs:5-8` 明说与事件总线"并存，不是替换"）。 | `trace_id` 透传与 span 层级**确实没有**。单机单进程下价值有限。<br>⚠️ 若做跨设备追踪**须先解`unknown` 设备键碰撞**（`device/registry/mod.rs:38`、`mcp/registry.rs:13-14` 均自承"多台会挤进同一条记录"）。<br>**建议**：先做 turn 级 id + metrics 维度，暂不引 otel。 |
| 12 | **§14 Provider Health / Failover** | ⚠️ **部分实现（v1 判定错误，已拆分重判）** | **健康状态机 =已实现**：`plugins/host.rs:33-42` `pub enum Phase { Disabled, Active, Degraded, Failed }`；`:47-53` `as_str()`；`:54-56` `is_deviation()`；`:123-140` 完整决策链（`:128-129` `warn_if`→`Degraded` + 原因 / `validate` 失败 + `fatal_if_enabled`→`Failed`，否则→`Degraded`）；`:180` 模块注释"记原因 → `Phase::Degraded`，**服务照常启动**（记忆坏了不能让语音不可用）"。<br>字段定义：`plugins/registry/mod.rs:395-427` `PluginDescriptor.warn_if: Option<fn(&Config) -> Option<String>>`（文档明写"软性问题（能构建、但能力受限）…与 `validate` 的分工"）。<br>聚合端点：`gateway/ws/health.rs:54-61` phase 严格程度序（Active<Degraded<Failed，Disabled 最低）；`:84-90` probe 合并（必需能力 Fail→`Failed`，可选→`Degraded`）；`:128-131` 四计数 `active/degraded/failed/disabled`。<br>**超时不完整**：LLM `plugins/llm/openai.rs:104-105` `connect_timeout`+`timeout`；TTS-AIUI `plugins/tts/aiui.rs:38,40` `TOTAL_TIMEOUT=30s`/`READ_TIMEOUT=15s`。**Kokoro 本地合成无超时**（CPU 推理，难设）。<br>**重试=已有（仅 LLM）**：`plugins/llm/retry.rs:29-35` `RetryingLlm` 装饰器；`plugins/llm/failure.rs:21-25` `MAX_RETRIES=2`/`RETRY_BUDGET=8s`；`:41-56` `LlmFailure` 六分类 + `:60-67` `retryable()`（4xx 除 429 不重试）。<br>**熔断= 零命中**：`grep -rniE "circuit\|breaker\|half.?open" src/` = 0。<br>**自动 Fallback = 零命中**：`Phase::Degraded` 只上报不切换（`host.rs:180` 明说"服务照常启动"）。 | **v1 判"未实现"是错的**（grep 失效导致）。拆成5 个子项重判，见 §3.1。<br>核心结论：**「状态上报」已完备，「故障自愈」完全空白**——`Degraded` 如实上报但不自动切换实现。 |
| 13 | **§15 Backpressure** | ✅ **已实现（三层齐备）** | ① 推理闸门 `workflow/pipeline/inference.rs:1`（模块定位）、`:40` `Semaphore`、`:119` `acquire_abortable`、`:59-66` `Gate`（可热换+排队计数）；配置 `plugins/asr/mod.rs:46-53` `max_concurrent` + `:62-64` clamp≥1。<br>② 水库 `workflow/downlink/reservoir.rs:209` `mpsc::channel::<DownlinkItem>(cap_items)`（**有界**）、`:355/376/401` abort 感知。<br>③ 节拍器 `workflow/downlink/mod.rs:35` `DownlinkPacer`（`:11-24` 长注释说明 ESP 20 包硬约束）。<br>④ 预取预算 `workflow/pipeline/prefetch.rs:141` `reserve()`、`:321`/`:356` 有界 channel。<br>⑤ 其他有界通道 `session/start.rs:114`、`session/aiui.rs:108`、`session/bench.rs:210`。 | **提案要的三层全部存在**，且比提案更细（abort 感知排队、水位观测、可热换许可数）。<br>无界通道仅用于流内句子队列（`session/producer.rs:50`、`session/reply.rs:341`），是**刻意**的（句子流以 LLM 产出速率自然背压）。 |

---

## 3. 深入核实：新增与修正（v2 重点）

### 3.1 §14 拆解重判：状态上报 vs 故障自愈（团队指正的重点）

提案把五样东西捆在"Provider Health/Failover"一个条目里，导致我v1 误判为整体未实现。逐项重判：

| 子项 | 判定 | 证据 | 说明 |
|------|------|------|------|
| **14a 健康状态机**（Phase 四态） | ✅ **已实现** | `plugins/host.rs:33-42`；`:47-53` `as_str()`；`:54-56` `is_deviation()` | 四态齐全（`Disabled`/`Active`/`Degraded`/`Failed`），对齐 DSH `FiberState` |
| **14b 降级告警**（`warn_if`） | ✅ **已实现** | `plugins/host.rs:128-129`；字段 `plugins/registry/mod.rs:395-427` | 软性问题→`Degraded`+原因；`validate`（硬条件）失败→`Failed`/`Degraded`，分工清晰 |
| **14c状态聚合/上报** | ✅ **已实现** | `gateway/ws/health.rs:128-131` 四计数；`:54-61` 严重度序；`:84-90` 与 probe 结果合并 | 端点已可用；`host.rs:180` 明确"服务照常启动"哲学 |
| **14d Timeout** | ⚠️ **部分实现** | LLM `openai.rs:104-105`；MCP `workflow/mcp/mod.rs:71` `DEFAULT_CALL_TIMEOUT=2s`；TTS-AIUI `aiui.rs:38,40` `30s/15s`；闸门轮询 `inference.rs:45` `ABORT_POLL_MS=200` | 在线provider 有超时；**Kokoro 本地合成无超时**（CPU 推理，设超时意义有限）。缺口在于**超时值不可配**（全为const） |
| **14e Retry** | ⚠️ **部分实现（仅 LLM）** | `plugins/llm/retry.rs:29-35`；`failure.rs:21-25`（2 次/8 秒）；`:60-67` `retryable()` 六分类 | 语音场景纪律完备（已吐文本不重试）。**TTS/ASR 无重试**（合理：ASR 无云端备胎，TTS 重试会重复发音） |
| **14f Circuit Breaker** | ❌ **未实现** | `grep -rniE "circuit\|breaker\|half.?open" src/` = **0 命中**（重跑确认） | 无熔断状态机（Closed/Open/HalfOpen） |
| **14g 自动 Fallback** | ❌ **未实现** | `host.rs:180`"服务照常启动"；`Phase::Degraded` 仅上报 | `Degraded` **不触发实现切换**。这是与 14a-14c 的**本质区别** |

**核心洞察（提案没讲清的地方）**：
- **`Phase` 是"观测面"，不是"控制面"**。它回答"现在哪个实现不健康"，但不回答"健康的话该切到谁"。
- 自动 Fallback 需要三层，**当前一层都没有**：① 探测（14c 部分有）→ ② **决策**（该切到哪个实现？按什么优先级？`PluginDescriptor` 无`fallback_to` 元数据）→ ③ **切换 + 状态机维护**（熔断器，防抖动）。
- ⚠️ 且**并非所有能力都能 Fallback**：ASR/VAD 是纯本地 sherpa，**无云端备胎**。只有 LLM/TTS 两类值得做。

### 3.2 §14 方法论错误的根因（必须记录，避免复发）

```
$ grep -n "pub struct\|pub enum" src/workflow/protocol/protocol.rs     # → 空（错！）
$ grep -nE "pub struct|pub enum" src/workflow/protocol/protocol.rs    # → 5 行（对）
```
本机 `grep` 为 BSD/zsh 版，**双引号内 `\|` BRE 交替被静默吞掉**：不报错、不匹配、**退出码仍为 0**。v1 报告中所有 `"A\|B\|C"` 形式的"零命中"结论均因此失效。**v2 起所有 grep 均改用 `-E`。**

### 3.3 §5 状态机：v1 判断错误的修正

v1 称"状态机不存在"，实为grep 失效。实际：

```
device/registry/types.rs:11-20   pub enum Phase { Idle, Listening, Thinking, Speaking }
workflow/session/mod.rs:422            note_phase(Phase::Listening)      // 设备上报 listen start
device/registry/mod.rs:264       s.phase = Phase::Thinking          // ASR 完成
workflow/session/reply.rs:188      note_phase(Phase::Speaking)        // 开始下行音频
device/registry/mod.rs:174/304   Phase::Idle                        // 登记 / 归位
```
**四态 + 四处迁移点全部在用**，界面上"这台设备在干什么"已可答（`types.rs:34-41` 附中文文案）。提案 §5 的状态机**已基本实现**，唯一差距是提案的 `Recognizing` 未单列（识别合并在 `Listening→Thinking` 之间），且枚举定义在 `device/registry/types.rs`（数据模型模块）而非 `workflow/session/`，归属略偏。

### 3.4 §3 MQTT+UDP：真实缺口的精确定位

`grep -rniE "mqtt|webrtc|udp"` = 7 处，**全是注释**：

| 位置 | 性质 |
|------|------|
| `workflow/protocol/transport.rs:2` | 模块文档："未来 MQTT+UDP / 本机测试通道等具体承载" |
| `workflow/protocol/transport.rs:11` | 设计取舍："MQTT 走自身心跳" |
| `workflow/protocol/transport.rs:54` | trait 注释："自带心跳机制的承载（如 MQTT）留空即可" |
| `workflow/protocol/transport.rs:60` | `close()` 注释："其他承载（MQTT/本机通道）按各自协议关闭" |
| `workflow/session/mod.rs:107` | `Session` 文档："MQTT+UDP / 本机通道各实现一份 Transport 即可" |
| `workflow/mcp/mod.rs:10` | "JSON-RPC 套在既有那条 WS/MQTT 上（固件 `Protocol::SendMcpMessage`）" |
| **`gateway/ws/ota.rs:20`** | **实质性**："`mqtt`（官方返回 → 固件优先选 MQTT）；`activation`（官方激活流程，自建端不需要）" |

**关键发现**：前6 处是"为未来预留"的文档化设计，**抽象层已就绪**；第 7 处 `ws/ota.rs:20` 显示**固件 OTA 协议已经把 `mqtt` 作为传输选项之一**，且已实现"官方返回 → 固件优先选 MQTT"的择路逻辑。

→ **设备侧有 MQTT 需求，服务端侧完全空白**。抽象层（trait Transport）已就位，接入成本 = 写一份 `impl Transport`。这是 WAKE_DASHBOARD 类重构的前置依赖。专项评估见 [mqtt-udp-transport-assessment.md](<mqtt-udp-transport-assessment.md>)（结论：现在不做）。

### 3.5 §9 事件总线：基础设施完备，但只有一个消费者

**`ServerEvent` 变体数与位置**：`observability/events.rs:158-206`，**6 个变体**（`SessionStart`/`SessionEnd`/`MessageAdded`/`TtsRound`/`Error`/`TurnLatency`）。

**是否支持多订阅者**：**是**，机制完备——
- `observability/events.rs:213` `tx: broadcast::Sender<ServerEvent>`（`tokio::sync::broadcast`，容量 1024）
- `:224` `pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent>`（每次调用产出独立 receiver，**语义确为多订阅者扇出**）
- `:229-231` `emit()` 忽略一切发送失败（无订阅者 `SendError` / 慢消费者 `Lagged` 都不影响发布者）
- `:19` 容量取舍文档："订阅者处理不过来会丢最旧事件（LAGGED），但发布者永不阻塞"

**实际有几个消费者**：**只有 1 个**。全仓 `subscribe()` 的唯一调用点是 `src/main.rs:69`：
```rust
tokio::spawn(crate::observability::metrics::subscriber(
    engines.metrics.clone(),
    engines.event_bus.subscribe(),   // ← 唯一订阅点
));
```
`metrics::subscriber`（`observability/metrics.rs:212-222`）消费并聚合到 `Metrics`，经 `gateway/ws/metrics.rs:21` 暴露 `GET /api/metrics`（路由 `gateway/ws/mod.rs:190`）。

**关于 `workflow/pipeline/inject.rs`**：主理人问它是否是另一个通道——**不是事件总线的消费者**。它是**设备消息注入通道**，用 `mpsc`（`workflow/pipeline/inject.rs` + `session/start.rs:114` `mpsc::channel(CHANNEL_CAPACITY)`），与会话主循环 `select!` 直接消费（`session.rs:164` `inject_rx: Option<mpsc::Receiver<String>>`）。它**不经过 `EventBus`**，注入轮次通过 `events.rs:33` 的 `ANCHOR_INJECT` 锚点进入延迟统计，属**延迟口径的衔接**而非通道共享。

**结论表述建议（供报告引用）**：
> **broadcast 基础设施已在且完备（多订阅者扇出、失败不崩、1024 容量有文档论证），当前只有 `metrics` 一个消费者。缺的不是基础设施，而是多消费者用例**——若要加审计/告警/大盘订阅者，`subscribe()` 直接可用，**无需改动事件总线**。

### 3.6 §6 双重取消源：真bug 风险（团队认可的发现，展开）

v1 说"多套并行机制不存在"不准确。实际是**两套作用域不同的取消**，且**缺统一契约**：

| | 第一套：会话级 `abort` | 第二套：引擎内 `cancelled` |
|---|---|---|
| 位置 | `workflow/session/mod.rs:130` `Arc<AtomicBool>` | `plugins/llm/openai.rs:268`；`plugins/tts/kokoro.rs:71`（局部 `bool`，非 `Arc`） |
| 语义 | **用户意图**（barge-in 打断） | **流读取已终止**（HTTP 流终止标志） |
| 作用域 | 跨 producer/prefetch/reservoir/sender/inference 6 处 | 单个引擎函数内 |
| 传播 | `clone()` 逐层传递 | 函数内本地变量，不可跨层 |

**风险点**：LLM 流被 `abort` 打断时，引擎内 `cancelled` 只终止**HTTP 流读取**，**不保证已送出的 chunk 被丢弃**。`openai.rs:304,307` 用`if !cancelled && out.text.trim().is_empty()` 做判断，语义是"没被取消且空回复才算失败"——但**已通过 `on_event` 交付给上层的 chunk 属于副作用**，取消后仍可能已播出一部分。

**唯一防线**：`plugins/llm/retry.rs` 的"已交付文本就不重试"纪律（`retry.rs:9-11` 注释明说"调用方收到 `LlmEvent::Text` 时很可能已经在下行音频了，重试会让用户听两遍前半句"）。

**因此 §6 的正确结论**：提案想统一的方向**有价值**，但真问题不是"多套并存"（那是 v1 的误判），而是**"取消意图"与"取消执行"之间没有契约**——`CancellationToken` 能统一"意图广播"，但**不能自动解决"已交付副作用的回滚"**（那需要幂等性设计，超出提案范围）。

### 3.7 §12 Wyoming：一行结论

见 [wyoming-provider.md](<wyoming-provider.md>)：**不必引入独立 Wyoming 协议层**——现有五大 trait（`AsrEngine`/`TtsEngine`/`LlmProvider`/`VadEngine`/`FullChainEngine`）+ `plugins/registry` 的 `Built` 闭枚举已是本地/远端共用的最小公共面，新增 Wyoming 实现 = 加1 条描述符。上一轮已闭环，**不重开为新任务**。

### 3.8 §11 board/model 的真实使用（v1 遗漏修正）

v1 称"`board` 零命中"，实为**grep 失效导致的误报**。实际 `device/firmware/http.rs` 大量使用：

- `:65` `resolve_target(&query, parsed.as_ref())` → `:76` `entry_dir(&firmware_dir(), &project, &model, &board)`
- `:155-163` `board` 查询参数校验（"只允许字母数字与.-_，≤64 字符"）
- `:203-218` 删除路径；`:223-243` 下载路径 `GET /api/ota/firmware/latest?project=&model=&board=`
- `:41` 注释："型号/功能/版本**优先从包内元数据解析**"

**修正后的准确表述**：`board`/`model` 是**真实存在且大量使用**的概念，但**仅用于固件 OTA 包选择，不用于运行时行为分支**。这恰好**支持**提案 §11 的核心主张（"Device ≠ Hardware Model"）——仓库已经做对了：设备身份用 `Device-Id` 握手头（`session.rs:113-115`），能力靠 MCP `tools/list` 运行时发现（`session.rs:18-20`），**不以型号推断行为**。

### 3.9 未因grep 失效而改变的其他结论

以下结论经`-E` 重跑后**保持不变**，可信度高：
- `trace_id`/`turn_id`/`opentelemetry`/`prometheus` → 0 命中（仅 `usage.rs:95` 注释提及）✅
- `media_player`/`ducking`/`audio_focus`/`music`/`duck` → 0 命中 ✅
- `circuit`/`breaker`/`half.?open` → 0 命中 ✅
- `is_cancelled`/`stop_flag` → 0 命中 ✅
- `esp32_s3`/`esp32_p4`/`chip` → 0 命中 ✅（`esp32` 命中是仓库名 `xiaozhi-esp32` 与文档路径）

---

## 4. 改动量估算（v2，仅针对「真缺口」）

| 缺口 | 归属 | 改动量 | 内容 |
|------|------|--------|------|
| **G1 熔断 + 自动 Fallback** | §14f/g | **新增 1 文件 + 改 5–8 处**（中） | ① 新增 `plugins/breaker.rs`（Closed/Open/HalfOpen 三态 + `probe()`，参考 `health.rs:11-13` 的"探测不引网络"纪律）；② `plugins/registry/mod.rs:395-427` `PluginDescriptor` 加 `fallback_to: Option<&'static str>` 元数据；③ `plugins/llm/retry.rs:29-35` 接入熔断（需重新划清`MAX_RETRIES=2`/`BUDGET=8s` 与熔断的职责边界）；④ `plugins/tts/mod.rs` xfyun↔kokoro 切换。<br>⚠️ **仅 LLM + TTS 可做**，ASR/VAD 纯本地无备胎。**建议 P2，先补 LLM 一条路径验证设计。** |
| **G2 设备能力字段消费** | §11 | **新增 1 文件 + 改 2–3 处**（小–中） | ① 消费 `workflow/protocol/protocol.rs:90` `ClientHello.transport`（去`#[allow(dead_code)]`）；② 决定 `ClientFeatures.aec`(`:76`) 用途；③ **扩展已有 `workflow/mcp/registry.rs:63-74` `DeviceTools`**，**不新建 capability 表**（避免 §16 的三处 registry 混淆）。 |
| **G3 取消契约收敛** | §6 | **改 3–6 处**（中，**有回归风险**） | ① 在 `workflow/session/mod.rs:130` 附近补一份**取消契约文档**（明确 `abort` = 用户意图、`cancelled` = 执行终止、各层谁负责丢弃副作用）；② `plugins/llm/retry.rs` 显式记录"已交付 chunk"边界；③ **不建议**换 `CancellationToken`——`inference.rs:20-27` 的ASR 非对称语义无法用广播式 token 表达。<br>⚠️ 若强行统一，**必须先补 barge-in 回归测试**。 |
| **G4 turn 级 id + metrics 维度** | §13 | **新增 1 文件 + 改 3–5 处**（中） | ① `TurnTrace`(`events.rs:78`) 加 `turn_id`；② `observability/metrics.rs:104` `observe` 加 turn 维度；③ `/api/metrics`(`:199`) 加该维度。<br>⚠️ 引 otel 前须先解 `device/registry/mod.rs:38` 的 `unknown` 设备键碰撞。<br>**建议**：先做单机版，暂不引 otel/prometheus。 |
| **G5 MQTT+UDP 承载**（v2 新增，从 §3拆出） | §3 | **大重构前置**（不单列实施） | 抽象层已就绪（`trait Transport`），接入= 写一份 `impl Transport` +协议编解码。**前置依赖**：需先确认设备侧 MQTT 固件的实际接入形态（`ws/ota.rs:20` 显示 OTA 已有 MQTT 择路逻辑，但**实时音频通道是否走 MQTT 尚未验证**）。<br>**建议**：先做技术验证（1–2 天），不直接排期。专项结论见 [mqtt-udp-transport-assessment.md](<mqtt-udp-transport-assessment.md>)：现在不做。 |

**总计：5 个真缺口，0 个需要「大重构」。** 其中 G1/G4/G5 为中等改动，G2/G3 为小改动但 G3 有回归风险。

---

## 5. §21 八条原则逐条核对（v2）

| # | 原则 | 满足 | 证据 |
|---|------|------|------|
| 1 | **Transport ≠ Session** | ✅ | `workflow/session/mod.rs:108-109` `Session<'a, T: Transport> { transport: &'a mut T, ... }`；`session/start.rs:30` `run_session<T: Transport>` 泛型静态分发；`transport.rs:1-3` 文档 |
| 2 | **Session ≠ Connection** | ✅ | `workflow/session/mod.rs:1-8`+`:141-160`：指令闸门命中后**用 MCP 让设备结束聆听、音频通道刻意保留**（"设备回到 idle 而会话还活着；下次唤醒复用连接不重连"）；`close_requested: bool`(`:140`) 仅在不支持 MCP 或调用失败时降级断开 |
| 3 | **Session ≠ Provider** | ✅ | `workflow/session/mod.rs:110-111` 只持 `engines: Arc<Engines>` + `eff_config: Arc<Config>`，不new 任何 provider；构建全在 `plugins/registry/impls.rs` |
| 4 | **Provider ≠ Implementation** | ✅ | `plugins/registry/mod.rs:19-23`：**`Built` 是闭枚举**（非 `Box<dyn Any>`），"枚举 + 按能力 `match` 保持 100% 编译期检查"；`:15-18` `REGISTRY`→`/api/plugins`、`FieldSchema`→`/api/config/schema`、"新增实现 = 表里加 1 条" |
| 5 | **Device ≠ Hardware Model** | ✅ **更彻底** | 型号字面量零命中（`esp32_s3`/`esp32_p4`/`chip` = 0）；`board`/`model` 仅用于 OTA 包选择（`device/firmware/http.rs:65-243`）**不影响运行时行为**；设备身份用 `Device-Id`（`session.rs:113-115`），能力靠 MCP `tools/list` 运行时发现（`session.rs:18-20`）。**仓库已按此原则设计** |
| **6** | **Pipeline ≠ Provider** | ⚠️ **部分满足** | Provider 侧已解耦（#3/#4）；但文本侧**未管线化**：无 `workflow/pipeline.rs`，控制流靠 `workflow/session/mod.rs:37-45` `enum UtteranceDecision`（**仅两态**），command/wakeup 仍是早退return。**但扩展位已预留**：`workflow/session/mod.rs:375` 注释"Stage 顺序与语义（新增文本侧变换——记忆注入/敏感词/指令扩展——在此序列中按需…）"。<br>→ **唯一未落地原则**，且是 §8 的真实缺口 |
| **7** | **Event ≠ Direct Dependency** | ✅ | `observability/events.rs:5-8` 明说与 `observe.rs` `note_*` "**并存，不是替换**"；`:14` "观测面绝不允许把语音链路带崩"；`:150-151` "所有字段均为只读快照…**不能据此改变流水线行为**（决策仍在会话内部）" |
| **8** | **LLM ≠ Tool** | ✅ | `workflow/session/mod.rs:13-20` "## 与设备的关系（MCP 方向，勿反）——**设备是 MCP Server、服务器是 MCP Client**"；`session/mcp.rs` 走独立 JSON-RPC 通道（`mcp_next_id: u64`，`session.rs:146-147`），与 LLM 流式链路（`session/producer.rs`）两条独立路径；`mcp_enabled: bool`(`:145`) 单点开关 |
| 9* | **TTS ≠ AudioPlayback** | ✅ | `workflow/downlink/mod.rs:1-2` 三段分离：`framer`（PCM→重采样→Opus）/ `reservoir`（已编码帧水库）/ `sender`（按实时节拍下发）；`DownlinkPacer`(`:35`) 独立；TTS 引擎只负责"出 PCM"（`plugins/tts/mod.rs:313`），**不含播放语义** |

> **8/9 完全满足，仅 #6 Pipeline ≠ Provider 未落地**（且扩展位已预留）。

---

## 6. §16「Registry」概念混淆分析

**仓库已有三处 registry：**

| 路径 | 语义 | 索引键 | 生命周期 | 行数 |
|------|------|--------|---------|------|
| `plugins/registry/` | **Provider/能力注册表**：能力的可枚举描述（`REGISTRY`→`/api/plugins`；`FieldSchema`→`/api/config/schema`；`Built` 闭枚举）；**含 `warn_if` 软性校验**（`:395-427`） | capability / impl id | 编译期 `'static` 表 | `mod.rs` 520 + `fields*.rs` 5 + `impls.rs` |
| `device/registry/` | **设备/会话/对话进程内注册表**：管理页数据源；**含 `Phase` 会话状态机**（`types.rs:11-20`） | `device_id`/`session_id`（单调 `seq`） | 全内存重启清零；`SessionGuard` RAII 兜底（`mod.rs:36`） | `mod.rs` 529 + `types.rs` |
| `workflow/mcp/registry.rs` | **设备 MCP 工具注册表**：管理「MCP 工具」页数据源 | `device_id`（刻意不用 session_id，`:11-14`） | 全内存，`MAX_DEVICES=64`(`:33`) | `registry.rs` + `discovery.rs` |

**结论：**
1. **提案作者很可能未读 `device/registry/`**——设备注册表**已存在**（529 行，含状态机 + RAII + 单调 seq + unknown 碰撞自承）。新增 `device/registry.rs` 会造成**真实概念混淆**：三个 `registry` 已难区分，再加第四个且名字雷同。
2. **更糟的重叠**：提案的 capability 语义与 `workflow/mcp/registry.rs:63-74` `DeviceTools`（已含 `board`/`firmware_version`/`protocol_version`/`tools`）**高度重叠**。正确做法是**扩展已有 `DeviceTools`**，而非新建。
3. **命名建议**：不建议改名（纯改名零收益）。

---

## 7. 未验证声明（v2）

| 事项 | 未验证原因 |
|------|-----------|
| **提案原文全文** | 提案文档**不在仓库内**（`find docs/`、`grep -rlE "VoiceSession\|trait Transport\|八条原则"` 全仓无命中；`git status` 仅两个 wyoming 文档 untracked）。本报告依据主理人转述的条目描述核实。**若原文措辞更精确，个别判定需微调。** |
| **§21 原则条数** | 主理人转述为"八条"但列了 9 个 `≠`。我按 **9 条**核对（含 TTS ≠ AudioPlayback）。若原提案实为 8 条，请指出被合并项。 |
| **MQTT 是否用于实时音频通道** | `gateway/ws/ota.rs:20` 仅表明 **OTA** 择路支持 mqtt。**设备实时音频是否走 MQTT+UDP 未经证实**，也未读 `../xiaozhi-esp32/docs/websocket_zh.md`。G5 的可行性因此**未验证**。 |
| **AEC/NS/AGC 实现情况** | 确认 `media/opus.rs`、`pcm.rs` 存在且被 `framer.rs` 使用，但**未逐行核实**其内部是否含 AEC/NS/AGC。`ClientFeatures.aec`(`protocol.rs:76`) 未被消费，倾向"未实现"，但**无直接证据**。 |
| **Kokoro 合成是否需要超时** | 判定"Kokoro 无超时"基于 `grep -rniE timeout` 未命中 `kokoro.rs`；未评估本地 CPU 推理是否**应当**设超时（可能设计上不需要）。 |
| **`barge-in` 时序正确性** | 核实了 abort 在 6 处被轮询消费，但**未读** `reply.rs` 全文（推测 >500 行）。§3.6 的"已交付 chunk 可能已播出"是**基于 `openai.rs:304-307` 与 `retry.rs:9-11` 注释的推断**，非完整时序分析。 |
| **测试覆盖** | 各测试文件均有 mock 实现，但**未运行 `cargo test`**。本次为**纯静态核实**，所有行号判定**未经编译/运行确认**。 |
| **`docs/research/architecture-optimization-analysis.md`** | 该文（223 行，2026-10-12，对比 Golang 版）与本次提案**不是同一份**。仅引用其 `§0` 定位与 `§③` 旁证，**未通读全文**。 |

---

## 8. 判定汇总（v2）

| 判定 | 条数 | 条目 |
|------|------|------|
| ✅ **已实现** | 5 | §3 Transport抽象层 / §5 VoiceSession + 状态机 / §7 Event Bus 机制 / §9 TTFA·TTFT·TTFS / §15 Backpressure |
| ⚠️ **部分实现** | 4 | §4 Provider async / §6 Cancellation（两套作用域）/ §8 Middleware（缺文本侧管线化）/ §11 Capability（死字段）/ §13 Observability / §14 Health（缺熔断与Fallback） |
| ❌ **未实现** | 0 | （原 v1 的 3 条"未实现"经核实均属"部分实现"或"不需要"） |
| 🚫 **不需要/伪需求** | 3 | §10 Audio Focus / §12 Wyoming（已闭环）/ §11 的型号分支部分（零命中） |

**五个真缺口，按性价比排序：**
1. **G1 熔断 + 自动 Fallback**（§14f/g）——唯一"想加的新能力"；仅 LLM/TTS 可做
2. **G2 设备能力字段消费**（§11）——激活死字段；**挂已有 `mcp/registry.rs`，勿新建**
3. **G3 取消契约收敛**（§6）——补文档契约优先，**换Token 有回归风险**
4. **G4 turn 级 id + metrics 维度**（§13）——先单机版，暂不引 otel
5. **G5 MQTT+UDP 承载**（§3）——**先技术验证**，前置依赖未确认

**必须驳回的四条：** §10 Audio Focus（伪需求）/ §11 型号分支（零命中）/ §14 状态机部分（已实现）/ §5 状态机部分（已实现）。

---

## 附：方法论教训（供后续核实者参考）

**`grep "A\|B"` 在本机会静默失败**（BSD/zsh 版，`\|` BRE 交替被吞，退出码仍为 0）。
- ✅ 必须用 `grep -E "A|B"` 或 `grepE "A|B"`
- ✅ 或改用专用搜索工具（本报告 v2 已改用 `-E` 重跑全部"零命中"结论）
- ⚠️ **任何"零命中/不存在"的结论，若未经 `-E` 或专用工具复核，一律视为未验证**

---

# 第二部分 · 价值评估与过度设计审查

> 审查对象：某架构提案（Transport 抽象 / 统一 Provider / VoiceSession / CancellationToken / Typed Event Bus / Pipeline Middleware / Audio Focus / Device Capability / Wyoming / Observability / Health-Failover / Backpressure / WebRTC）。
> 审查立场：**站在本仓库真实约束上**判「必要 vs 过度设计」，不评判提案的表达质量，只判它是否解决本项目存在的问题。
> 标尺（作者自己写下的原则 + 物理约束）：
> - 部署硬件：**Intel Celeron N5105**，`docker-compose.yml` 里 `cpus: "3.5"` 硬限核（`docker-compose.yml:38`）。
> - 客户端：**ESP32 固件**，协议严格对齐小智 WebSocket，音频 **Opus**，设备端解码队列上限 **20 包 ≈ 1.2 秒**（`agents.md` §5.2c，`xiaozhi-esp32/main/audio/audio_service.h` 的 `MAX_DECODE_PACKETS_IN_QUEUE`）。
> - 作者原则（`agents.md`）：「边重构、边运行、边测试、边验证，不推倒重写」「Modular Monolith 而不是立即 Microservice 化」「学习设计思想而不是复制框架」。

---

## 1. 结论摘要

1. **提案最大的问题不是"条目多了"，而是它把大量条目当成"缺失"，而它们在本仓库已经存在。** `trait Transport` 已实现（`workflow/protocol/transport.rs`，158 行含注释，`WsTransport` 是唯一实现）；`EventBus` 已落地（`observability/events.rs`，`tokio::sync::broadcast`）；`AsrEngine`/`TtsEngine`/`LlmProvider` 三个 Provider trait 已存在；`TurnLatency` 六阶段分段指标（含 **TTFA**）已在 `observability/metrics` 聚合并在 `/api/metrics` 暴露。把这些标成 P0/P1「建议引入」，等于**让团队重做已验证的东西**，这是本提案最大的实际风险。
2. **Audio Focus（音乐 ducking / Alarm>TTS>Music 优先级 / Exclusive·Transient·Duck·Background）是明确的职责错位，建议直接拒绝。** 播放发生在 **ESP32 设备上**，服务端只发 Opus 帧；固件仓库 `grep -rli "duck|audio_focus|AudioFocus" main/` **零命中**。在服务端实现"焦点仲裁"，是给一个不存在的播放域做仲裁器。
3. **Device Capability 11 项（display/touch/camera/battery/media_player）在本项目拿不到数据，建议拒绝。** 协议里设备只声明两个 feature：`ClientFeatures { mcp, aec }`（`workflow/protocol/protocol.rs:76-81`）。而**设备真实能力（音量/屏幕/灯/媒体）已经通过 MCP `tools/list` 拿到了**，落在 `workflow/mcp/registry.rs`（含 `board` / `firmware_version` / `tools`）。再造一张静态能力表 = 与 MCP 探测结果**双源真相**，且新表必然长期与现实脱节。
4. **Transport 抽象：现状正确（已存在、泛型静态分发、注释明确写了"待出现第二承载再引入 dyn"），提案要求"再加WebRTC/MQTT 实现"应拒绝。** 固件**确有一条** MQTT+UDP 编译开关（`main/CMakeLists.txt:136-139`，`CONFIG_CONNECTION_TYPE_MQTT_UDP`），但与 WebSocket **互斥**，且固件仓库 `grep -rli webrtc main/` **零命中**——**WebRTC 在本生态没有设备侧实现**，为它写服务端实现是纯投机。
5. **trait 改 async：建议拒绝，且本仓库已有第三份独立评估给出同样结论。** [wyoming-provider.md](<wyoming-provider.md>) 第二部分 §⑥ 已做三方案对比，明确"**先 (a)，不主动做 (b)/(c)**"，理由是 `aiui.rs` 这个**已在生产跑的同步网络 provider** 就是现成范式，改 async 会**废除 `spawn_blocking` 纪律**（三处模块级注释 + `workflow/pipeline/inference.rs` 闸门设计前提）。提案与本仓库既有结论**直接冲突**，应以后者为准。
6. **真正值得提前的是被放在 P2 的两件事：并发/内存护栏（Backpressure 的真需求）与 VAD 模型池化。** 提案的 Backpressure 写成了通用 channel 策略（Wait/Drop/Cancel/Replace），但本项目的真实背压压力点是**多设备并发下的 CPU 超订与 reservoir 内存**，而闸门（`workflow/pipeline/inference.rs`）+ 水库背压（`workflow/downlink/reservoir.rs`）**已经实现**；真正未做且有实测数据的是 `agents.md` §9 明写的「**VAD 模型每会话加载，连接数大时超订**」。
7. **优先级需要整体重排：提案的 P0 里4 条中有 3 条已存在或不该做；被放在 P2/P3 的「WebRTC、Advanced Agent」应拒绝而非「延后」。**
8. **一句话结论：这份提案适合当"设计思想参考"，不适合当"本项目实施清单"。** 若照单施工，本项目最可能的结果不是"架构更优雅"，而是**在已调通的 ESP 下行节流水路上引入回归**——`agents.md` §5.2c 记录的"长回复断续"正是这类改动最容易踩坏的地方。

---

## 2. 条目级判定表

> 「已存在」列是本表最重要的一列：**提案把已有能力当新需求提出，是本提案的首要风险。**

| # | 条目 | 本项目是否成立 | 真实收益 | 建议 | 理由（证据） |
|---|---|---|---|---|---|
| ① | **Transport 抽象** `trait Transport` | ⚠️ **已存在** | 已有：`workflow/protocol/transport.rs`（`Transport` + `WsTransport`），`Session<'a, T: Transport>` 泛型静态分发，流水线零改动复用 | **保持现状**（不要重写）<br>新增实现 = 拒绝 | `workflow/protocol/transport.rs:40-65` 已定义 6 方法（含 `close` 粘滞语义），`:106-107` 注释明说"当前 WS，未来 MQTT+UDP / 本机通道各实现一份即可"。**模块注释 `:13-16` 已明确设计取舍**："不上 FrameProcessor 全家桶……待出现第二承载且需运行期切换时再引入 `dyn`"。提案若重做=推翻一条已被记录的、正确的决策 |
| ② | **统一 ASR/LLM/TTS Provider**（建议改 async） | ✅ 已存在 / ❌ async 部分**不成立** | Provider trait 已有：`AsrEngine`（`asr/mod.rs:29`）、`TtsEngine`（`tts/mod.rs:315`）、`LlmProvider`（`llm/mod.rs:163`） | **拒绝 async 化**<br>维持同步 trait | 本仓库已有**第三份独立评估**同结论：[wyoming-provider.md](<wyoming-provider.md>) 第二部分 §⑥ 判定"**先 (a)，不主动做 (b)/(c)**"。改async 会**废除 `spawn_blocking` 纪律**（`asr/mod.rs:7-9`、`tts/mod.rs:37-39`、`runtime/mod.rs:7-8` 三处模块注释 + `workflow/pipeline/inference.rs` 闸门的前提）。而 `plugins/tts/aiui.rs` 是**已在生产跑**的"同步 trait + 阻塞网络 IO + 流式回调 + `ActiveDeadline`"完整范式。**收益拿不到**（第一期仍走短连接，差异为零），成本是组织性纪律作废 |
| ③ | **VoiceSession 6 态状态机** | ⚠️ **概念已存在** | `Session` 已聚合全部会话状态（`workflow/session/mod.rs:108-150`，含 `abort`/`pacer`/`mcp_enabled`/`commands`） | **延后/拒绝显式状态机** | `Session` 已是"核心抽象"，且注释 `:103-105` 说明聚合成 `Session` 的动机正是"消除参数膨胀与漏传风险"。把隐式阶段流转改成显式 6 态枚举，会与 `stream_response`/`aiui_downlink`/`speak_canned` 三条入口（`agents.md` §5.2c 末"生命周期=一次回复"）产生**状态爆炸**：AIUI 全链路不走 ASR/LLM 阶段，若进状态机需要额外迁移边。**收益低、回归风险高** |
| ④ | **统一 CancellationToken** | ✅ **成立（真实缺口）** | 中—高：消除 `abort`/`is_cancelled` 混用；让打断可组合 | **采纳，但降为 P1** | 全仓 `grep CancellationToken` = **零命中**；打断标志是 `Arc<AtomicBool>`（`workflow/session/mod.rs:130`），在 `splitter.rs:114`、`downlink/sender.rs:55`、`downlink/reservoir.rs:355`、`sender.rs:153` 多处以**不同轮询形态**出现（`acquire_with_abort` 200ms 轮询 vs 裸 acquire）。语义已正确但**表达分散**，统一有真实收益。⚠️ 但**必须保留现有取消语义**：`workflow/pipeline/inference.rs` 纪律要求 ASR 侧**裸 acquire 不查 abort**（"stale abort 不能吞掉打断后紧跟的新话语"，`agents.md` §5.2c-i）。换token 时这条极易踩坏 |
| ⑤ | **Typed Event Bus（18 变体）** | ⚠️ **已存在（6 变体）** | 已有：`EventBus`（`observability/events.rs:212`，`broadcast` 容量 1024）+ `observability/metrics` 作为常驻订阅者 | **保持现状**（不要扩到 18） | `observability/events.rs:158-206` 已有 6 个变体（含 `TurnLatency`）。模块注释 `:18-19` 已写明容量取舍："P0 仅有调试订阅者，1024 足够；未来大盘若需零丢失，再评估换方案"。**为凑 18 变体而加事件 = 无订阅者的死字段**（现有变体已因该问题统一 `#[allow(dead_code)]`，`:156`） |
| ⑥ | **Pipeline Middleware（Audio/Transcript/Response 三层）** | 🟡 **部分成立** | 音频层：**AEC/NS/AGC 已在设备端**（固件 `main/audio_processing/audio_processor.cc`，`CONFIG_USE_AUDIO_PROCESSOR`）；VAD 已有；Resample 已有（`media/resample.rs`）。文本层：切句/定时 flush 已有（`workflow/pipeline/splitter.rs`） | **拆分采纳（文本/工具层）**<br>**拒绝内容过滤层** | **AEC/NS/AGC 照搬是职责错位**——服务端拿到的是设备**已处理完**的 Opus，解码后只有 PCM，**没有回声参考信号**，AEC 在服务端物理上不可能做（`ClientFeatures.aec` 只表示设备声明支持，`protocol.rs:79-80`）。**敏感词/Markdown/Emoji 过滤：从代码零命中，是从合规场景照搬**——本项目自用私有助手，`grep -rni "敏感词\|markdown\|emoji"` 全仓无实现。真实需求只有一条：**Markdown/符号影响 TTS 可听性**（见 §5 TOP3） |
| ⑦ | **Streaming First（TTFT/TTFS/TTFA）** | ✅ **已存在** | 已有：`TurnStages` 六里程碑（`observability/events.rs:44-58`）含 `llm_first_token_ms`、`tts_first_pcm_ms`、`first_audio_ms`，聚合在 `observability/metrics` | **保持现状** | `agents.md` §5.2c-i 记录这是"缺口②"已补齐项，p50/p90/p95/p99/max 已在 `/api/metrics` 的 `latency` 节。**提案把它当 P1 新增，是没读文档** |
| ⑧ | **Audio Focus（ducking/优先级/四态）** | ❌ **不成立（职责错位）** | **零** | **拒绝**（决断） | **播放发生在设备上**：服务端只发 Opus 帧（`workflow/downlink/sender.rs`），固件 `main/audio/audio_service.cc` 自己管解码与播放。固件仓库 `grep -rli "duck\|audio_focus\|AudioFocus" main/` = **零命中**；`grep -rli webrtc` 亦零命中。在服务端做"焦点仲裁"=给不存在的播放域建仲裁器。若真要 ducking，该做在**固件**里，且由设备侧收`tts stop`/`volume` 指令实现——**服务端已有更好的杠杆：`workflow/mcp/registry.rs` 客户端可调设备音量**（探测到的工具） |
| ⑨ | **Device Capability（11 项能力表）** | ❌ **不成立（拿不到数据）** | 负（引入双源真相） | **拒绝**（决断） | 协议侧设备只声明 **2 个** feature：`ClientFeatures { mcp, aec }`（`workflow/protocol/protocol.rs:76-81`），**display/touch/camera/battery/volume/media_player 全部不在协议里**，服务端无从得知。而**真实能力已经通过 MCP `tools/list` 拿到并落库**：`workflow/mcp/registry.rs:45-59` 的 `ProbeResult` 含 `board`（板型）、`firmware_version`、`tools: Vec<McpToolInfo>`，按 `device_id` 存最近一次结果（离线可看，`registry.rs:16-21`）。**再加一张静态能力表 = 与 MCP 探测双源冲突**，且 48 种板型（`xiaozhi-esp32/main/boards/` 48 个目录）差异巨大，静态表必然长期过期 |
| ⑩ | **Wyoming Provider** | 🟡 **成立但优先级不是 P2** | 高（本项目是**唯一**能对接 Wyoming 生态的路径） | **提前到 P1**（先TTS） | 本仓库已有详评 [wyoming-provider.md](<wyoming-provider.md>)，结论"**部分可行，TTS 收益/成本比最高**"。关键发现（第二部分 §5.1）：**PCM↔Opus 转码成本几乎为零**（引擎边界两侧本就是 PCM f32，已有 `StreamingResampler`/`f32_to_i16_16k`）。WakeWord 则**无处可插**（`[wakeup]` 是话术插件不是唤醒词引擎，§①.5）。放在 P2 偏低——它比提案里多数 P1 条目更贴合本项目 |
| ⑪ | **Observability（trace_id/OTel/Prometheus）** | 🟡 **部分成立** | 中：分段延迟已有；缺的是**跨设备关联** | **拆分采纳** | 六阶段分段延迟 + 闸门等待计数**已有**（`/api/metrics` 的 `latency`、`inference_gates` 节）。**OpenTelemetry 整套不建议引入**：单机单容器部署、`N5105` 3.5 核预算、已有零依赖的 `/api/metrics` JSON 投影 + 内置管理页（`client/` Kuikly）。OTel collector 是**常驻进程**，与"轻量"诉求正面冲突。真需要 trace，**先加 `session_id`/`turn_id` 关联即可** |
| ⑫ | **Provider Health / Failover（熔断/回退）** | 🟡 **健康检查已有，熔断收益低** | 低—中（仅 LLM 一路） | **只保留"健康页"，拒绝熔断器** | ①**健康检查已存在**：`plugins/health.rs`（capability 级本地 fs 探测，`GET /api/plugins` 投影）。②**重试/退避/预算已有**：`plugins/llm/retry.rs` 的 `RetryingLlm` 装饰器，且**三条语音场景纪律**（有界短预算、**已吐文本不重试**、只重试瞬时失败）已把"照抄 DSH"拒了。③**熔断器收益低**：ASR/TTS 是**本地引擎**（SenseVoice/Kokoro），**不会"云服务挂"**；只有 LLM 一路走远程，而 LLM 挂了现有兜底话术（`session.rs:99` `FALLBACK_REPLY`）+ `plugins/llm/failure` 已在处理。**为一个链路加熔断器，收益 < 抽象成本**。④ 提案的"自动 Fallback 到 Cloud"对本项目**方向相反**——本项目的设计前提就是**本地优先、不依赖云** |
| ⑬ | **Backpressure** | ⚠️ **大部分已实现** | 剩余真实缺口：**VAD 池化**（见 §5） | **拒绝通用策略枚举**<br>**采纳 VAD 池化（P1）** | 已实现：①推理闸门（`workflow/pipeline/inference.rs`，ASR/TTS 各一把 `Semaphore`，`max_concurrent` 默认 1）；②水库背压（`workflow/downlink/reservoir.rs::full_reservoir_backpressures_producer`）；③预取有意背压（`PREFETCH_SENTENCES`，`agents.md` §5.2c）。**提案的 Wait/Drop/Cancel/Replace 四策略是通用框架，本项目三处背压各自语义不同且已调通**，强行抽象会掩盖差异 |
| ⑭ | **WebRTC（P3）** | ❌ **不成立** | **零** | **拒绝**（非"延后"） | 固件仓库 `grep -rli webrtc main/` = **零命中**。小智生态**无设备侧 WebRTC 实现**。服务端写 WebRTC 实现 = **没有任何客户端能连上**。这是本提案最明确的投机项 |
| ⑮ | **Advanced Agent（P3）** | ❌ **本项目不需要** | 负 | **拒绝** | 本项目是**单轮语音助手**，已有：MCP 客户端（设备工具反向调用）、灵魂人格、本地图记忆。提案的 "Advanced Agent"（多步规划/任务编排）在语音单轮场景**没有需求来源**，且会与已验证的逐句预取流水线路径冲突 |

### 2.1 提案声称的 P0 vs 实际状态（速查）

| 提案 P0 条目 | 实际状态 | 证据 |
|---|---|---|
| ① Transport 抽象 | ✅ **已实现** | `workflow/protocol/transport.rs`（158 行） |
| ② 统一 Provider + 改 async | ✅ Provider 已存在 / ❌ async 建议被本仓库评估否决 | `asr/mod.rs:29`、`tts/mod.rs:315`、`llm/mod.rs:163`；[wyoming-provider.md](<wyoming-provider.md>) 第二部分 §⑥ |
| ③ VoiceSession 状态机 | ⚠️ `Session` 已聚合状态 | `workflow/session/mod.rs:108-150` |
| ④ 统一 CancellationToken | ✅ **真实缺口** | `grep CancellationToken` = 零命中 |

**四个 P0 里，两个已存在、一个概念已存在、只有 ④ 是真缺口。**

---

## 3. 过度设计 TOP5（含反噬机制）

### 🥇 TOP1 —— Audio Focus（ducking / 优先级 / 四态）

**反噬机制：职责错位 → 抽象泄漏 → 维护负担零收益。**

服务端**不下发音频**，只发 Opus 帧；**播放/解码/混音全在 ESP32**。在服务端实现"焦点仲裁"，会造出一个**没有播放器可仲裁的仲裁器**。后果不是"没用"，而是**有害**：

- 它会诱使后续开发者在服务端引入 `ducking` 状态机，进而需要在**协议里下发焦点指令**——而固件**零实现**（`grep -rli duck main/` 零命中），下发即被设备静默忽略（与 `agents.md` §5.2c 记录的固件静默丢帧是同类坑：**看起来成功，实际无效**）。
- 真正需要 ducking 的场景（设备自己播音乐时降低 TTS）**在固件侧**，服务端唯一正确的杠杆是**已有的 MCP 工具**（`workflow/mcp/registry.rs` 探测到的音量工具）——提案没有意识到这个杠杆已经存在。
- **判据**：一个抽象如果它的**状态在它所在的那个进程里根本不会被观测到**，它就是纯负债。

### 🥈 TOP2 —— WebRTC（P3）+ Transport 多实现扩张

**反噬机制：过早泛化 + 沉没成本陷阱。**

固件 `grep -rli webrtc main/` **零命中**，`CONFIG_CONNECTION_TYPE_WEBSOCKET`/`MQTT_UDP` 是**互斥编译开关**（`main/CMakeLists.txt:136-139`）。为不存在的客户端写实现，是纯投机。代价：

- WebRTC 的 ICE/DTLS/SRTP 栈在 Rust 生态里依赖重、编译体积大、维护活跃度低——**与 `Dockerfile` 三阶段构建 + 轻量镜像诉求正面冲突**。
- 更危险的是：提案用 WebRTC 论证 Transport 抽象的**必要性**（"要有多个实现才值得抽trait"）。但**`trait` 已经存在**了——所以真实后果不是"该抽的没抽"，而是**为一个不存在的实现去扩充已正确的抽象**，可能把 `workflow/protocol/transport.rs` 那份写明"再引入 dyn"克制的设计，重新拖向 `dyn` 全家桶。
- MQTT+UDP 也应**延后而非排期**：固件有开关≠你有设备。

### 🥉 TOP3 —— trait 改 async（推翻已验证方案）

**反噬机制：废除组织性纪律 → 全链路回归风险。**

这是本提案**唯一与本仓库既有结论直接冲突**的条目（[wyoming-provider.md](<wyoming-provider.md>) 第二部分 §⑥ 已判"不主动做"）。反噬机制：

- 改 async 会**废除 `spawn_blocking` 纪律**（`asr/mod.rs:7-9`、`tts/mod.rs:37-9`、`runtime/mod.rs:7-8` 三处模块级 `//!` 注释 + `workflow/pipeline/inference.rs` 整章前提）。
- `workflow/pipeline/inference.rs` 的推理闸门（`Semaphore`）**与 async 无关**（闸门管并发上限，async 管IO 模型），但注释与大量测试桩要重写（`plugins/tts/tests.rs`、`aiui/tests.rs`、`prefetch/tests.rs`、`prefetch/tests/gates.rs` 都有 `fn synthesize_stream` 桩）。
- **收益在第一期拿不到**：§⑥.3 已论证"第一期走短连接时 (a) 与 (b) 行为差异为零"。
- 而 `plugins/tts/aiui.rs` **正在生产运行**（讯飞在线 TTS，实测 ~7 倍速），它是"同步 trait + 阻塞网络 IO + `ActiveDeadline` + 流式回调"的**完整范式**。推倒一个在跑的范式，去换一个收益尚未证明的模型，是典型负期望。

### 🏅 TOP4 —— Device Capability 静态能力表（11 项）

**反噬机制：双源真相 → 抽象泄漏 → 长期腐化。**

服务端**拿不到** display/touch/camera/battery（协议里只有 `mcp`/`aec`，`protocol.rs:76-81`）。而**真实能力已经通过 MCP `tools/list` 拿到并落库**（`workflow/mcp/registry.rs:45-59`）。新增静态表的反噬：

- **两个能力来源**（MCP 实时探测 vs 静态表）必然分叉。分叉后，**每个消费方都要决定信谁**——这就是抽象泄漏。
- `registry.rs:16-21` 已写明探测的语义边界（"只存在于固件里、只有设备连着才问得到"，故按 `device_id` 落库、离线显示上次结果+`age_ms`）。**这套"能力随时间变化"的正确建模已存在**，静态表与之哲学冲突。
- 48 种板型（`main/boards/` 48 目录）差异巨大（带屏/无屏、带触控/不带），静态 11 项表**必然长期过期**，且没人维护。

### 🏅 TOP5 —— 敏感词 / Markdown / Emoji 过滤（合规场景照搬）

**反噬机制：伪需求 → 徒增TTFA → 引入内容审查争议。**

`grep -rni "敏感词\|markdown\|emoji"` 全仓**零命中**。本项目是**自用/私有**语音助手，不过审内容。反噬：

- 敏感词过滤放在响应链上 = **每轮增加一次字符串扫描 + 一个可配置词表 + 一套误伤处理**，换零收益。
- 更实际的风险：**它会坐实"LLM 输出不可信"的假设**，从而推动后续加更多内容管控——对自用助手是纯粹的复杂度通胀。
- 提案把这类归入"统一 middleware 链"，会让**真实需求（Markdown 影响 TTS 可听性）被合规噪声淹没**——真需求见 §5 TOP3。

---

## 4. 收益/成本矩阵（仅对"本项目成立"的条目）

> 度量口径：**N5105 / 4 核，容器 `cpus: "3.5"`**。CPU 预算是本项目**最硬的稀缺资源**（`runtime/mod.rs:5-13` 的预算表：tokio worker 2 + ASR 2 线程 + TTS 4 线程 + VAD 1 线程，已贴近 4 核）。

### 4.1 采纳项

| 条目 | 收益 | CPU 影响 | TTFA 影响 | 并发能力影响 | 维护成本 | 净判定 |
|---|---|---|---|---|---|---|
| **④ 统一 CancellationToken** | 打断语义统一、多处轮询收敛为一种表达 | **≈0**（内存标志，不占线程） | **≈0**（不阻塞） | 中性| **↓**（消除 `abort`/`is_cancelled` 混用与轮询形态漂移） | ✅ **采纳（P1）**。⚠️ 必须保留 `workflow/pipeline/inference.rs` 的"ASR 侧裸 acquire 不查 abort"纪律 |
| **⑩ Wyoming Provider（TTS 先行）** | 对接 Home Assistant / Wyoming 生态（本项目**唯一**通路） | **≈0**（本地仍是 Kokoro；Wyoming 为网络型不吃本地 CPU，与 `aiui.rs` 同档，**不过推理闸门**） | **可能改善**（进程级连接复用可省每句握手；第二部分 §①.1 指出 TTS 近亲形态） | **↑**（远端算力分担本地 TTS） | **↑**（新provider ~1 个文件，按§5.2b「改两处」约定） | ✅ **提前到 P1**（先做 TTS）。⚠️ 按 §①.7 先短连接，**不要**顺带改 async |
| **⑬ VAD 池化**（提案未提，从 §9 已知限制提上来） | **直接省 CPU**：VAD 模型每会话 `create` 一次（`agents.md` §9 明写），多设备时重复加载 | **↓↓ 显著**（`plugins/vad/pool.rs` 已存在雏形，`VadGuard` 已在用——把它做实） | **↓**（首帧更快，少一次模型构造） | **↑↑ 最直接**（这是真正的并发瓶颈之一） | ↓（池化是成熟模式，`pool.rs` 已有基础） | ✅ **采纳（P1，最高性价比）** |
| **⑥ 拆分：文本/工具层 middleware** | tool 检测、符号清理归位 | ≈0（纯字符串，µs 级） | **↓小幅**（更早切句→更早首帧） | ≈0 | 中性（是把散落逻辑归位，不是新增层） | 🟡 **采纳其中真实项**（tool 检测/可听符号清理），**拒绝**内容过滤与音频 DSP |

### 4.2 成本对比（为何拒绝项"成本 > 收益"）

| 条目 | 真实收益 | 真实成本 | 净判定 |
|---|---|---|---|
| ⑧ Audio Focus | **0** | 新模块 + 协议字段 + 无固件配合的死代码 | ❌ 净负债 |
| ⑭ WebRTC | **0**（无客户端） | 重依赖 + 镜像变大 + 维护活跃度低 | ❌ 净负债 |
| ⑨ Device Capability |负（双源） | 48 板型表长期腐化 + 消费方歧义 | ❌ 净负债 |
| ⑫熔断器 | 低（仅 LLM 一路；已有 retry + 兜底话术） | 状态机 + 时间窗调参 + 单机无意义的跨实例语义 | ❌ 不划算 |
| ⑪ OTel 全套 | 低（已有 `/api/metrics`） | **常驻 collector 进程吃N5105 预算** + exporter 依赖 | ❌ 与"轻量"冲突 |
| ⑤ 事件扩到 18 变体 | ≈0 | 死字段（现变体已需 `#[allow(dead_code)]`，`events.rs:156`） | ❌ 无收益 |
| ③ 显式 6 态状态机 | 低（隐式已覆盖） | AIUI 全链路等**状态爆炸** + 三条入口回归风险 | ❌ 风险 > 收益 |
| ② trait async | ≈0（第一期） | 废除 spawn_blocking 纪律 + 测试桩大面积改写 | ❌ 冲突既有结论 |

---

## 5. 建议优先级表（替代提案的 P0/P1/P2）

> 判据三问：①本项目**现在**有这个痛吗？②收益能在**N5105 + ESP 队列**约束下兑现吗？③会不会动到**已验证**的下行/打断链路？

### P0 — 立刻做（缺口真实且低风险）

| 项 | 内容 | 依据 |
|---|---|---|
| **P0-1** | **补齐 [wyoming-provider.md](<wyoming-provider.md>) 第二部分 §⑨ 的未决问题**，然后**只做 TTS 侧 Wyoming provider**（短连接起步，照 `aiui.rs` 形态） | 该文档已把方案推到"可实施"边缘，§①.1/§⑥.3 已给路线；缺的只是几个拍板 |
| **P0-2** | **VAD 模型池化**（把 `plugins/vad/pool.rs` 做实） | `agents.md` §9 明写"每次会话 `create` 会加载模型；连接数大时建议池化（待优化）"——**已知、未做、有实测依据** |
| **P0-3** | **背压护栏补测**：多设备并发下的CPU 观测（把 `inference_gates` + `latency` 落到管理页） | `workflow/pipeline/inference.rs` 已实现但缺"看得见"；N5105 上并发超订是真实风险 |

### P1 — 该做（真实收益，但需谨慎或收益稍低）

| 项 | 内容 | 依据 |
|---|---|---|
| **P1-1** | **统一 CancellationToken**（渐进替换，保留现有取消语义） | §4.1 唯一净收益为正的架构条目。⚠️ 硬约束：保留 `agents.md` §5.2c-i 纪律 |
| **P1-2** | **TTS 可听性清理**（Markdown 标记 / 多余符号 / 英文缩写误切） | `splitter.rs:15` 已自认"英文缩写 `Mr.`/`Dr.` 会被误切"；Markdown 输出进 TTS 会被念出来——**这是真实可听缺陷，不是合规过滤** |
| **P1-3** | **`session_id`/`turn_id` 贯穿日志与指标**（轻量关联，不引 OTel） | §4.2：OTel 全套被拒，但关联 ID 成本≈0 |
| **P1-4** | ASR 侧 Wyoming（若P0-1 TTS 验证通过） | 第二部分 §①.2：`Engines.asr` 无热切换路径，但与 SenseVoice 同档可接受 |

### P2 — 想清楚再做（**提案放这里的东西反而该提前**）

| 项 | 内容 | 依据 |
|---|---|---|
| **P2-1** | MQTT+UDP Transport 实现 | **仅当**你手上真有走 `CONFIG_CONNECTION_TYPE_MQTT_UDP` 的设备。固件确有此编译开关（`main/CMakeLists.txt:136-139`），但与 WS 互斥 |
| **P2-2** | ASR流式（换 `OnlineRecognizer`） | `agents.md` §9 已列为未实现；是**能力升级**而非重构 |

### P3 → ❌ 拒绝（不是延后）

| 项 | 原因 |
|---|---|
| **WebRTC** | 固件零实现（`grep -rli webrtc main/` 零命中），**无客户端可连** |
| **Audio Focus / ducking** | 播放在设备侧；固件零实现；职责错位 |
| **Advanced Agent** | 单轮语音助手无多步编排需求来源 |
| **设备能力静态表** | 协议无此数据，且 MCP 已提供真实能力 |

### 与提案优先级的关键差异（对照）

| 提案条目 | 提案优先级 | **我的建议** | 差异原因 |
|---|---|---|---|
| Transport 抽象 | P0 | **已存在，保持** | 提案未读`workflow/protocol/transport.rs` |
| Provider 统一 + async | P0 | **已存在/ 拒绝 async** | 与 [wyoming-provider.md](<wyoming-provider.md>) 第二部分 §⑥ 冲突 |
| VoiceSession 6 态 | P0 | **拒绝** | `Session` 已聚合；状态爆炸风险 |
| CancellationToken | P0 | **P1** | 真缺口，但收益温和、需保语义 |
| Typed Event Bus | P1 | **已存在，保持** | 提案未读 `observability/events.rs` |
| Pipeline Middleware | P1 | **拆分采纳** | 拒内容过滤/AEC，留 tool/符号清理 |
| Streaming First | P1 | **已存在** | 提案未读 §5.2c-i |
| Audio Focus | P1 | **❌ 拒绝** | 职责错位 |
| Device Capability | P1 | **❌ 拒绝** | 拿不到数据 + 双源 |
| Wyoming Provider | **P2** | **⬆ 提前到 P0** | 详评已就绪，收益/成本比高于多数 P1 |
| Observability | P2 | **拆分**：轻量关联✅ / OTel ❌ | collector 与N5105 冲突 |
| Health/Failover | P2 | **健康页✅ /熔断器 ❌** | retry + 兜底已覆盖；本地引擎无"云挂" |
| Backpressure | P2 | **大部分已实现**；VAD 池化 **⬆ 提前到 P0** | 真缺口在 VAD，不在 channel 策略 |
| WebRTC | P3 | **❌ 拒绝** | 无固件支持 |

---

## 6. 提案没提但该做的事（TOP3 + 补充）

### 🥇 TOP1 —— VAD 模型池化（并发瓶颈的真实所在）

`agents.md` §9 白纸黑字："**VAD 模型加载**：每次会话 `VoiceActivityDetector::create` 会加载模型；连接数大时建议池化（待优化）"。

- **为什么提案漏了**：它列了 18 个"事件/中间件/抽象"，却没有一个针对**已记录在案的性能缺陷**。
- **收益量化**：在 `cpus: "3.5"` 上，每会话新建 VAD = 重复 CPU 与内存峰值；设备数 N 时是 N 倍。池化后VAD 段（`runtime/mod.rs` 预算表里的 1 线程）从"每会话"降为"共享池"。
- **已有基础**：`plugins/vad/pool.rs` + `VadGuard` 已在用（`workflow/session/mod.rs:123`），**是把它做实而非从零设计**。
- **与提案的关系**：这才是提案 P2「Backpressure」想解决的真问题——**并发资源护栏**，而非 channel 策略枚举。

### 🥈 TOP2 —— TTS 可听性清理（Markdown/符号/缩写误切）

- `workflow/pipeline/splitter.rs:15` 自述："已知取舍：英文缩写 `Mr.`/`Dr.` 会被误切——第一版接受"。
- LLM 常见输出含 `**加粗**`、`- 列表`、`1.`、`（括号）`、Emoji。这些**会被 Kokoro/讯飞念出来**或产生异常停顿。
- **这是提案"Markdown 过滤"唯一站得住脚的形态**：动机不是合规，而是**可听性**。放在 `splitter` 出句时或 TTS 前**剥离**，成本 µs 级，收益直接可听。
- 顺带修`Mr.`/`Dr.` 误切（英文缩写表即可）。

### 🥉 TOP3 —— 首帧/打断语义的可观测化（不是加指标，而是加**告警**）

`agents.md` §5.2c 已有 `DownlinkPacer::stats`（真空次数、**设备解码队列余量最低估计**），且明确"② 是关键：上游只是略慢于实时时……正是'长回复越到后面越断续'的形态，只看真空会漏判"。

- **提案的 Observability 只增聚合维度，不增告警**。真正该做的是：把"设备队列余量 ≤1 帧"（`agents.md` §5.2c 自称**此时报"断音"**）变成**管理页上的显式告警/红标**。
- **理由**：这是一个**已被精确定位过的回归模式**（长回复断续），且已有诊断数据；缺的不是"更多指标"，而是"**指标越线要有人看见**"。这条比引入 OTel 有用百倍，且不增加任何常驻进程。

### 补充（提案完全没提，但与"轻量"直接相关）

- **保持`docker-compose.yml` 的 3.5 核预算不被侵蚀**：任何新增常驻组件（OTel collector、连接池预热、无界缓存）都应先问"它吃几核"。提案的 OTel 与多provider 预热都会侵蚀。
- **`cpus:"3.5"` 下的并发上限应显式化**：`[asr]/[tts].max_concurrent` 默认 1 是对的（N5105 承不住 2 路本地推理），建议**写进部署文档**而非只留在模块注释。
- **固件侧联动才是 ducking 的正确落点**：若将来真要做音乐 ducking，应在 `xiaozhi-esp32` 侧实现，服务端通过**已有的 MCP 工具通道**下发指令。

---

## 7. 建议向用户确认的 5 个问题

> 这 5 个是我有强烈观点、但需要**产品侧拍板**的（技术判断已明确，取舍取决于你的目标）。

### Q1 —— 目标设备范围是否包含 **Armbian/Linux/Android/iOS**？

提案 P3 说"目标支持 ESP32-S3/ESP32-P4/Armbian/Linux/Android/iOS 共用一个 Voice Runtime"，而这正是 Transport/Device Capability 抽象的**唯一真实理由**。

- **但证据**：固件 `main/boards/` 48 个目录**全是 ESP32 系列**；`main/audio_processing/` 只有 `audio_processor` + `wake_word_detect`。**没有任何非 ESP 客户端在本仓库内**。
- **请确认**：这些平台是**已有真实设备/客户端**在用，还是**未来设想**？
  - 若是**已有** → Transport 抽象值得投入，但 **Device Capability 表仍应拒绝**（改走 MCP/能力协商）。
  - 若是**设想** → 应整体降级为P3 且**不排期**（YAGNI）。
  - **我的倾向**：先问清楚，这一条决定了提案一半条目的生死。

### Q2 —— 是否已拥有（or计划采购）走 **MQTT+UDP** 的设备？

- 固件有 `CONFIG_CONNECTION_TYPE_MQTT_UDP` 编译开关（`main/CMakeLists.txt:136-139`），与 WebSocket **互斥**。
- **请确认**：你手上的设备是**自己刷固件**吗？是否可能刷 MQTT 固件？
  - 若"只刷官方 WS 固件" → MQTT Transport **延后/拒绝**（无端验证）。
  - 若"能自己刷" → MQTT+UDP 是**真实可选路径**，可进 P2。
- **注意**：MQTT 与 WebSocket 互斥，意味着**这是换承载而非并行**——对 `workflow/protocol/transport.rs` 的泛型静态分发是天然契合的（这正是现有设计的价值）。

### Q3 —— 内容过滤（敏感词/合规）是否是**真实需求**？

- 我判定为**从 HA/OVOS 合规场景照搬**，本项目自用不需要，故建议拒绝。
- **但这是产品定位问题不是技术问题**：如果你的助手**要给他人使用**（家人/同事/对外），或内容会**在 TTS 里被公开播放**，则需重新评估。
- **请确认**：使用场景是**纯自用**还是**多人/对外**？
  - 自用 → 拒绝敏感词过滤，但采纳 TTS 可听性清理（§6 TOP2）。
  - 对外 → 保留该条目，但**明确它属于新功能而非"架构对齐"**，并单独排期。

### Q4 —— 是否计划**长期多设备并发**（这决定 VAD 池化的优先级）？

- `agents.md` §9 已把 VAD 池化列为待优化；`workflow/pipeline/inference.rs` 闸门默认 `max_concurrent=1` 也是为"N5105 承不住多路"设计的。
- **请确认**：目标并发设备数（1~2 台？10+ 台？）与使用场景（家庭？小办公室？）。
  - 1~2 台 → VAD 池化可**降到 P2**，省下精力做 Wyoming TTS。
  - 10+ 台 → **VAD 池化应升到 P0**（它是最直接的多设备瓶颈）。

### Q5 —— 是否接受"提案 = 设计参考，不作为实施清单"的定位？

- 我的核心判断是：**这份提案作为"设计思想参考"是合格的**（它引用的 Wyoming/OHF/HA/OVOS 确实是正确的信息源），**但作为本项目实施清单会导致返工**——因为它把 4 个已存在能力当P0 新提。
- **请确认**：是否接受我给出的替代优先级表（§5），并把提案降级为"演进方向参考"？
  - 若你**坚持按提案排期** → 我需要至少就②（async）与④（Audio Focus）做一次书面异议留档（我已在本文件给出证据链）。
  - 若接受 → 建议下一步：把 Q1~Q4 的答案落进 §5 优先级表，再交给 architect 出实施设计。

---

## 8. 证据索引

| 结论 | 证据位置 |
|---|---|
| N5105 / 3.5 核硬约束 | `docker-compose.yml:38`（`cpus: "3.5"` + 注释） |
| CPU 预算表（4 段已贴近 4 核） | `server/src/runtime/mod.rs:5-13` |
| **Transport 已存在** | `server/src/workflow/protocol/transport.rs`（`:40-65` trait、`:106-107`取舍注释、`:13-16` "不上 FrameProcessor 全家桶"） |
| **Session 已聚合状态** | `server/src/workflow/session/mod.rs:108-150`（含 `abort`/`pacer`/`mcp_enabled`/`:103-105` 聚合动机） |
| **EventBus 已存在（6 变体）** | `server/src/observability/events.rs:158-206`（变体）、`:212-232`（`EventBus`）、`:18-19`（容量取舍）、`:156`（`#[allow(dead_code)]`） |
| **分段延迟/TTFA 已存在** | `observability/events.rs:44-58`（六里程碑）；`agents.md` §5.2c-i（缺口②已补） |
| Provider trait 已存在 | `asr/mod.rs:29`、`tts/mod.rs:315`、`llm/mod.rs:163` |
| **async 化已被本仓库否决** | [wyoming-provider.md](<wyoming-provider.md>) 第二部分 §⑥.3（"先 (a)，不主动做 (b)/(c)"）、§①.7；§⑥.2 表（改动面差一个数量级） |
| 同步网络 provider 生产范式 | `plugins/tts/aiui.rs:13`（同步阻塞 + `spawn_blocking` + 流式回调）、`ActiveDeadline`（`:46-74`） |
| **设备只声明 mcp/aec 两 feature** | `workflow/protocol/protocol.rs:76-81`（`ClientFeatures`）、`:83-97`（`ClientHello`，`features`/`transport` 未消费） |
| **设备真实能力走 MCP tools/list** | `workflow/mcp/registry.rs:45-59`（`ProbeResult`：board/firmware/tools）、`:16-21`（按 device_id 落库、离线可看） |
| **无 CancellationToken（真缺口）** | `grep CancellationToken` = 零命中；`abort: Arc<AtomicBool>` 在 `session.rs:130`、`splitter.rs:114`、`downlink/sender.rs:55,153`、`reservoir.rs:355` |
| 取消语义不可破坏的纪律 | `agents.md` §5.2c-i（ASR 侧**裸 acquire 不查 abort**）、`workflow/pipeline/inference.rs:1-16` |
| **固件零 Audio Focus/ducking** | `xiaozhi-esp32`：`grep -rli "duck\|audio_focus\|AudioFocus" main/` = 零命中 |
| **固件零 WebRTC** | `xiaozhi-esp32`：`grep -rli webrtc main/` = 零命中 |
| MQTT+UDP 是真实但互斥的编译开关 | `xiaozhi-esp32/main/CMakeLists.txt:136-139`（`CONFIG_CONNECTION_TYPE_MQTT_UDP` vs `..._WEBSOCKET`） |
| AEC/NS/AGC 在**设备**侧 | `xiaozhi-esp32/main/audio_processing/audio_processor.cc`（`CONFIG_USE_AUDIO_PROCESSOR`，`main/CMakeLists.txt:142-144`） |
| 48 种板型（能力异质） | `xiaozhi-esp32/main/boards/` = 48 个目录 |
| **内容过滤从零开始** | `grep -rni "敏感词\|markdown\|emoji"` 全仓 = 零命中 |
| TTS 可听性缺陷（真实） | `workflow/pipeline/splitter.rs:15`（`Mr.`/`Dr.` 误切）、`:23`（`MAX_SENTENCE_CHARS=50`） |
| 背压**已实现**三处 | `workflow/pipeline/inference.rs`（闸门）、`downlink/reservoir.rs`（水库背压）、`agents.md` §5.2c（`PREFETCH_SENTENCES` 有意背压） |
| **VAD 池化是已知未做项** | `agents.md` §9（"每次会话 `create` 会加载模型；连接数大时建议池化（待优化）"）；已有 `plugins/vad/pool.rs` |
| 重试/退避已有且已拒绝照抄 | `plugins/llm/retry.rs:1-15`（三条语音纪律）、`plugins/llm/failure.rs`（具名分类）、`llm/mod.rs:181-185`（`RetryingLlm` 装饰） |
| 引擎健康检查已有 | `plugins/health.rs:1-30`（capability 级本地 fs 探测，**明确禁止网络探测**） |
| LLM 失败已有兜底 | `workflow/session/mod.rs:99`（`FALLBACK_REPLY`） |
| **设备队列 20 包/1.2 秒（核心坑）** | `agents.md` §5.2c全节（`:152-266`）；固件 `main/audio/audio_service.h`（`MAX_DECODE_PACKETS_IN_QUEUE`） |
| 队列余量诊断已存在（缺告警） | `agents.md` §5.2c（② "设备解码队列余量最低估计……≤1 帧即报'断音'"） |
| plugin描述符注册表 | `server/src/plugins/registry/mod.rs:396-456`（`PluginDescriptor`/`REGISTRY`/`find`/`impls_of`）、`:73-89`（`Capability` 闭枚举） |
| Wyoming 转码成本≈0 | [wyoming-provider.md](<wyoming-provider.md>) 第二部分 §5.1 |
| Wyoming WakeWord 无接入点 | [wyoming-provider.md](<wyoming-provider.md>) 第二部分 §①.5 |
| ASR 无热切换路径 | [wyoming-provider.md](<wyoming-provider.md>) 第二部分 §①.2（`runtime/mod.rs:47` 不可变字段）；`runtime/mod.rs`（`refresh_*` 为 `RwLock` 热替换） |
