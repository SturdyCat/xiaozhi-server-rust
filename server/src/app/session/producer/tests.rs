//! `server/src/app/session/producer.rs` 的单元测试。
//!
//! 覆盖缺口③在 producer 层的行为级语义（QA Round 1 建议 4 补测）：
//! ① `flush_wait = 0` 不启用 watchdog——LLM 挂起流（半句后停顿）在窗口期内不出残句；
//! ② `flush_wait = 50ms` 停顿后出残句；
//! ③ watchdog 出的残句同样 mark `sentence_ready`（阻塞 2 修复的回归）；
//! ④ 正常标点出句与流结束 flush 不受 watchdog 影响。
//!
//! 不用实现细节断言（如数任务数），全部断言"通道里有没有句子"这一可观测行为。

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use super::spawn_llm_producer;
use crate::app::events::{TurnTrace, ANCHOR_SPEECH_END};
use crate::plugins::llm::{
    LlmEvent, LlmProvider, LlmTurnResult, ToolSpec,
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
        trace(),
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
        trace(),
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
        t.clone(),
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
        trace(),
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
