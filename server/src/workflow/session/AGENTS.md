# 会话流水线维护

入口[会话建立](<start.rs>)负责热刷新、依赖快照和注册；[主循环](<./mod.rs>)处理帧；[回复编排](<reply.rs>)串起上下文、LLM、TTS 和下行。测试台[bench](<bench.rs>)只服务 `hello.test=true`，不是生产捷径。

**传输入口定型**：`SessionParams.kind` 是入口声明的 `SessionTransportProfile`（opaque id + 是否已完成设备端 VAD），会话专用分支只读 profile，不从音频格式反推；连接欢迎语文本由入口经 `run_session` 的 `welcome` 参数注入，workflow 不反向依赖任何设备插件。共享流水线（ASR/技能/LLM/TTS/下行）只消费通用 profile；几何差异（采样率、PCM/Opus、lead）由入口经 `SessionParams` 投影，下行分帧按该投影走各自 transport adapter。

## 推理与流式生产

- ASR/TTS 同步推理通过 `spawn_blocking` 隔离，并共用[InferenceGates](<../pipeline/inference.rs>)。许可 move 进 blocking 闭包以 RAII 释放，不能在等上一句时占着许可。
- ASR 裸 acquire，不受上一轮 stale abort 影响；TTS acquire 感知 abort。缓存命中不过推理闸门；AIUI 云闭环、音色探测和模型热重建不混进本地闸门。
- [LLM producer](<producer.rs>)与[分句器](<../pipeline/splitter.rs>)旁挂 watchdog，LLM 静默时也能 flush 残句；不能把定时逻辑移回只在 token 到达时执行的回调。退出前先查 done/abort/通道关闭。
- 阶段延迟由[事件记录器](<../../observability/events.rs>) first-only 标记，使用单调钟；[观测出口](<observe.rs>)统一发事件，[metrics](<../../observability/metrics.rs>)独立聚合，接口只增不改。
- **轮次可归因**：`TurnTrace::new` 分配进程内唯一的 `turn_id`（一个 trace = 一轮），随事件负载与 `tracing` 的 `turn=` 字段流转；`grep turn=N` 能捞出整轮日志行，最后一行是分段摘要。首 token / 首句日志的判据取 `mark_*` 的返回值（是否首次写入），不要另设原子标志——那会与 first-only 语义漂移。`session_id` 在轮次内日志里不能丢：排查通常先按会话过滤再看轮次。
- **文件规模**：[reply.rs](<reply.rs>)是回复编排层（模型授权与动作出口在[command.rs](<command.rs>)、观测在[observe.rs](<observe.rs>)），当前 ~540 行仍有余量，但**观测类代码一律落[observe.rs](<observe.rs>)**（事件出口、轮次日志、id 取值口径都在那里），不要再往 reply.rs 加日志或分支。

## 可编排 hook 执行

- [计划选择](<plan.rs>)消费会话建立时编译的固定快照；空计划关闭全部可选增强，但不关闭 VAD/ASR/LLM/TTS 核心链路。条件使用真实文本和入口 transport profile id，普通上下文没有可信置信度时 `confidence` 一律不命中；模型门控在唯一分类后使用真实分数。
- `nlu.model` 是唯一模型授权节点；`command.gate` / `nlu.rules` / `skill.gate` 仅兼容别名，共存也只调用一次 `SkillRegistry::recognize`、执行一次。旧 command 绑定只授权 end_session，旧 NLU 绑定只授权 NLU 配置中的意图；条件不扩大目录权限。`confidence` 仅在该次真实分类后评估，非有限值拒绝。模型关闭/拒识可纯聊天，但不提供任何技能工具；模型故障消费本轮并明确记错，缺槽位明确澄清且不动作。
- [上下文与生命周期](<hooks.rs>)按 `link.nlu.llm` 位置顺序组装人格/召回，`stop` 在真实错误时结束当前轮，`continue` 保留核心 prompt 并继续；唤醒仍仅挂每次 `detect`，不是连接开始或 `listen start`。AIUI 托管云端链路不执行本地人格/记忆。
- `end` 写回只由正常完成、非取消、非打断的本地回复调用；会话内以单调 `turn_id` 防重入，原文 record 等待结果使错误策略真实生效，抽取/维护仍在后台。此保证不包含进程崩溃 exactly-once，也不代表设备已播放确认；背景任务失败不追溯撤销已完成轮次。
- [真实会话回归](<plan/tests.rs>)使用无模型依赖、假承载/记忆提供方验证实际副作用，不替代 ESP/EchoMuse 真机与推理联调。

## 指令、MCP 与唤醒

- [模型门控](<command.rs>)只让真实模型选择意图；模板仅为已命中的同一意图抽槽，不再做关键词、规则或启发式识别。聊天 LLM `tools=None`，即使上游擅自返回 tool_calls 也不执行，不保留 `execute_skill` 回提。新用户轮才重置 stale abort；同轮澄清/receipt 使用不清 abort 的 `speak_canned_in_turn`，任务投递走同样不清 abort 的严格音频链；原生 `AudioWrite::Cancelled` 也须转为 sticky abort，防告别中断后继续 MCP。
- [声明式技能执行](<skill_plan.rs>)先验证条件并解析全部步骤参数，再对全计划调用纯 `preflight_step_args` 检查时区枚举、时长语法/范围、at 语法和日历合法性，首 effect 前全部通过才按顺序调用 [SkillSystem](<../../skills/AGENTS.md>) 稳定白名单能力；`device_id/session_id` 固定取会话，所有 SQLite/model 阻塞工作隔离且轮询 abort，每个 step 幂等键为 `session:turn:index`。设备工具名必须是保存配置的静态字面量；一般 devices.call/legacy MCP 动作必须有本设备明确声明（has_tool=Some(true)），未知/发现失败/明确不存在都不调用、不报成功。仅 end_session 的既有 stop 兼容路径保留未知清单试探及四种降级。设备动作先成功再说 receipt；`replies.say` 延后所有步骤成功后播报。仅其 `text` 允许 `{{steps.N.speech}}` 引用已成功的前序 tasks.create/list/cancel 返回值，执行前验证索引/来源，运行时一次替换且拒绝二次注入；不允许结果控制 API、MCP、设备或动作参数。查询/创建/取消的文案来自真实服务结果，不从意图名拼造；同名取消的 typed `SkillActionError.speech` 作为错误澄清播报且终止计划，不播整体成功。`sessions.end` 仅允许末步，未执行完不得成功；计划多步骤不是跨设备/SQLite原子事务，提交过的步骤不声称可撤销，取消后不再启动新动作。
- [在线消息投递](<delivery.rs>)由主循环每秒 idle tick 拉取本设备 outbox，原子 claim → 再查取消/租约 → [严格投递音频](<delivery_audio.rs>) → finish_delivery；不调用 `handle_inject`/NLU。task_id 存在时先放[本地720ms短旋律](<alarm.rs>)再说文案，普通 messages.send 不带铃。铃声是确定性淡入淡出 PCM（不下载、不借TTS文本伪音效），与文案复用同一个 TTS start/stop、节拍器、水库、既有设备分帧/Opus或PCM下行；不能句间 reset。每个音源都必须完成合成/编码并产生帧，铃成功而文案失败仍不 submitted；完整句缓存遵守既有身份与字节/帧上限。仅本地链路真实设备可投递，测试台/AIUI 不 claim；监听/累积音频期不打断投递。投递中每200ms复查取消/租约，watcher 只置本次 delivery_abort，独立传给预取/填充/下行，不污染用户 sticky abort；SQL 取消 A 后同连接 idle tick 仍可送 B。真实 Abort/Closed 与独立原生 Cancelled 仍置用户 sticky abort，已知局部取消竞态返回的 Cancelled 不再误标用户取消，不做临时 swap/clear。薄承载包装保留实际 native send_audio/terminal_profile；所有异常路径停止并 join filler，已 start 或 drain Err 时 best-effort tts stop，清理失败不得掩盖原错或变成提交成功。成功只记 `submitted`（音频完整交给承载），绝不宣称 `played`；离线、失败保留待投递，崩溃恢复是租约重试而非 exactly-once。
- 新用户轮入口清 stale abort；同轮 abort 后不执行 MCP 或新动作。告别顺序是告别音频 → `tts stop` → MCP stop-listening。失败降级置 close_requested，由主循环跳过 VAD flush、排空后 close；不要分支直接 drop socket。
- MCP 主循环内调用必须同任务抽帧，否则等自己投递响应会死锁。能力发现保留上行音频，退下等待则丢弃残留音频；其他文本帧按序补派发。详细契约见[MCP 说明](<../mcp/AGENTS.md>)。
- [唤醒问候](<wakeup.rs>)挂 `listen detect`，不是 listen start 或 ASR；每次 detect 均问候，不能设“每会话一次”。复用 `speak_canned` 和原下行路径，跳过 LLM。
- TTS **实际租约**在会话建立时固定（`engine::TtsLease`），实例/构建配置/签名/代际原子发布；设备 overlay 仅覆写 speaker/speed。欢迎语、问候、固定应答和正常分句统一由 `HotAudioKey::from_lease` 取缓存身份（实际引擎/音色/语速/文本入磁盘身份，内存键再并入会话下行格式），重建失败继续用旧实例身份，不把请求配置当作已生效构建；别的会话热刷新不会令旧会话句间换引擎。
- EchoMuse 有界话语通过 typed epoch 化 `AudioBoundary` 结束，不改变 ESP `listen stop`：Speech 合并已切段与 flush 尾段后 reset 再识别；NoSpeech 丢弃两类残留并 reset，零 ASR/LLM。`utterance.rs` 的有界承载包装在下行轮询期间有序暂存上行（保留原生 PCM 的时长/字节 RAII 额度，另有4096项/8MiB防御上限，溢出关闭），不丢末包/结束事件；ESP 仍沿用播报期丢上行行为。
- 热音频内存→磁盘→真合成三级复用，共用[HotAudioKey](<../../media/hot_audio.rs>)；生产预取路径逐句登记完整身份（`SentenceSource.hot_audio_key`），完整句合成成功经 `insert_async` 落盘（随行提交**引擎原样 PCM**，不做重采样），磁盘命中经 `get_async` 读 PCM 按会话格式编码并暖入内存。**键分两级**：磁盘身份只含引擎签名/音色/语速/**源采样率**/文本（存裸 `.pcm` 单文件，引擎原样不重采样，换下行档位/设备不作废），内存键另含整个下行格式（已编码帧不可跨档互喂）；含凭据的签名绝不写日志；磁盘是内容寻址的裸 `.pcm` 文件（淘汰只认 `.pcm`，旧格式遗留不读取也不自动清理）；测试台等不可复现路径填 `None`。
- persist 要求 TTS 缓存开启且配置来源有路径；问候语应短，避免主循环等待播放期间拖住用户第一句话。

[下行维护](<../downlink/AGENTS.md>) · [推理设计记录](<../pipeline/inference/AGENTS.md>) · [延迟设计记录](<../../observability/metrics/AGENTS.md>) · [残句设计记录](<#3-缺口③sentencesplitter-定时-flush>)

---

## 3. 缺口③：SentenceSplitter 定时 flush

> **历史设计 / 实施记录**：本章并入自原《三项缺口实施方案（gap-implementation-design）》§3，保留原章节编号及当时的结论，不代表所有提案均已落地。
> 文中 `§N` 指原长文对应章节，完整历史记录见[三项缺口设计](<../../../../docs/designs/gap-implementation-design.md>)；当前行为先读本文件、源码与测试。

### 3.1 定时器位置取舍（核心论证）

现状：producer 是 `tokio::spawn` 的 async 任务，但**句子产出发生在同步回调里**（`llm.chat_stream` 的 `Box<dyn FnMut(LlmEvent)->bool>`，reply.rs:174-184 内 `splitter.feed(&delta)` 即时推 unbounded channel）。候选方案：

| 方案 | 结论 |
|---|---|
| A. **producer 旁挂独立 watchdog 任务**（`tokio::time::sleep_until` 到期检查共享 splitter） | ✅ 采用 |
| B. 回调内自判 | ❌ 回调只在 token 到达时执行——"停顿"期间根本没有代码在跑，物理上无法触发 |
| C. 消费者侧（prefetch worker）定时拉取 | ❌ 消费者看不到 splitter 的残句 buffer；除非也共享状态——那等于 A 却把 splitter 内部泄露给 prefetch，耦合更差 |
| D. 把 `chat_stream` 改成 async 事件循环 | ❌ 侵入引擎 trait 与实现（openai SSE 解析），为一个小功能动大接口，违反最小方案 |

方案 A 细节：`SentenceSplitter` 提到 `Arc<Mutex<..>>` 共享（回调与 watchdog 各自瞬时持锁、锁内无 await，`std::sync::Mutex` 足够）；watchdog 由 producer 侧 spawn，持有 `tx` 克隆：

- **sleep 目标**：`splitter.flush_deadline(wait)` = `last_feed + wait`（残句为空则 `None` → 只等 notify/500ms 兜底轮询）；
- **到期动作**：`take_expired(now, wait)` 返回残句（非空且静默 ≥ wait 才给）→ `tx.send`（消费者已退出则 `send` Err → watchdog 退出）；
- **误醒无害**：新 token 刷新了 `last_feed` → 到期重算 deadline 继续睡（每次停顿至多多醒一次）；
- **终止**：三个条件——producer 结束（置 `done: Arc<AtomicBool>` + `Notify::notify_waiters`，watchdog 收到即退）、`abort` 置位（select 侧轮询/醒来检查即退）、`tx.is_closed()`。Notify 与检查之间存在微小竞态窗口时，最坏多睡一个 `wait` 周期（≤ 上限 60s 内、默认 1.5s）后自行退出，可接受并注释；
- **abort 后不再出句**：watchdog 醒来先查 `done/abort` 再 `take_expired`，顺序保证打断后不追加残句。

### 3.2 配置与默认值

```rust
// plugins/llm/mod.rs :: LlmConfig 追加
/// 残句定时出句等待（毫秒）：LLM 流距上一 token 静默 ≥ 此值且残句非空时，
/// 先把残句送 TTS（TTFA 上界收紧；思考型停顿不再无限等标点或 50 字硬切）。
/// 0 = 关闭（回退现行为）。默认 1500：比常见 inter-token 间隙（20~100ms，
// 偶发 300~800ms）高一个量级，避免误切；normalize clamp 到 [0, 60000]。
#[serde(default = "default_sentence_flush_ms")]
pub sentence_flush_ms: u32,        // fn default_sentence_flush_ms() -> u32 { 1500 }
```

- 热生效：`NextSession`（`stream_response` 每轮从 `self.eff_config` 现读；仅**全局**生效——该键不在设备 overlay 白名单，`config/overlay.rs` 的 llm 段只放行 system_prompt/max_history）；`fields.rs` 的 LLM 字段表同步 1 条元数据。
- `config.example.toml` 加注释示例。
- 取值权衡写进 help：调小 → TTFA 更稳但残句更碎（中文 TTS 对碎片句韵律可接受）；调大 → 行为趋近现状。

### 3.3 数据结构与接口

```rust
// server/src/workflow/pipeline/splitter.rs 扩展（纯同步逻辑 + watchdog）
pub struct SentenceSplitter {
    buf: String,
    /// 上次 feed 时刻（定时 flush 判定；None = 还没喂过）
    last_feed: Option<Instant>,
}

impl SentenceSplitter {
    pub fn new() -> Self;
    /// 喂增量（内部记录 last_feed = now）；返回本次切出的完整句（0~N 句）。
    pub fn feed(&mut self, delta: &str) -> Vec<String>;          // 标点/50字硬切逻辑不变
    /// 流结束：倒出残句（不变）。
    pub fn flush(&mut self) -> Vec<String>;
    /// 定时 flush 判定（NEW，纯函数可单测）：静默 ≥ max_wait 且残句非空 →
    /// 取走残句（trim 后非空才返回），并刷新 last_feed（残句清空）。
    pub fn take_expired(&mut self, now: Instant, max_wait: Duration) -> Option<String>;
    /// 下一次到期时刻（NEW）：last_feed + max_wait；残句空/没喂过 → None。
    pub fn flush_deadline(&self, max_wait: Duration) -> Option<Instant>;
}

/// 残句看门狗（NEW）：producer 侧 spawn，到期把残句推入句子通道。
/// 退出三条件：done（producer 收工）/ abort / tx 关闭。锁内无 await。
pub(crate) fn spawn_flush_watchdog(
    shared: Arc<Mutex<SentenceSplitter>>,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
    abort: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    finished: Arc<tokio::sync::Notify>,
    max_wait: Duration,
)
```

```rust
// server/src/workflow/session/producer.rs（新文件：从 reply.rs 抽出的 LLM 生产者 + 分句泵）
pub(super) struct LlmProducer {
    pub sentences: mpsc::UnboundedReceiver<String>,
    pub result: tokio::task::JoinHandle<anyhow::Result<crate::plugins::llm::LlmOutput>>,
}

/// 读 LLM 流 → 按句切分（含定时 flush watchdog）→ unbounded channel。
/// 回调内：abort 轮询（不变）+ 首 token/首句 trace 埋点（缺口②）。
pub(super) fn spawn_llm_producer(
    llm: crate::plugins::llm::Llm,
    history: Vec<(String, String)>,
    user: String,
    prompt: TurnPrompt,
    abort: Arc<AtomicBool>,
    trace: Arc<TurnTrace>,
    flush_wait: Duration,          // 0 = 不 spawn watchdog（回退现行为）
) -> LlmProducer
```

`reply.rs::stream_response` 改为调用 `spawn_llm_producer(...)`（`self.engines.llm()`、`self.abort`、trace、`Duration::from_millis(self.eff_config.llm.sentence_flush_ms)`），自身瘦身为编排层（LLM 失败兜底/历史/计量逻辑原样保留）。**顺带解决 reply.rs 行数**：缺口①②③在 reply.rs 的净增量被 producer 抽出抵消，最终 ~560 行（< 600 上限）。

### 3.4 序列图（定时 flush）

```mermaid
sequenceDiagram
    participant L as llm.chat_stream(回调)
    participant SP as Arc<Mutex<Splitter>>
    participant W as watchdog任务
    participant TX as unbounded channel
    participant PF as prefetch worker

    Note over L: token 正常到达（间隙 < wait）
    L->>SP: feed(delta)（刷新 last_feed）
    SP-->>TX: 命中标点出句
    Note over L: LLM 停顿（思考长句，无标点）
    W->>W: sleep_until(last_feed + wait)
    W->>SP: take_expired(now, wait)
    SP-->>W: Some("前面那半句")
    W->>TX: send(残句)
    TX->>PF: 照常预取合成（TTFA 不再等下一个 token）
    Note over L: 流恢复/结束
    L->>SP: feed / flush
    L->>W: done=true + notify_waiters → watchdog 退出
```

---

## 上级入口

[上级 AGENTS](<../AGENTS.md>)
