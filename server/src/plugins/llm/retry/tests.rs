//! `server/src/plugins/llm/retry.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::*;
use crate::plugins::llm::failure::TokenUsage;
use std::sync::atomic::AtomicU32;

/// 可编程的假 provider：按脚本失败/成功，并记录被调用次数。
///
/// 遵守 `LlmTurnResult` 的**取消契约**：回调返 `false` 即整轮取消——工具调用丢空、
/// `cancelled = true`、不发 `Done`。这条纪律让本文件能测"取消不重试"而不必
/// 断言实现细节。
struct FakeLlm {
    calls: AtomicU32,
    /// 前 N 次调用失败（503，可重试）。
    fail_first: u32,
    /// 失败前是否先吐一段文本（模拟"已出声"）。
    emit_before_fail: bool,
    /// 用不可重试的错误（401）失败。
    unauthorized: bool,
}

impl FakeLlm {
    fn new(fail_first: u32) -> Self {
        Self {
            calls: AtomicU32::new(0),
            fail_first,
            emit_before_fail: false,
            unauthorized: false,
        }
    }
}

impl LlmProvider for FakeLlm {
    fn chat_stream<'a>(
        &'a self,
        _history: &'a [(String, String)],
        _user_text: &'a str,
        _prompt: &'a TurnPrompt,
        _tools: Option<&'a [ToolSpec]>,
        mut on_event: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmTurnResult>> + Send + 'a>> {
        Box::pin(async move {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if n <= self.fail_first {
                if self.emit_before_fail {
                    on_event(LlmEvent::Text("前半句".into()));
                }
                let f = if self.unauthorized {
                    LlmFailure::Status {
                        code: 401,
                        body: "bad key".into(),
                    }
                } else {
                    LlmFailure::Status {
                        code: 503,
                        body: "upstream busy".into(),
                    }
                };
                return Err(anyhow::Error::new(f));
            }
            // 取消检测（与 openai.rs 同一套纪律）：任一事件被拒绝即整轮取消
            let cancelled = !on_event(LlmEvent::Text("最终回答".into()));
            if cancelled {
                return Ok(LlmTurnResult {
                    text: "最终回答".into(),
                    tool_calls: Vec::new(),
                    usage: Some(TokenUsage::default()),
                    cancelled: true,
                });
            }
            on_event(LlmEvent::Done);
            Ok(LlmTurnResult {
                text: "最终回答".into(),
                tool_calls: Vec::new(),
                usage: Some(TokenUsage::default()),
                cancelled: false,
            })
        })
    }
}

/// `on_event` 一律拒绝（= 立刻取消），用于测取消路径不被重试。
async fn run_cancelling(inner: Arc<FakeLlm>) -> (Result<LlmTurnResult>, u32) {
    let llm: Llm = Arc::new(RetryingLlm::new(inner.clone()));
    let history: Vec<(String, String)> = Vec::new();
    let out = llm
        .chat_stream(
            &history,
            "你好",
            &TurnPrompt::default(),
            None,
            Box::new(|_| false),
        )
        .await;
    (out, inner.calls.load(Ordering::SeqCst))
}

async fn run(inner: Arc<FakeLlm>) -> (Result<LlmTurnResult>, u32, String) {
    let llm: Llm = Arc::new(RetryingLlm::new(inner.clone()));
    let collected = Arc::new(Mutex::new(String::new()));
    let sink = collected.clone();
    let history: Vec<(String, String)> = Vec::new();
    let out = llm
        .chat_stream(
            &history,
            "你好",
            &TurnPrompt::default(),
            None,
            Box::new(move |e| {
                if let LlmEvent::Text(t) = e {
                    sink.lock().unwrap_or_else(|e| e.into_inner()).push_str(&t);
                }
                true
            }),
        )
        .await;
    let text = collected.lock().unwrap_or_else(|e| e.into_inner()).clone();
    (out, inner.calls.load(Ordering::SeqCst), text)
}

/// 瞬时 5xx 应被重试，最终成功；且文本不重复交付。
#[tokio::test]
async fn retries_transient_then_succeeds() {
    let fake = Arc::new(FakeLlm::new(2)); // 前两次 503，第三次成功
    let (out, calls, text) = run(fake).await;
    assert_eq!(calls, 3, "应重试两次后成功");
    assert!(out.is_ok());
    assert_eq!(text, "最终回答", "重试成功后只交付一次文本");
}

/// 已吐出文本后再失败 → **不重试**（用户已听到前半句，重来会重复播报）。
#[tokio::test]
async fn does_not_retry_after_text_delivered() {
    let mut fake = FakeLlm::new(u32::MAX); // 永远失败
    fake.emit_before_fail = true;
    let fake = Arc::new(fake);
    let (out, calls, text) = run(fake).await;
    assert!(out.is_err());
    assert_eq!(calls, 1, "已交付文本 → 不得重试");
    assert_eq!(text, "前半句");
}

/// 4xx（除 429）不可重试：重试只会白等。
#[tokio::test]
async fn does_not_retry_client_errors() {
    let mut fake = FakeLlm::new(u32::MAX);
    fake.unauthorized = true;
    let fake = Arc::new(fake);
    let (out, calls, _) = run(fake).await;
    assert!(out.is_err());
    assert_eq!(calls, 1, "401 不应重试");
}

/// 重试次数有上界：永远失败时恰好 MAX_RETRIES+1 次尝试（不会无限打上游）。
#[tokio::test]
async fn retries_are_bounded() {
    let fake = Arc::new(FakeLlm::new(u32::MAX));
    let (out, calls, _) = run(fake).await;
    assert!(out.is_err());
    assert_eq!(calls, MAX_RETRIES + 1);
}

/// **取消绝不重试**：被调用方取消的一轮（`cancelled = true`，见 `LlmTurnResult` 取消契约）
/// 必须原样透传、且只调用 provider **一次**。
///
/// 为什么这条值得钉死：用户已经闭嘴了，重试这条被放弃的请求＝白烧 token + 白等 400ms 起退避。
/// 该不变量由"取消表达为 `Ok{.., cancelled}` 而非 `LlmFailure`"保证（`Ok` 分支直接 return），
/// 本测试防止将来有人"顺手"把它改成失败分类、或给 `Ok` 分支加上重试逻辑。
#[tokio::test]
async fn cancelled_turn_is_not_retried() {
    let fake = Arc::new(FakeLlm::new(0)); // 首次即成功：取消发生在成功轮里，任何重试都会体现为 calls > 1
    let (out, calls) = run_cancelling(fake).await;
    let out = out.expect("取消不是错误，应透传为 Ok");
    assert!(out.cancelled, "取消标记必须原样透传给下游");
    assert!(out.tool_calls.is_empty(), "取消轮不得携带工具调用");
    assert_eq!(calls, 1, "取消轮不得重试（会白烧 token）");
}
