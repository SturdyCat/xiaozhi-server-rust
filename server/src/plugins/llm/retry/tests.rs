//! `server/src/plugins/llm/retry.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::*;
use crate::plugins::llm::failure::TokenUsage;
use std::sync::atomic::AtomicU32;

/// 可编程的假 provider：按脚本失败/成功，并记录被调用次数。
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
            on_event(LlmEvent::Text("最终回答".into()));
            on_event(LlmEvent::Done);
            Ok(LlmTurnResult {
                text: "最终回答".into(),
                tool_calls: Vec::new(),
                usage: Some(TokenUsage::default()),
            })
        })
    }
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
