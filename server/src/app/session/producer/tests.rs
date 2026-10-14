//! `server/src/app/session/producer.rs` 的单元测试。
//!
//! 覆盖缺口③在 producer 层的行为级语义（QA Round 1 建议 4 补测）：
//! ① `flush_wait = 0` 不启用 watchdog——LLM 挂起流（半句后停顿）在窗口期内不出残句；
//! ② `flush_wait = 50ms` 停顿后出残句；
//! ③ watchdog 出的残句同样 mark `sentence_ready`（阻塞 2 修复的回归）；
//! ④ 正常标点出句与流结束 flush 不受 watchdog 影响。
//!
//! 不用实现细节断言（如数任务数），全部断言"通道里有没有句子"这一可观测行为。
//!
//! ⑤ **打断后句子不再进入通道**（含流结束时的残句 flush）：取消契约在producer 层的落地，
//!    见 [`crate::plugins::llm::LlmTurnResult`] 的类型文档。
//!
//! ⑥ **一轮的日志都带同一个 `turn=`**（轮次级可归因）：用真实 `tracing` 订阅器捕获
//!    首 token / 首句就绪两条日志的结构化字段，断言它们等于该 `TurnTrace` 的 `turn_id`。
//!    这里选 producer 作为切入点，因为它是**唯一能在无引擎环境下跑通真实日志路径**的
//!    轮次环节（`Session` 需要真实 VAD 模型才能构造，见 `wakeup/tests.rs` 的说明）。

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;

use super::{spawn_llm_producer, ProducerTurn};
use crate::app::events::{TurnTrace, ANCHOR_SPEECH_END};
use crate::plugins::llm::{
    LlmEvent, LlmProvider, LlmTurnResult, ToolCall, ToolSpec,
};
use crate::plugins::prompt::TurnPrompt;

/// 可编程的假 LLM：先把 `prefix` 一次性吐给回调（模拟半句无标点输出），
/// 然后**挂起** `hold`（模拟思考停顿——期间没有任何 token 到达），
/// 最后按 `finish` 决定是"继续吐到流结束"还是"就此结束"（都算正常收工）。
struct HangingLlm {
    prefix: &'static str,
    hold: Duration,
    /// 挂起结束后再吐的尾巴（可为空串 = 直接收工）。
    tail: &'static str,
    /// 收工前是否补一个句末标点（决定尾巴是否成句）。
    punct: bool,
}

impl LlmProvider for HangingLlm {
    fn chat_stream<'a>(
        &'a self,
        _history: &'a [(String, String)],
        _user_text: &'a str,
        _prompt: &'a TurnPrompt,
        _tools: Option<&'a [ToolSpec]>,
        mut on_event: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmTurnResult>> + Send + 'a>> {
        Box::pin(async move {
            if !self.prefix.is_empty() {
                on_event(LlmEvent::Text(self.prefix.to_string()));
            }
            tokio::time::sleep(self.hold).await;
            if !self.tail.is_empty() {
                let mut t = self.tail.to_string();
                if self.punct {
                    t.push('。');
                }
                on_event(LlmEvent::Text(t));
            }
            on_event(LlmEvent::Done);
            Ok(LlmTurnResult::default())
        })
    }
}

fn trace() -> Arc<TurnTrace> {
    TurnTrace::new(ANCHOR_SPEECH_END)
}

/// 轮次上下文（`session_id` 只进日志，测试里固定值即可）。
fn turn_ctx(t: Arc<TurnTrace>) -> ProducerTurn {
    ProducerTurn { trace: t, session_id: "s1".to_string() }
}

fn abort() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn prompt() -> TurnPrompt {
    TurnPrompt::default()
}

/// **wait=0 不启用 watchdog**：半句挂起 150ms（远超假定的 50ms 阈值）期间，
/// 句子通道必须安静——残句只能等流结束时由 producer 的 flush 倒出。
#[tokio::test]
async fn zero_wait_disables_watchdog_no_residual_during_pause() {
    let llm: crate::plugins::llm::Llm = Arc::new(HangingLlm {
        prefix: "思考中尚未成句的半句",
        hold: Duration::from_millis(150),
        tail: "结束",
        punct: false,
    });
    let producer = spawn_llm_producer(
        llm,
        Vec::new(),
        "用户问句".into(),
        prompt(),
        abort(),
        turn_ctx(trace()),
        Duration::ZERO, // 0 = 关闭定时 flush（回退现行为）
    );
    let mut rx = producer.sentences;
    // 观察窗（120ms < 挂起 150ms）：watchdog 关闭时通道必须无句
    assert!(
        tokio::time::timeout(Duration::from_millis(120), rx.recv())
            .await
            .is_err(),
        "wait=0 时挂起期不得出残句（watchdog 未启用）"
    );
    // 流结束后 producer 的 flush 倒出整段（半句+尾巴拼接成一句）
    let got = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("流结束后应有 flush 残句")
        .expect("通道不应关闭");
    assert_eq!(got, "思考中尚未成句的半句结束");
    // 之后通道关闭（producer 收工，无更多句子）
    assert!(rx.recv().await.is_none());
    // 收工句也算 sentence_ready（flush 路径的 mark）
    let _ = producer.result.await.expect("任务不 panic");
}

/// **wait=50ms 出残句**：同一挂起流，watchdog 在静默 50ms 后把半句送 TTS；
/// 后续尾巴（收工时 flush）另行到达——两句按时间序出。
#[tokio::test]
async fn short_wait_flushes_residual_during_pause() {
    let llm: crate::plugins::llm::Llm = Arc::new(HangingLlm {
        prefix: "迟迟没有标点的长思考",
        hold: Duration::from_millis(300),
        tail: "然后是结尾",
        punct: true,
    });
    let producer = spawn_llm_producer(
        llm,
        Vec::new(),
        "用户问句".into(),
        prompt(),
        abort(),
        turn_ctx(trace()),
        Duration::from_millis(50),
    );
    let mut rx = producer.sentences;
    // 第一句：watchdog 在挂起期（约 50ms 后、300ms 收工前）送出的残句
    let first = tokio::time::timeout(Duration::from_millis(200), rx.recv())
        .await
        .expect("50ms 静默后 watchdog 应出残句（先于流结束）")
        .expect("通道不应关闭");
    assert_eq!(first, "迟迟没有标点的长思考");
    // 第二句：收工 flush 出的尾巴（带标点）
    let second = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("流结束后应 flush 尾巴")
        .expect("通道不应关闭");
    assert_eq!(second, "然后是结尾。");
    assert!(rx.recv().await.is_none(), "两句之后通道关闭");
    let _ = producer.result.await;
}

/// **阻塞 2 回归**：watchdog 出的残句 mark `sentence_ready`。
/// producer 收工后快照里该阶段必须非 None（否则思考型停顿轮分位数失真）。
#[tokio::test]
async fn watchdog_residual_marks_sentence_ready() {
    let llm: crate::plugins::llm::Llm = Arc::new(HangingLlm {
        prefix: "没有标点的首句在停顿后被送出",
        hold: Duration::from_millis(400),
        tail: "",
        punct: false,
    });
    let t = trace();
    let producer = spawn_llm_producer(
        llm,
        Vec::new(),
        "用户问句".into(),
        prompt(),
        abort(),
        turn_ctx(t.clone()),
        Duration::from_millis(60),
    );
    let mut rx = producer.sentences;
    let first = tokio::time::timeout(Duration::from_millis(250), rx.recv())
        .await
        .expect("停顿后 watchdog 应出残句")
        .expect("通道不应关闭");
    assert_eq!(first, "没有标点的首句在停顿后被送出");
    let _ = producer.result.await;
    let snap = t.snapshot();
    assert!(
        snap.sentence_ready_ms.is_some(),
        "watchdog 残句必须 mark sentence_ready（QA 阻塞 2），实测 None"
    );
    assert!(
        snap.llm_first_token_ms.is_some(),
        "首个 token 也应被 mark（同一条回调链路）"
    );
}

/// 标点出句路径不受 watchdog 影响：首句带句号，立即（不等任何 wait）出句。
#[tokio::test]
async fn punctuated_sentence_still_immediate() {
    let llm: crate::plugins::llm::Llm = Arc::new(HangingLlm {
        prefix: "马上就有标点。",
        hold: Duration::from_millis(80),
        tail: "尾巴",
        punct: false,
    });
    let producer = spawn_llm_producer(
        llm,
        Vec::new(),
        "用户问句".into(),
        prompt(),
        abort(),
        turn_ctx(trace()),
        Duration::from_millis(500), // 远大于挂起 → watchdog 不该抢在标点前出句
    );
    let mut rx = producer.sentences;
    let first = tokio::time::timeout(Duration::from_millis(150), rx.recv())
        .await
        .expect("标点句应立即出（feed 路径）")
        .expect("通道不应关闭");
    assert_eq!(first, "马上就有标点。");
    let second = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("尾巴在流结束 flush")
        .expect("通道不应关闭");
    assert_eq!(second, "尾巴");
    let _ = producer.result.await;
}

/// 打断的假 LLM：吐一段**无标点**的半句 → 停一下 → 让调用方置位 abort（模拟设备打断）
/// → 再吐一段 → 收工。
///
///刻意让打断发生在"半句已累积但尚未成句"的时刻：此时 `splitter` 里躺着残句，
/// 流结束时的 flush **本会**把它推进通道——这正是要验证被取消轮不再外送的那一句。
/// 另外它在收工前递一个工具调用，用来验证**回调拒绝之后连事件都不再递**
/// （引擎侧丢弃 `tool_calls` 的前置条件）。
struct AbortingLlm {
    prefix: &'static str,
    hold: Duration,
    tail: &'static str,
}

impl LlmProvider for AbortingLlm {
    fn chat_stream<'a>(
        &'a self,
        _history: &'a [(String, String)],
        _user_text: &'a str,
        _prompt: &'a TurnPrompt,
        _tools: Option<&'a [ToolSpec]>,
        mut on_event: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmTurnResult>> + Send + 'a>> {
        Box::pin(async move {
            on_event(LlmEvent::Text(self.prefix.to_string()));
            tokio::time::sleep(self.hold).await;
            if !self.tail.is_empty() {
                on_event(LlmEvent::Text(self.tail.to_string()));
            }
            on_event(LlmEvent::ToolCall(ToolCall {
                id: "call_1".into(),
                name: "set_volume".into(),
                arguments: "{\"level\":80}".into(),
            }));
            on_event(LlmEvent::Done);
            Ok(LlmTurnResult::default())
        })
    }
}

/// **打断回归（集成视角）**：abort 置位 → 回调返 `false` → 句子**不再进入通道**。
///
/// 覆盖两条外送路径：
/// 1. 打断后的增量文本（回调首行就查 abort，直接拒收）；
/// 2. 流结束时splitter 里的**残句 flush** —— 这条以前是无条件外送的，等于打断后
///    还在为已被否决的这一轮做 TTS 合成，并把 `sentence_ready` 记成一个并不存在的时刻。
#[tokio::test]
async fn abort_stops_sentences_from_entering_channel() {
    let llm: crate::plugins::llm::Llm = Arc::new(AbortingLlm {
        prefix: "打断前累积的半句",
        hold: Duration::from_millis(150),
        tail: "打断后的内容",
    });
    let flag = abort();
    // 打断时刻由假 LLM 的 `hold` 结束那一刻决定：这里让 abort 在 hold 期间由测试置位。
    let aborter = flag.clone();
    let trigger = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(80)).await;
        aborter.store(true, std::sync::atomic::Ordering::Release);
    });
    let trace = trace();
    let producer = spawn_llm_producer(
        llm,
        Vec::new(),
        "用户问句".into(),
        prompt(),
        flag,
        turn_ctx(trace.clone()),
        Duration::ZERO, // 关掉 watchdog：只考察回调与 flush 两条路径
    );
    let mut rx = producer.sentences;
    // 打断后 producer 不再推句：通道应收工关闭，且**没有**任何句子
    let leaked = tokio::time::timeout(Duration::from_millis(400), rx.recv())
        .await
        .expect("打断后不应再有句子进入通道（含流结束残句 flush）");
    assert!(
        leaked.is_none(),
        "打断后泄漏了一句：{leaked:?}——已被用户否决的这一轮不该继续占用 TTS 合成"
    );
    // 假 LLM 仍会递工具调用；provider 层必须把它当作"回调已拒绝"而不当成有效意图。
    // 真实 provider 会把 tool_calls 丢空（见plugins/llm/openai.rs 的取消契约），
    // 这里断言的是 producer 不依赖它——通道与观测都干净即可。
    let _ = producer
        .result
        .await
        .expect("任务不 panic")
        .expect("取消不是错误");
    trigger.await.expect("打断触发任务不 panic");
    // 取消轮的观测语义：不应把`sentence_ready` 记成一个并不存在的时刻
    let snap = trace.snapshot();
    assert!(
        snap.sentence_ready_ms.is_none(),
        "取消轮不该mark sentence_ready（半句从未真正就绪），实测 {:?}",
        snap.sentence_ready_ms
    );
}

// ───────────────────────── 轮次级可归因：日志带 turn= ─────────────────────────

/// 捕获到的一条日志：只取本测试关心的消息文本与 `turn` 结构化字段。
#[derive(Clone, Debug)]
struct CapturedLog {
    message: String,
    turn: Option<u64>,
}

/// 把 `tracing` 事件字段抄进 `Vec` 的极简订阅器。
///
/// 为什么自己写而不引测试工具：项目**零新增依赖**是硬约束（不引入新观测栈），
/// 而 `tracing` 的 [`tracing::Subscriber`] trait 本身足够实现这个断言。
struct CaptureSubscriber(Arc<Mutex<Vec<CapturedLog>>>);

impl CaptureSubscriber {
    fn new() -> (Self, Arc<Mutex<Vec<CapturedLog>>>) {
        let sink: Arc<Mutex<Vec<CapturedLog>>> = Arc::new(Mutex::new(Vec::new()));
        (Self(Arc::clone(&sink)), sink)
    }
}

/// 字段访问器：`message` 收文本、`turn` 收 u64（`turn = 7` 走 `record_u64`）。
struct TurnFieldVisitor<'a> {
    log: &'a mut CapturedLog,
}

impl tracing::field::Visit for TurnFieldVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.log.message = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.log.message = value.to_string();
        }
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        if field.name() == "turn" {
            self.log.turn = Some(value);
        }
    }
}

impl tracing::Subscriber for CaptureSubscriber {
    fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut log = CapturedLog { message: String::new(), turn: None };
        event.record(&mut TurnFieldVisitor { log: &mut log });
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(log);
    }

    fn enter(&self, _span: &tracing::span::Id) {}

    fn exit(&self, _span: &tracing::span::Id) {}
}

/// 单线程 runtime + **在闭包内 `block_on`** 跑完整一轮，把捕获订阅器装在当前线程上。
///
/// 两个细节缺一不可：
/// - `tracing::subscriber::with_default` 只在**闭包同步执行期间**生效，因此 future
///   必须在闭包里被真正 poll（`block_on`），不能把 future 返回出来再 await；
/// - 必须用 current-thread runtime：producer 的回调/flush 跑在 `tokio::spawn` 出的
///   任务里，多线程 runtime 会把它们调度到别的线程，而线程局部订阅器在那里不可见。
fn capture_turn_logs<F, Fut>(run_turn: F) -> Vec<CapturedLog>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<Vec<String>>>,
{
    let (subscriber, sink) = CaptureSubscriber::new();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime 可构建");
    let outcome = tracing::subscriber::with_default(subscriber, || rt.block_on(run_turn()));
    outcome.expect("本轮 producer 正常收工");
    let logs = sink.lock().unwrap_or_else(|e| e.into_inner()).clone();
    logs
}

/// 跑完一整轮 producer 并收齐全部句子（通道关闭 = 任务体已跑完，收尾日志已打出）。
async fn drain_one_turn(
    llm: crate::plugins::llm::Llm,
    trace: Arc<TurnTrace>,
    flush_wait: Duration,
) -> anyhow::Result<Vec<String>> {
    let producer = spawn_llm_producer(
        llm,
        Vec::new(),
        "用户问句".into(),
        prompt(),
        abort(),
        turn_ctx(trace),
        flush_wait,
    );
    let mut rx = producer.sentences;
    let mut sentences = Vec::new();
    while let Ok(Some(s)) = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
        sentences.push(s);
    }
    producer.result.await??;
    Ok(sentences)
}

/// **一轮的关键节点日志都带同一个 `turn_id`**。
///
/// 跑一整轮 producer（首 token → 首句就绪 → 收工残句），断言这些日志的 `turn`
/// 结构化字段都等于该 [`TurnTrace`] 的 `turn_id`。这是「`grep turn=3` 能捞出整轮
/// 日志行」的可执行证据：字段是结构化字段而非拼进消息的字符串前缀，因此按 `turn=3`
/// 过滤不会被消息文案里的相似文本误命中，也不会因文案改写而失效。
#[test]
fn all_turn_logs_carry_the_same_turn_id() {
    let trace = trace();
    let expected_turn = trace.turn_id();

    let logs = capture_turn_logs(|| {
        drain_one_turn(
            Arc::new(HangingLlm {
                prefix: "第一句有标点。",
                hold: Duration::from_millis(20),
                tail: "尾巴",
                punct: true,
            }),
            trace,
            // 远大于 hold：不走 watchdog 路径，只验回调出句 + 收工残句两条日志
            Duration::from_millis(500),
        )
    });

    let messages: Vec<&str> = logs.iter().map(|l| l.message.as_str()).collect();
    // 两条关键节点日志都必须真实发出（否则下面的逐条断言会空转通过）
    assert!(
        messages.iter().any(|m| m.contains("LLM 首 token")),
        "应记录首 token 日志，实际捕获：{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("首句就绪")),
        "应记录首句就绪日志，实际捕获：{messages:?}"
    );
    for l in &logs {
        assert_eq!(
            l.turn,
            Some(expected_turn),
            "日志「{}」的 turn 字段应为 {expected_turn}（一轮内所有日志同一个轮次号），实测 {:?}",
            l.message,
            l.turn
        );
    }
}

/// 反向断言：轮次内**不得**出现缺 `turn` 字段的日志——那类行会被 `grep turn=N` 漏掉，
/// 正是本任务要消灭的"日志带了 session 但不带轮次"的形态。
#[test]
fn no_turn_log_line_is_missing_the_turn_field() {
    let logs = capture_turn_logs(|| {
        drain_one_turn(
            Arc::new(HangingLlm {
                prefix: "有标点的首句。",
                hold: Duration::from_millis(10),
                tail: "",
                punct: false,
            }),
            trace(),
            Duration::ZERO,
        )
    });

    assert!(!logs.is_empty(), "本测试应至少捕获到一条轮次内日志（否则断言空转）");
    for l in &logs {
        assert!(
            l.turn.is_some(),
            "轮次内的日志「{}」缺少 turn 字段——grep turn=N 会漏掉它",
            l.message
        );
    }
}
