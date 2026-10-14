# 架构提案 × 代码现状 差距核实报告（Gap Audit）

> 核实人：架构师 高见远（bob / software-architect）
> 核实对象：《xiaozhi-server-rust 架构优化建议》§3–§21（提案原文未入库，本报告依据主理人转述的条目描述逐条核实）
> 工作区：`/Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust`
> 核实方式：逐文件精读 + `grep -E` 计数，**所有判定均带 `文件:行号` 证据**；未读到的一律标注「未验证」。

---

## 0. 修订记录（v2，重要）

**v1 存在方法论错误，已全部修正。**

**根因**：本机`grep` 是 BSD/zsh 版本，**双引号内的 `\|` BRE 交替语法被静默吞掉**——不报错、不匹配、退出码 0（实测：`grep -n "pub struct\|pub enum" src/app/protocol.rs` 返回**空**，而 `grep -nE "pub struct|pub enum"` 正确返回 5 行）。v1 报告中所有形如 `grep -rn "A\|B\|C"` 的"零命中"结论**全部无效**。

**受影响并已修正的判定**：

| 条目 | v1 结论（错） | v2 结论（已核实） |
|------|--------------|------------------|
| §14 Provider Health | ❌ 未实现（"Phase/warn_if 零命中"） | ⚠️ **部分实现**——`Phase` 四态 + `warn_if` 早已实现于 `plugins/host.rs`；缺的是熔断/自动 Fallback |
| §5 VoiceSession 状态机 | ⚠️ "抽象已有，**状态机没有**" | ✅ **状态机已存在**——`app/registry/types.rs:11-20` 四态枚举 + 三处状态迁移点 |
| §11 Device Capability | 🚫 "硬件分支零命中" | 🚫 **结论不变但理由修正**——型号字面量零命中（真），但 `board`/`model` 在固件 OTA 路径大量使用 |
| §10 Audio Focus | 🚫 不需要 | 🚫 **结论不变**（`media_player/ducking/audio_focus/music` 重跑后确为 0 命中） |
| §8 敏感词过滤 | 🚫 零命中 | 🚫 **结论不变**，但发现 `session.rs:375` 已预留扩展位 |
| §6 Cancellation | 🚫 "零命中" | ⚠️ **前提部分修正**——`is_cancelled`/`stop_flag` 确为零命中，但存在**第二套独立取消语义**（见下） |
| §3 MQTT/UDP | "零命中" | ⚠️ 修正为**仅注释/文档提及**，无实现代码 |

**v1 唯一保留的"零命中"结论**：`trace_id`/`turn_id`/`opentelemetry`/`prometheus` 重跑后确为 0 命中（`usage.rs:95` 仅一处"供后续 Prometheus 类导出"的注释性提及）。

---

## 1. 结论摘要（v2）

1. **提案 13 个可核实条目中，5 条已实现（✅）、4 条部分实现（⚠️）、0 条完全未实现、3 条不需要/伪需求（🚫）**。v1 说"6 已实现 / 3 部分 / 3 未实现"是把"部分实现"误记为"未实现"。
2. **最严重的错误：提案把已有的 `trait Transport` 当作"待建抽象"提**。`app/transport.rs:40-65` 已定义 `trait Transport`，`app/session.rs:108` 的 `Session<'a, T: Transport>` 已泛型化消费，模块文档 `transport.rs:1-3` 甚至已论证"未来 MQTT+UDP 实现一份 trait 即可"（正是提案 §3 的理由）。
3. **第二个错误：§14 假设健康状态机需要新建**。实际 `plugins/host.rs:33-42` 已有 `Phase { Disabled, Active, Degraded, Failed }` 四态 + `:54-56` `is_deviation()` + `:123-140` 完整决策链（`warn_if`→`Degraded`、`validate` 失败→`Failed`/`Degraded`）+ `:377-380` 聚合端点。**提案把已有基建重提为待建项。**
4. **第三个错误：§5 假设状态机需要新建**。实际 `app/registry/types.rs:11-20` 已有 `Phase { Idle, Listening, Thinking, Speaking }`，且有**三处活跃迁移点**（`session.rs:422` Listening、`app/registry/mod.rs:264` Thinking、`session/reply.rs:188` Speaking）。提案点名的"Recognizing"未单列（识别合并进Listening→Thinking 之间）。
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
| 1 | **§3 Transport 抽象**（`trait Transport { send_audio / send_event / close }`，实现 WS/MQTT+UDP/WebRTC/Local） | ✅ **已实现（抽象层）/ ❌ 仅 WS 一种承载** | `app/transport.rs:40-65` trait 定义；`:30-37` `IncomingFrame`；`:79-155` `WsTransport`；`:62-64` `close()` 带设计说明；`app/session.rs:108-109` `Session<'a, T: Transport>`；`app/session/start.rs:30` `run_session<T: Transport>`<br>`grep -rniE "mqtt\|webrtc\|udp" src/` = **7 处命中，全是注释**：`transport.rs:2/11/54/60`、`session.rs:107`、`mcp.rs:10`、`ws/ota.rs:20` | **抽象层零差距**（方法名不同但语义等价）。<br>⚠️ **v1 修正**：v1 说"MQTT/UDP 零命中"，实为**注释命中、代码零命中**。`ws/ota.rs:20` 表明固件 OTA 已把 `mqtt` 列为传输选项→ **设备侧有需求、服务端侧完全空白**。这是真实的第二承载缺口（详见 §5 G5）。 |
| 2 | **§4 Provider 统一为 async trait** | ⚠️ **部分实现** | 已 async：`plugins/llm/mod.rs:163-174`、`plugins/memory/mod.rs:61`（`BoxFut`）+ `:69-95`。<br>仍同步：`plugins/asr/mod.rs:29-31`、`plugins/tts/mod.rs:315-326`、`plugins/vad/mod.rs:26-31`（流式喂帧，设计上就该同步） | 调用点实测：`.recognize(`（排除 tests）= **1 处**（`app/session.rs:318`）；`synthesize_stream(` 真实调用 3 处（`app/bench_api.rs:227`、`app/prefetch.rs:372`、`app/session/bench.rs:219`）。<br>→ 改 `asr/mod.rs` 签名 + 1 个调用点加 `.await`。**VAD 不建议改**（改 async 引入缓冲语义变化=倒退）。 |
| 3 | **§5 VoiceSession 抽象 + 状态机（Idle→Listening→Recognizing→Thinking→Speaking）** | ✅ **已实现（状态机已存在，v1 判断错误）** | 抽象：`app/session.rs:101-165` `Session<'a, T: Transport>` 聚合 transport/engines/params/session_id/device_id/eff_config/history/vad/uplink_decoder/abort/aiui/pacer/commands/mcp_*/wake/inject_rx（`:103-107` 文档："原本要传 7~11 个离散参数…统一用 `&mut self`"）。<br>**状态机（v2 新增证据）**：`app/registry/types.rs:11-20` `pub enum Phase { Idle, Listening, Thinking, Speaking }`；`:24-31` `as_str()` JSON 契约；`:34-41` `display()` 中文文案。<br>**三处活跃迁移点**：`app/session.rs:422` `note_phase(Phase::Listening)`、`app/registry/mod.rs:264` `s.phase = Phase::Thinking`、`app/session/reply.rs:188` `note_phase(Phase::Speaking)`；归位 `app/registry/mod.rs:174/304` | **v1 说"状态机没有"是错的**（因 grep 失效）。实际四态枚举 + 迁移点全部在用，界面上"这台设备在干什么"已可答。<br>⚠️ 唯一差距：提案的 **`Recognizing` 未单列**（识别阶段被合并在 `Listening→Thinking` 之间）；且`Phase` 定义在 `app/registry/types.rs`（数据模型模块）而非 `app/session/`，归属略偏。 |
| 4 | **§6 统一 Cancellation（`CancellationToken`）** | ⚠️ **前提部分成立，结论修正** | **第一套（会话级）**：`app/session.rs:130` `abort: Arc<AtomicBool>`（`:128-129` 注释"打断标志（barge-in）"），置位 `:440`，复位 `:144`（reply 入口）/`:67`（command 刻意）。clone 传递链 6 处消费：`session/producer.rs:46,66,87`→`app/splitter.rs:114,124,146`→`app/prefetch.rs:233,310`→`app/downlink/reservoir.rs:355,376,401`→`app/downlink/sender.rs:55,153`→`app/inference.rs:119,186`。<br>**第二套（引擎内局部，v2 新增）**：`plugins/llm/openai.rs:268,286,292,304,307` `let mut cancelled = false`；`plugins/tts/kokoro.rs:71,96,105,107,109` `cancelled: bool`。<br>`grep -rnE "is_cancelled\|stop_flag" src/` = **0 命中**（该结论有效）。<br>`Cargo.toml` 无 `tokio-util`。 | **v1 说"不存在多套并行机制"不准确**。实为**两套作用域不同的取消**：① 会话级意图标志；② 引擎内流读取终止标志。二者**无统一契约**（详见 §3.6 bug 风险）。<br>但 `is_cancelled`/`stop_flag` 确为零命中，且 `inference.rs:20-27` 记载的**不对称语义**（ASR 侧刻意不查 abort 以免stale abort 吞掉 barge-in 首句）`CancellationToken` 的广播语义**无法表达**。<br>→ **部分成立**：值得统一的是"取消契约"，**不是**简单换类型。建议只做 G3（见 §5）。 |
| 5 | **§7 Typed Event Bus**（18 变体） | ⚠️ **机制完备，变体数少于提案** | `app/events.rs:158-206` `ServerEvent` **6 变体**：`SessionStart`(`:160`)/`SessionEnd`(`:168`)/`MessageAdded`(`:175`)/`TtsRound`(`:183`)/`Error`(`:192`)/`TurnLatency`(`:200`)。<br>`:212-232` `EventBus` 基于 `tokio::sync::broadcast`（容量 1024，`:19` 论述取舍），`subscribe()`(`:224`)/`emit()`(`:229` 忽略所有失败)。<br>emit 点7 处：`session/start.rs:91`、`session.rs:242,392`、`session/reply.rs:216,228,240,250,267,282`。 | 传输机制（broadcast 多订阅者扇出）**完备**。提案 18 变体是 HA 场景完备性列举，本仓库只需当前 6 个。<br>⚠️ **实际只有 1 个消费者**（详见 §3.9）：全仓 `subscribe()` 唯一调用点为 `main.rs:69`→`metrics::subscriber`。**基础设施已在，缺的是多消费者用例**，不是缺基础设施。 |
| 6 | **§8 Pipeline Middleware** | 🚫 **大部分不需要；真实缺口是文本侧管线化** | 已有：句子切分 `app/splitter.rs:28-96` + `:111-165` watchdog；Resample `app/audio/resample.rs:28-39`；Opus `app/audio/opus.rs`；VAD `plugins/vad/mod.rs:26-31` + `plugins/vad/pool.rs`（**已池化**）；AEC 能力位`app/protocol.rs:72-77` `ClientFeatures{mcp,aec}`；Stop 信号范式 `app/session.rs:37-45` `enum UtteranceDecision{Respond,CommandStop}`。<br>`app/pipeline.rs` **不存在**（提案自己建议新增）。<br>`grep -rniE "media_player\|ducking\|audio_focus\|music"` = 0 命中（**重跑确认**）。<br>`grep -rniE "敏感词\|sensitive\|moderat\|profanity"` = **1 处**：`app/session.rs:375` 注释"Stage 顺序与语义（新增文本侧变换——记忆注入/**敏感词**/指令扩展——在此序列中按需…）"。 | 音频变换与句子切分**已全覆盖**，不需要 middleware 链这层间接。<br>敏感词/语言检测/Markdown 过滤**业务上不需要**（自用离线助手，不过审）。注意 `session.rs:375` 已**预留扩展位**——若将来真要做，扩展点已指定。<br>→ **真实缺口只有一个**：文本侧变换尚未管线化（command/wakeup 仍是早退 return，见 §6 原则 #6）。 |
| 7 | **§9 Streaming First + TTFA/TTFT/TTFS** | ✅ **已实现（五阶段分位齐备）** | `app/events.rs:44-58` `TurnStages`：`asr_end_ms`(`:49`)/`llm_first_token_ms`(`:51`)/`sentence_ready_ms`(`:53`)/`tts_first_pcm_ms`(`:55`)/`first_audio_ms`(`:57`)。<br>`:78-143` `TurnTrace`（first-only mark，`Instant` 单调钟）。<br>`app/metrics.rs:39-46` 五窗口；`:49-70` `stage_stats_json`(p50/p90/p95/p99/max)；`:132-157` 聚合（`:137` 按anchor 过滤）；`:199-206` `latency` 节。<br>路由 `app/ws/mod.rs:190`；投影 `app/ws/metrics.rs:21-30`。首帧真实时刻由 `downlink/sender.rs` 的 `first_frame_at` out-param 提供（`events.rs:124-137` 详述为何不能用"现在"）。 | **零代码差距**。TTFA（`first_audio_ms`）、TTFT（`llm_first_token_ms`）、TTFS（`tts_first_pcm_ms`）全部可测。 |
| 8 | **§10 Audio Session / Audio Focus** | 🚫 **不需要（伪需求，从 HA 照搬）** | `grep -rniE "media_player\|ducking\|audio_focus\|music\|duck" src/`（排除 tests）= **0 命中**（重跑确认）。<br>`plugins/` 11 个子模块无 media/player 类能力；`app/mcp.rs` 是**设备侧**MCP 原语（服务器是 Client，见 `session.rs:13-20`），工具由固件决定。<br>服务端音频出口唯一：`app/downlink.rs:1-2` "合成产物 → 水库 → 按实时节拍下发"。 | 服务端**不存在并发音频源**，无冲突可仲裁——`Alarm > TTS > Music` 三级优先级在只有一路 TTS 的服务端**无处施加**。音乐播放由固件本地负责，服务端连通道都拿不到。 |
| 9 | **§11 Device Capability 表** | 🚫 **型号分支零命中（结论不变，理由修正）** | **型号字面量零命中**：`grep -rniE "esp32_s3\|esp32_p4\|chip" src/` = 0（`protocol.rs:1,4` 的 "esp32" 是仓库名/文档路径，非型号分支）。<br>**但 board/model 大量使用（v1 遗漏）**：`app/firmware/http.rs:9,41,65,76,94,111,114,155-163,203-243` 全套固件 OTA 的 `project/model/board/version` 四元组。<br>**能力信息在 MCP 通道**：`app/mcp/registry.rs:45-59` `ProbeResult{device_id,board,firmware_version,protocol_version,tools,error}` + `:63-74` `DeviceTools`。<br>**真实缺口**：`app/protocol.rs:80-90` `ClientHello{version,features,transport}` 整个结构体标`#[allow(dead_code)]`，注释 `:80` 自承"当前流水线未消费"。 | 提案要替代的型号分支**不存在**（此结论有效）。<br>⚠️ **v1 修正**：board/model 在 OTA 路径是**大量使用的真实概念**，不是零命中——只是**不用于运行时行为分支**（仅选固件包）。<br>→ 真实缺口 = `ClientHello.transport`/`features.aec` 是死字段。**不要新建 capability 表**，扩展已有 `mcp/registry.rs:63-74` 即可。 |
| 10 | **§12 Wyoming Provider** | 🚫 **已评估完毕，不重复展开** | 见 `docs/wyoming-provider-feasibility.md`（597 行）+ `docs/wyoming-provider-research.md`（637 行）。 | 按主理人指示不重复。结论一句话见 §3.7。 |
| 11 | **§13 Observability**（四级 id + otel/prometheus） | ⚠️ **两级 id 已有，四级 id 缺**（结论不变，证据重跑） | 已有：`session_id`/`device_id` 在 `events.rs:160-205` 每个变体成对携带；`TurnTrace`(`events.rs:78-79`) 承担 turn 级锚点。另有 `app/registry/types.rs:11-20` `Phase` 提供**会话阶段维度**。<br>`grep -rniE "trace_id\|turn_id\|opentelemetry\|prometheus"` = **1 处**：`app/usage.rs:95` 注释"供测试与**后续 Prometheus 类导出**用"（仅提及，未实现）。<br>`Cargo.toml` 有 `tracing`/`tracing-subscriber`，**无** otel/prometheus。<br>另 `app/session/observe.rs:27-48` 四个 `note_*` 埋点写有状态观测面（`events.rs:5-8` 明说与事件总线"并存，不是替换"）。 | `trace_id` 透传与 span 层级**确实没有**。单机单进程下价值有限。<br>⚠️ 若做跨设备追踪**须先解`unknown` 设备键碰撞**（`app/registry/mod.rs:38`、`mcp/registry.rs:13-14` 均自承"多台会挤进同一条记录"）。<br>**建议**：先做 turn 级 id + metrics 维度，暂不引 otel。 |
| 12 | **§14 Provider Health / Failover** | ⚠️ **部分实现（v1 判定错误，已拆分重判）** | **健康状态机 =已实现**：`plugins/host.rs:33-42` `pub enum Phase { Disabled, Active, Degraded, Failed }`；`:47-53` `as_str()`；`:54-56` `is_deviation()`；`:123-140` 完整决策链（`:128-129` `warn_if`→`Degraded` + 原因 / `validate` 失败 + `fatal_if_enabled`→`Failed`，否则→`Degraded`）；`:180` 模块注释"记原因 → `Phase::Degraded`，**服务照常启动**（记忆坏了不能让语音不可用）"。<br>字段定义：`plugins/registry/mod.rs:395-427` `PluginDescriptor.warn_if: Option<fn(&Config) -> Option<String>>`（文档明写"软性问题（能构建、但能力受限）…与 `validate` 的分工"）。<br>聚合端点：`app/ws/health.rs:54-61` phase 严格程度序（Active<Degraded<Failed，Disabled 最低）；`:84-90` probe 合并（必需能力 Fail→`Failed`，可选→`Degraded`）；`:128-131` 四计数 `active/degraded/failed/disabled`。<br>**超时不完整**：LLM `plugins/llm/openai.rs:104-105` `connect_timeout`+`timeout`；TTS-AIUI `plugins/tts/aiui.rs:38,40` `TOTAL_TIMEOUT=30s`/`READ_TIMEOUT=15s`。**Kokoro 本地合成无超时**（CPU 推理，难设）。<br>**重试=已有（仅 LLM）**：`plugins/llm/retry.rs:29-35` `RetryingLlm` 装饰器；`plugins/llm/failure.rs:21-25` `MAX_RETRIES=2`/`RETRY_BUDGET=8s`；`:41-56` `LlmFailure` 六分类 + `:60-67` `retryable()`（4xx 除 429 不重试）。<br>**熔断= 零命中**：`grep -rniE "circuit\|breaker\|half.?open" src/` = 0。<br>**自动 Fallback = 零命中**：`Phase::Degraded` 只上报不切换（`host.rs:180` 明说"服务照常启动"）。 | **v1 判"未实现"是错的**（grep 失效导致）。拆成5 个子项重判，见 §3.1。<br>核心结论：**「状态上报」已完备，「故障自愈」完全空白**——`Degraded` 如实上报但不自动切换实现。 |
| 13 | **§15 Backpressure** | ✅ **已实现（三层齐备）** | ① 推理闸门 `app/inference.rs:1`（模块定位）、`:40` `Semaphore`、`:119` `acquire_abortable`、`:59-66` `Gate`（可热换+排队计数）；配置 `plugins/asr/mod.rs:46-53` `max_concurrent` + `:62-64` clamp≥1。<br>② 水库 `app/downlink/reservoir.rs:209` `mpsc::channel::<DownlinkItem>(cap_items)`（**有界**）、`:355/376/401` abort 感知。<br>③ 节拍器 `app/downlink.rs:35` `DownlinkPacer`（`:11-24` 长注释说明 ESP 20 包硬约束）。<br>④ 预取预算 `app/prefetch.rs:141` `reserve()`、`:321`/`:356` 有界 channel。<br>⑤ 其他有界通道 `session/start.rs:114`、`session/aiui.rs:108`、`session/bench.rs:210`。 | **提案要的三层全部存在**，且比提案更细（abort 感知排队、水位观测、可热换许可数）。<br>无界通道仅用于流内句子队列（`session/producer.rs:50`、`session/reply.rs:341`），是**刻意**的（句子流以 LLM 产出速率自然背压）。 |

---

## 3. 深入核实：新增与修正（v2 重点）

### 3.1 §14 拆解重判：状态上报 vs 故障自愈（团队指正的重点）

提案把五样东西捆在"Provider Health/Failover"一个条目里，导致我v1 误判为整体未实现。逐项重判：

| 子项 | 判定 | 证据 | 说明 |
|------|------|------|------|
| **14a 健康状态机**（Phase 四态） | ✅ **已实现** | `plugins/host.rs:33-42`；`:47-53` `as_str()`；`:54-56` `is_deviation()` | 四态齐全（`Disabled`/`Active`/`Degraded`/`Failed`），对齐 DSH `FiberState` |
| **14b 降级告警**（`warn_if`） | ✅ **已实现** | `plugins/host.rs:128-129`；字段 `plugins/registry/mod.rs:395-427` | 软性问题→`Degraded`+原因；`validate`（硬条件）失败→`Failed`/`Degraded`，分工清晰 |
| **14c状态聚合/上报** | ✅ **已实现** | `app/ws/health.rs:128-131` 四计数；`:54-61` 严重度序；`:84-90` 与 probe 结果合并 | 端点已可用；`host.rs:180` 明确"服务照常启动"哲学 |
| **14d Timeout** | ⚠️ **部分实现** | LLM `openai.rs:104-105`；MCP `app/mcp.rs:71` `DEFAULT_CALL_TIMEOUT=2s`；TTS-AIUI `aiui.rs:38,40` `30s/15s`；闸门轮询 `inference.rs:45` `ABORT_POLL_MS=200` | 在线provider 有超时；**Kokoro 本地合成无超时**（CPU 推理，设超时意义有限）。缺口在于**超时值不可配**（全为const） |
| **14e Retry** | ⚠️ **部分实现（仅 LLM）** | `plugins/llm/retry.rs:29-35`；`failure.rs:21-25`（2 次/8 秒）；`:60-67` `retryable()` 六分类 | 语音场景纪律完备（已吐文本不重试）。**TTS/ASR 无重试**（合理：ASR 无云端备胎，TTS 重试会重复发音） |
| **14f Circuit Breaker** | ❌ **未实现** | `grep -rniE "circuit\|breaker\|half.?open" src/` = **0 命中**（重跑确认） | 无熔断状态机（Closed/Open/HalfOpen） |
| **14g 自动 Fallback** | ❌ **未实现** | `host.rs:180`"服务照常启动"；`Phase::Degraded` 仅上报 | `Degraded` **不触发实现切换**。这是与 14a-14c 的**本质区别** |

**核心洞察（提案没讲清的地方）**：
- **`Phase` 是"观测面"，不是"控制面"**。它回答"现在哪个实现不健康"，但不回答"健康的话该切到谁"。
- 自动 Fallback 需要三层，**当前一层都没有**：① 探测（14c 部分有）→ ② **决策**（该切到哪个实现？按什么优先级？`PluginDescriptor` 无`fallback_to` 元数据）→ ③ **切换 + 状态机维护**（熔断器，防抖动）。
- ⚠️ 且**并非所有能力都能 Fallback**：ASR/VAD 是纯本地 sherpa，**无云端备胎**。只有 LLM/TTS 两类值得做。

### 3.2 §14 方法论错误的根因（必须记录，避免复发）

```
$ grep -n "pub struct\|pub enum" src/app/protocol.rs     # → 空（错！）
$ grep -nE "pub struct|pub enum" src/app/protocol.rs    # → 5 行（对）
```
本机 `grep` 为 BSD/zsh 版，**双引号内 `\|` BRE 交替被静默吞掉**：不报错、不匹配、**退出码仍为 0**。v1 报告中所有 `"A\|B\|C"` 形式的"零命中"结论均因此失效。**v2 起所有 grep 均改用 `-E`。**

### 3.3 §5 状态机：v1 判断错误的修正

v1 称"状态机不存在"，实为grep 失效。实际：

```
app/registry/types.rs:11-20   pub enum Phase { Idle, Listening, Thinking, Speaking }
app/session.rs:422            note_phase(Phase::Listening)      // 设备上报 listen start
app/registry/mod.rs:264       s.phase = Phase::Thinking          // ASR 完成
app/session/reply.rs:188      note_phase(Phase::Speaking)        // 开始下行音频
app/registry/mod.rs:174/304   Phase::Idle                        // 登记 / 归位
```
**四态 + 四处迁移点全部在用**，界面上"这台设备在干什么"已可答（`types.rs:34-41` 附中文文案）。提案 §5 的状态机**已基本实现**，唯一差距是提案的 `Recognizing` 未单列（识别合并在 `Listening→Thinking` 之间），且枚举定义在 `app/registry/types.rs`（数据模型模块）而非 `app/session/`，归属略偏。

### 3.4 §3 MQTT+UDP：真实缺口的精确定位

`grep -rniE "mqtt|webrtc|udp"` = 7 处，**全是注释**：

| 位置 | 性质 |
|------|------|
| `app/transport.rs:2` | 模块文档："未来 MQTT+UDP / 本机测试通道等具体承载" |
| `app/transport.rs:11` | 设计取舍："MQTT 走自身心跳" |
| `app/transport.rs:54` | trait 注释："自带心跳机制的承载（如 MQTT）留空即可" |
| `app/transport.rs:60` | `close()` 注释："其他承载（MQTT/本机通道）按各自协议关闭" |
| `app/session.rs:107` | `Session` 文档："MQTT+UDP / 本机通道各实现一份 Transport 即可" |
| `app/mcp.rs:10` | "JSON-RPC 套在既有那条 WS/MQTT 上（固件 `Protocol::SendMcpMessage`）" |
| **`app/ws/ota.rs:20`** | **实质性**："`mqtt`（官方返回 → 固件优先选 MQTT）；`activation`（官方激活流程，自建端不需要）" |

**关键发现**：前6 处是"为未来预留"的文档化设计，**抽象层已就绪**；第 7 处 `ws/ota.rs:20` 显示**固件 OTA 协议已经把 `mqtt` 作为传输选项之一**，且已实现"官方返回 → 固件优先选 MQTT"的择路逻辑。

→ **设备侧有 MQTT 需求，服务端侧完全空白**。抽象层（trait Transport）已就位，接入成本 = 写一份 `impl Transport`。这是 WAKE_DASHBOARD 类重构的前置依赖。

### 3.5 §9 事件总线：基础设施完备，但只有一个消费者

**`ServerEvent` 变体数与位置**：`app/events.rs:158-206`，**6 个变体**（`SessionStart`/`SessionEnd`/`MessageAdded`/`TtsRound`/`Error`/`TurnLatency`）。

**是否支持多订阅者**：**是**，机制完备——
- `app/events.rs:213` `tx: broadcast::Sender<ServerEvent>`（`tokio::sync::broadcast`，容量 1024）
- `:224` `pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent>`（每次调用产出独立 receiver，**语义确为多订阅者扇出**）
- `:229-231` `emit()` 忽略一切发送失败（无订阅者 `SendError` / 慢消费者 `Lagged` 都不影响发布者）
- `:19` 容量取舍文档："订阅者处理不过来会丢最旧事件（LAGGED），但发布者永不阻塞"

**实际有几个消费者**：**只有 1 个**。全仓 `subscribe()` 的唯一调用点是 `src/main.rs:69`：
```rust
tokio::spawn(crate::app::metrics::subscriber(
    engines.metrics.clone(),
    engines.event_bus.subscribe(),   // ← 唯一订阅点
));
```
`metrics::subscriber`（`app/metrics.rs:212-222`）消费并聚合到 `Metrics`，经 `app/ws/metrics.rs:21` 暴露 `GET /api/metrics`（路由 `app/ws/mod.rs:190`）。

**关于 `app/inject.rs`**：主理人问它是否是另一个通道——**不是事件总线的消费者**。它是**设备消息注入通道**，用 `mpsc`（`app/inject.rs` + `session/start.rs:114` `mpsc::channel(CHANNEL_CAPACITY)`），与会话主循环 `select!` 直接消费（`session.rs:164` `inject_rx: Option<mpsc::Receiver<String>>`）。它**不经过 `EventBus`**，注入轮次通过 `events.rs:33` 的 `ANCHOR_INJECT` 锚点进入延迟统计，属**延迟口径的衔接**而非通道共享。

**结论表述建议（供报告引用）**：
> **broadcast 基础设施已在且完备（多订阅者扇出、失败不崩、1024 容量有文档论证），当前只有 `metrics` 一个消费者。缺的不是基础设施，而是多消费者用例**——若要加审计/告警/大盘订阅者，`subscribe()` 直接可用，**无需改动事件总线**。

### 3.6 §6 双重取消源：真bug 风险（团队认可的发现，展开）

v1 说"多套并行机制不存在"不准确。实际是**两套作用域不同的取消**，且**缺统一契约**：

| | 第一套：会话级 `abort` | 第二套：引擎内 `cancelled` |
|---|---|---|
| 位置 | `app/session.rs:130` `Arc<AtomicBool>` | `plugins/llm/openai.rs:268`；`plugins/tts/kokoro.rs:71`（局部 `bool`，非 `Arc`） |
| 语义 | **用户意图**（barge-in 打断） | **流读取已终止**（HTTP 流终止标志） |
| 作用域 | 跨 producer/prefetch/reservoir/sender/inference 6 处 | 单个引擎函数内 |
| 传播 | `clone()` 逐层传递 | 函数内本地变量，不可跨层 |

**风险点**：LLM 流被 `abort` 打断时，引擎内 `cancelled` 只终止**HTTP 流读取**，**不保证已送出的 chunk 被丢弃**。`openai.rs:304,307` 用`if !cancelled && out.text.trim().is_empty()` 做判断，语义是"没被取消且空回复才算失败"——但**已通过 `on_event` 交付给上层的 chunk 属于副作用**，取消后仍可能已播出一部分。

**唯一防线**：`plugins/llm/retry.rs` 的"已交付文本就不重试"纪律（`retry.rs:9-11` 注释明说"调用方收到 `LlmEvent::Text` 时很可能已经在下行音频了，重试会让用户听两遍前半句"）。

**因此 §6 的正确结论**：提案想统一的方向**有价值**，但真问题不是"多套并存"（那是 v1 的误判），而是**"取消意图"与"取消执行"之间没有契约**——`CancellationToken` 能统一"意图广播"，但**不能自动解决"已交付副作用的回滚"**（那需要幂等性设计，超出提案范围）。

### 3.7 §12 Wyoming：一行结论

见 `docs/wyoming-provider-feasibility.md`（597 行）：**不必引入独立 Wyoming 协议层**——现有五大 trait（`AsrEngine`/`TtsEngine`/`LlmProvider`/`VadEngine`/`FullChainEngine`）+ `plugins/registry` 的 `Built` 闭枚举已是本地/远端共用的最小公共面，新增 Wyoming 实现 = 加1 条描述符。上一轮已闭环，**不重开为新任务**。

### 3.8 §11 board/model 的真实使用（v1 遗漏修正）

v1 称"`board` 零命中"，实为**grep 失效导致的误报**。实际 `app/firmware/http.rs` 大量使用：

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
| **G2 设备能力字段消费** | §11 | **新增 1 文件 + 改 2–3 处**（小–中） | ① 消费 `app/protocol.rs:90` `ClientHello.transport`（去`#[allow(dead_code)]`）；② 决定 `ClientFeatures.aec`(`:76`) 用途；③ **扩展已有 `app/mcp/registry.rs:63-74` `DeviceTools`**，**不新建 capability 表**（避免 §16 的三处 registry 混淆）。 |
| **G3 取消契约收敛** | §6 | **改 3–6 处**（中，**有回归风险**） | ① 在 `app/session.rs:130` 附近补一份**取消契约文档**（明确 `abort` = 用户意图、`cancelled` = 执行终止、各层谁负责丢弃副作用）；② `plugins/llm/retry.rs` 显式记录"已交付 chunk"边界；③ **不建议**换 `CancellationToken`——`inference.rs:20-27` 的ASR 非对称语义无法用广播式 token 表达。<br>⚠️ 若强行统一，**必须先补 barge-in 回归测试**。 |
| **G4 turn 级 id + metrics 维度** | §13 | **新增 1 文件 + 改 3–5 处**（中） | ① `TurnTrace`(`events.rs:78`) 加 `turn_id`；② `app/metrics.rs:104` `observe` 加 turn 维度；③ `/api/metrics`(`:199`) 加该维度。<br>⚠️ 引 otel 前须先解 `app/registry/mod.rs:38` 的 `unknown` 设备键碰撞。<br>**建议**：先做单机版，暂不引 otel/prometheus。 |
| **G5 MQTT+UDP 承载**（v2 新增，从 §3拆出） | §3 | **大重构前置**（不单列实施） | 抽象层已就绪（`trait Transport`），接入= 写一份 `impl Transport` +协议编解码。**前置依赖**：需先确认设备侧 MQTT 固件的实际接入形态（`ws/ota.rs:20` 显示 OTA 已有 MQTT 择路逻辑，但**实时音频通道是否走 MQTT 尚未验证**）。<br>**建议**：先做技术验证（1–2 天），不直接排期。 |

**总计：5 个真缺口，0 个需要「大重构」。** 其中 G1/G4/G5 为中等改动，G2/G3 为小改动但 G3 有回归风险。

---

## 5. §21 八条原则逐条核对（v2）

| # | 原则 | 满足 | 证据 |
|---|------|------|------|
| 1 | **Transport ≠ Session** | ✅ | `app/session.rs:108-109` `Session<'a, T: Transport> { transport: &'a mut T, ... }`；`session/start.rs:30` `run_session<T: Transport>` 泛型静态分发；`transport.rs:1-3` 文档 |
| 2 | **Session ≠ Connection** | ✅ | `app/session.rs:1-8`+`:141-160`：指令闸门命中后**用 MCP 让设备结束聆听、音频通道刻意保留**（"设备回到 idle 而会话还活着；下次唤醒复用连接不重连"）；`close_requested: bool`(`:140`) 仅在不支持 MCP 或调用失败时降级断开 |
| 3 | **Session ≠ Provider** | ✅ | `app/session.rs:110-111` 只持 `engines: Arc<Engines>` + `eff_config: Arc<Config>`，不new 任何 provider；构建全在 `plugins/registry/impls.rs` |
| 4 | **Provider ≠ Implementation** | ✅ | `plugins/registry/mod.rs:19-23`：**`Built` 是闭枚举**（非 `Box<dyn Any>`），"枚举 + 按能力 `match` 保持 100% 编译期检查"；`:15-18` `REGISTRY`→`/api/plugins`、`FieldSchema`→`/api/config/schema`、"新增实现 = 表里加 1 条" |
| 5 | **Device ≠ Hardware Model** | ✅ **更彻底** | 型号字面量零命中（`esp32_s3`/`esp32_p4`/`chip` = 0）；`board`/`model` 仅用于 OTA 包选择（`app/firmware/http.rs:65-243`）**不影响运行时行为**；设备身份用 `Device-Id`（`session.rs:113-115`），能力靠 MCP `tools/list` 运行时发现（`session.rs:18-20`）。**仓库已按此原则设计** |
| **6** | **Pipeline ≠ Provider** | ⚠️ **部分满足** | Provider 侧已解耦（#3/#4）；但文本侧**未管线化**：无 `app/pipeline.rs`，控制流靠 `app/session.rs:37-45` `enum UtteranceDecision`（**仅两态**），command/wakeup 仍是早退return。**但扩展位已预留**：`app/session.rs:375` 注释"Stage 顺序与语义（新增文本侧变换——记忆注入/敏感词/指令扩展——在此序列中按需…）"。<br>→ **唯一未落地原则**，且是 §8 的真实缺口 |
| **7** | **Event ≠ Direct Dependency** | ✅ | `app/events.rs:5-8` 明说与 `observe.rs` `note_*` "**并存，不是替换**"；`:14` "观测面绝不允许把语音链路带崩"；`:150-151` "所有字段均为只读快照…**不能据此改变流水线行为**（决策仍在会话内部）" |
| **8** | **LLM ≠ Tool** | ✅ | `app/session.rs:13-20` "## 与设备的关系（MCP 方向，勿反）——**设备是 MCP Server、服务器是 MCP Client**"；`session/mcp.rs` 走独立 JSON-RPC 通道（`mcp_next_id: u64`，`session.rs:146-147`），与 LLM 流式链路（`session/producer.rs`）两条独立路径；`mcp_enabled: bool`(`:145`) 单点开关 |
| 9* | **TTS ≠ AudioPlayback** | ✅ | `app/downlink.rs:1-2` 三段分离：`framer`（PCM→重采样→Opus）/ `reservoir`（已编码帧水库）/ `sender`（按实时节拍下发）；`DownlinkPacer`(`:35`) 独立；TTS 引擎只负责"出 PCM"（`plugins/tts/mod.rs:313`），**不含播放语义** |

> **8/9 完全满足，仅 #6 Pipeline ≠ Provider 未落地**（且扩展位已预留）。

---

## 6. §16「Registry」概念混淆分析

**仓库已有三处 registry：**

| 路径 | 语义 | 索引键 | 生命周期 | 行数 |
|------|------|--------|---------|------|
| `plugins/registry/` | **Provider/能力注册表**：能力的可枚举描述（`REGISTRY`→`/api/plugins`；`FieldSchema`→`/api/config/schema`；`Built` 闭枚举）；**含 `warn_if` 软性校验**（`:395-427`） | capability / impl id | 编译期 `'static` 表 | `mod.rs` 520 + `fields*.rs` 5 + `impls.rs` |
| `app/registry/` | **设备/会话/对话进程内注册表**：管理页数据源；**含 `Phase` 会话状态机**（`types.rs:11-20`） | `device_id`/`session_id`（单调 `seq`） | 全内存重启清零；`SessionGuard` RAII 兜底（`mod.rs:36`） | `mod.rs` 529 + `types.rs` |
| `app/mcp/registry.rs` | **设备 MCP 工具注册表**：管理「MCP 工具」页数据源 | `device_id`（刻意不用 session_id，`:11-14`） | 全内存，`MAX_DEVICES=64`(`:33`) | `registry.rs` + `discovery.rs` |

**结论：**
1. **提案作者很可能未读 `app/registry/`**——设备注册表**已存在**（529 行，含状态机 + RAII + 单调 seq + unknown 碰撞自承）。新增 `device/registry.rs` 会造成**真实概念混淆**：三个 `registry` 已难区分，再加第四个且名字雷同。
2. **更糟的重叠**：提案的 capability 语义与 `app/mcp/registry.rs:63-74` `DeviceTools`（已含 `board`/`firmware_version`/`protocol_version`/`tools`）**高度重叠**。正确做法是**扩展已有 `DeviceTools`**，而非新建。
3. **命名建议**：不建议改名（纯改名零收益）。

---

## 7. 未验证声明（v2）

| 事项 | 未验证原因 |
|------|-----------|
| **提案原文全文** | 提案文档**不在仓库内**（`find docs/`、`grep -rlE "VoiceSession\|trait Transport\|八条原则"` 全仓无命中；`git status` 仅两个 wyoming 文档 untracked）。本报告依据主理人转述的条目描述核实。**若原文措辞更精确，个别判定需微调。** |
| **§21 原则条数** | 主理人转述为"八条"但列了 9 个 `≠`。我按 **9 条**核对（含 TTS ≠ AudioPlayback）。若原提案实为 8 条，请指出被合并项。 |
| **MQTT 是否用于实时音频通道** | `app/ws/ota.rs:20` 仅表明 **OTA** 择路支持 mqtt。**设备实时音频是否走 MQTT+UDP 未经证实**，也未读 `../xiaozhi-esp32/docs/websocket_zh.md`。G5 的可行性因此**未验证**。 |
| **AEC/NS/AGC 实现情况** | 确认 `app/audio/opus.rs`、`pcm.rs` 存在且被 `framer.rs` 使用，但**未逐行核实**其内部是否含 AEC/NS/AGC。`ClientFeatures.aec`(`protocol.rs:76`) 未被消费，倾向"未实现"，但**无直接证据**。 |
| **Kokoro 合成是否需要超时** | 判定"Kokoro 无超时"基于 `grep -rniE timeout` 未命中 `kokoro.rs`；未评估本地 CPU 推理是否**应当**设超时（可能设计上不需要）。 |
| **`barge-in` 时序正确性** | 核实了 abort 在 6 处被轮询消费，但**未读** `reply.rs` 全文（推测 >500 行）。§3.6 的"已交付 chunk 可能已播出"是**基于 `openai.rs:304-307` 与 `retry.rs:9-11` 注释的推断**，非完整时序分析。 |
| **测试覆盖** | 各测试文件均有 mock 实现，但**未运行 `cargo test`**。本次为**纯静态核实**，所有行号判定**未经编译/运行确认**。 |
| **`docs/architecture-optimization-analysis.md`** | 该文（223 行，2026-10-12，对比 Golang 版）与本次提案**不是同一份**。仅引用其 `§0` 定位与 `§③` 旁证，**未通读全文**。 |

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