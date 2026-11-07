//! `server/plugins/llm/fallback.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件）。
//! 主用按生产组装包 [`RetryingLlm`]（重试预算耗尽才轮到备用），
//! 备用为裸 provider（与 `build_llm` 一致）。

use super::*;
use crate::plugins::llm::failure::MAX_RETRIES;
use crate::plugins::llm::RetryingLlm;
use std::sync::atomic::AtomicU32;

/// 可编程假 provider：`fail` 为 `None` 时成功，否则按给定分类失败并记录调用次数。
struct FakeLlm {
    calls: AtomicU32,
    fail: Option<LlmFailure>,
}

impl FakeLlm {
    fn ok() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicU32::new(0),
            fail: None,
        })
    }
    fn failing(f: LlmFailure) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicU32::new(0),
            fail: Some(f),
        })
    }
    fn count(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
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
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(f) = &self.fail {
                return Err(anyhow::Error::new(f.clone()));
            }
            // 取消检测（与 openai.rs 同一套纪律）
            let cancelled = !on_event(LlmEvent::Text("备用回答".into()));
            if cancelled {
                return Ok(LlmTurnResult {
                    text: "备用回答".into(),
                    cancelled: true,
                    ..LlmTurnResult::default()
                });
            }
            on_event(LlmEvent::Done);
            Ok(LlmTurnResult {
                text: "备用回答".into(),
                ..LlmTurnResult::default()
            })
        })
    }
}

/// 与 `build_llm` 相同的组装：主用包 `RetryingLlm`，备用裸挂。
fn build(
    primary: &Arc<FakeLlm>,
    fallbacks: Vec<Arc<FakeLlm>>,
    min_switch_reset: Duration,
) -> FallbackLlm {
    let mut chain: Vec<Llm> = vec![Arc::new(RetryingLlm::new(primary.clone()))];
    chain.extend(fallbacks.into_iter().map(|f| Arc::clone(&f) as Llm));
    FallbackLlm::new(chain, min_switch_reset)
}

async fn run(llm: &FallbackLlm) -> (Result<LlmTurnResult>, String) {
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
    (out, text)
}

/// 主用持续 5xx：重试预算耗尽后切备用，备用成功交付；备用不被重复消费。
#[tokio::test]
async fn switches_to_fallback_after_retry_budget_exhausted() {
    let primary = FakeLlm::failing(LlmFailure::Status {
        code: 503,
        body: "busy".into(),
        retry_after: None,
    });
    let backup = FakeLlm::ok();
    let llm = build(&primary, vec![backup.clone()], Duration::ZERO);
    let (out, text) = run(&llm).await;
    assert!(out.is_ok());
    assert_eq!(text, "备用回答");
    assert_eq!(primary.count(), MAX_RETRIES + 1, "主用应先耗尽重试预算");
    assert_eq!(backup.count(), 1);
}

/// 参数类 4xx（404 model 不存在）换备用救不了：不切换。
#[tokio::test]
async fn non_switchable_error_stays_on_primary() {
    let primary = FakeLlm::failing(LlmFailure::Status {
        code: 404,
        body: "no such model".into(),
        retry_after: None,
    });
    let backup = FakeLlm::ok();
    let llm = build(&primary, vec![backup.clone()], Duration::ZERO);
    let (out, _) = run(&llm).await;
    assert!(out.is_err());
    assert_eq!(backup.count(), 0, "404 不应触发切换");
}

/// 鉴权失败（401）立即切备用：重试主用不会好，主用只撞一次。
#[tokio::test]
async fn auth_failure_switches_immediately() {
    let primary = FakeLlm::failing(LlmFailure::Status {
        code: 401,
        body: "bad key".into(),
        retry_after: None,
    });
    let backup = FakeLlm::ok();
    let llm = build(&primary, vec![backup.clone()], Duration::ZERO);
    let (out, text) = run(&llm).await;
    assert!(out.is_ok());
    assert_eq!(text, "备用回答");
    assert_eq!(primary.count(), 1, "401 不应重试主用");
    assert_eq!(backup.count(), 1);
}

/// 限流声明了恢复时刻：主用冷藏到该时刻，下一轮直接用备用（不再撞主用）。
#[tokio::test]
async fn bench_until_retry_after_skips_primary_next_turn() {
    let primary = FakeLlm::failing(LlmFailure::Status {
        code: 429,
        body: "quota".into(),
        retry_after: Some(3600),
    });
    let backup = FakeLlm::ok();
    let llm = build(&primary, vec![backup.clone()], Duration::ZERO);

    let (out1, _) = run(&llm).await;
    assert!(out1.is_ok());
    // 429 可重试 → 主用先耗尽重试预算（MAX_RETRIES+1 次）才切备用
    assert_eq!(primary.count(), MAX_RETRIES + 1);
    assert_eq!(backup.count(), 1);

    let (out2, _) = run(&llm).await;
    assert!(out2.is_ok());
    assert_eq!(primary.count(), MAX_RETRIES + 1, "冷藏期内不应再撞主用");
    assert_eq!(backup.count(), 2);
}

/// 瞬时失败（5xx）**不**冷藏：下一轮先回主用探测（hermes turn-scoped 语义）。
#[tokio::test]
async fn transient_failure_is_not_benched() {
    let primary = FakeLlm::failing(LlmFailure::Status {
        code: 503,
        body: "busy".into(),
        retry_after: None,
    });
    let backup = FakeLlm::ok();
    let llm = build(&primary, vec![backup.clone()], Duration::ZERO);

    let _ = run(&llm).await;
    let after_turn1 = primary.count();
    let _ = run(&llm).await;
    assert!(
        primary.count() > after_turn1,
        "下一轮必须先回主用探测（5xx 是瞬时失败）"
    );
}

/// `min_switch_reset`：限流即将恢复（窗口更短）时不切备用，本轮失败走会话兜底。
#[tokio::test]
async fn min_switch_reset_waits_out_short_window() {
    let primary = FakeLlm::failing(LlmFailure::Status {
        code: 429,
        body: "quota".into(),
        retry_after: Some(60),
    });
    let backup = FakeLlm::ok();
    let llm = build(&primary, vec![backup.clone()], Duration::from_secs(120));
    let (out, _) = run(&llm).await;
    assert!(out.is_err(), "窗口短于 min_switch_reset 应放弃本轮");
    assert_eq!(backup.count(), 0, "不应切备用");
}

/// 已向调用方交付文本后再失败：绝不切备用（换个声音把前半句重说一遍 = 重复播报）。
#[tokio::test]
async fn no_switch_after_text_delivered() {
    /// 先吐文本再失败的假 provider。
    struct EmitThenFail {
        calls: AtomicU32,
    }
    impl LlmProvider for EmitThenFail {
        fn chat_stream<'a>(
            &'a self,
            _history: &'a [(String, String)],
            _user_text: &'a str,
            _prompt: &'a TurnPrompt,
            _tools: Option<&'a [ToolSpec]>,
            mut on_event: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
        ) -> Pin<Box<dyn Future<Output = Result<LlmTurnResult>> + Send + 'a>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                on_event(LlmEvent::Text("前半句".into()));
                Err(anyhow::Error::new(LlmFailure::Status {
                    code: 503,
                    body: "busy".into(),
                    retry_after: None,
                }))
            })
        }
    }

    let primary = Arc::new(EmitThenFail {
        calls: AtomicU32::new(0),
    });
    let backup = FakeLlm::ok();
    let mut chain: Vec<Llm> = vec![Arc::new(RetryingLlm::new(primary.clone()))];
    chain.push(backup.clone());
    let llm = FallbackLlm::new(chain, Duration::ZERO);
    let (out, text) = run(&llm).await;
    assert!(out.is_err());
    assert_eq!(text, "前半句", "已交付的文本原样保留");
    assert_eq!(backup.count(), 0, "已出声不得切备用");
}

/// 取消轮透传为 `Ok{cancelled}`，不触发切换（重试一条被放弃的请求 = 白烧 token）。
#[tokio::test]
async fn cancelled_turn_passes_through() {
    // 主用第一次调用即被取消：回调恒返 false。
    struct CancelAlways {
        calls: AtomicU32,
    }
    impl LlmProvider for CancelAlways {
        fn chat_stream<'a>(
            &'a self,
            _history: &'a [(String, String)],
            _user_text: &'a str,
            _prompt: &'a TurnPrompt,
            _tools: Option<&'a [ToolSpec]>,
            mut on_event: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
        ) -> Pin<Box<dyn Future<Output = Result<LlmTurnResult>> + Send + 'a>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                let cancelled = !on_event(LlmEvent::Text("半句".into()));
                Ok(LlmTurnResult {
                    text: "半句".into(),
                    cancelled,
                    ..LlmTurnResult::default()
                })
            })
        }
    }

    let primary = Arc::new(CancelAlways {
        calls: AtomicU32::new(0),
    });
    let backup = FakeLlm::ok();
    let mut chain: Vec<Llm> = vec![Arc::new(RetryingLlm::new(primary.clone()))];
    chain.push(backup.clone());
    let llm = FallbackLlm::new(chain, Duration::ZERO);
    let history: Vec<(String, String)> = Vec::new();
    let out = llm
        .chat_stream(
            &history,
            "你好",
            &TurnPrompt::default(),
            None,
            Box::new(|_| false),
        )
        .await
        .expect("取消不是错误");
    assert!(out.cancelled);
    assert_eq!(backup.count(), 0, "取消轮不得切备用");
}

/// 主用与全部备用都失败：返回最后的错误（会话侧兜底话术接手）。
#[tokio::test]
async fn all_failures_return_error() {
    let primary = FakeLlm::failing(LlmFailure::Status {
        code: 503,
        body: "busy".into(),
        retry_after: None,
    });
    let backup1 = FakeLlm::failing(LlmFailure::Connect("dns".into()));
    let backup2 = FakeLlm::failing(LlmFailure::EmptyResponse);
    let llm = build(
        &primary,
        vec![backup1.clone(), backup2.clone()],
        Duration::ZERO,
    );
    let (out, _) = run(&llm).await;
    let err = out.expect_err("全链失败应返回错误");
    assert!(
        matches!(LlmFailure::classify(&err), LlmFailure::EmptyResponse),
        "应返回最后一个备用的错误"
    );
    assert_eq!(backup1.count(), 1);
    assert_eq!(backup2.count(), 1);
}
