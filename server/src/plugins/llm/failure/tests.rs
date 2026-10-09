//! `server/src/plugins/llm/failure.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::*;

fn status(code: u16) -> LlmFailure {
    LlmFailure::Status {
        code,
        body: String::new(),
    }
}

/// 可重试集合必须与 DSH 一致：瞬时/限流/服务端/超时/空回复可重试，4xx 不重试。
#[test]
fn retryable_covers_transient_only() {
    for f in [
        LlmFailure::Connect("dns".into()),
        LlmFailure::Timeout("idle".into()),
        LlmFailure::Transport("broken".into()),
        LlmFailure::EmptyResponse,
        status(429),
        status(500),
        status(503),
    ] {
        assert!(f.retryable(), "{f:?} 应可重试");
    }
    for f in [
        status(400),
        status(401),
        status(403),
        status(404),
        status(422),
    ] {
        assert!(
            !f.retryable(),
            "{f:?} 不应重试（客户端错误重试只是浪费预算）"
        );
    }
    assert!(!LlmFailure::Protocol("bad json".into()).retryable());
}

#[test]
fn categories_are_stable() {
    assert_eq!(status(429).category(), "rate_limit");
    assert_eq!(status(503).category(), "server");
    assert_eq!(status(401).category(), "client");
    assert_eq!(LlmFailure::EmptyResponse.category(), "empty_response");
    assert_eq!(LlmFailure::Timeout("x".into()).category(), "timeout");
}

/// 退避必须**递增且有界**（避免重试把语音延迟拖长）。
#[test]
fn backoff_grows_and_stays_within_budget() {
    let d1 = retry_delay(1);
    let d2 = retry_delay(2);
    assert!(
        d1 >= RETRY_BASE_DELAY && d1 < RETRY_BASE_DELAY * 2,
        "{d1:?}"
    );
    assert!(d2 >= RETRY_BASE_DELAY * 2, "{d2:?} 应比第一次更久");
    assert!(
        d1 + d2 < RETRY_BUDGET,
        "两轮退避之和必须落在总预算内（{d1:?}+{d2:?}）"
    );
}

/// 用量口径：压力 = 输入（含缓存），**不含 output**；缓存读不重复计入 input。
#[test]
fn usage_pressure_excludes_output() {
    let v = serde_json::json!({
        "input_tokens": 100,
        "output_tokens": 40,
        "total_tokens": 140,
        "input_tokens_details": { "cached_tokens": 60 }
    });
    let u = TokenUsage::from_responses_usage(&v);
    assert_eq!(u.cache_read, 60);
    assert_eq!(u.input, 40, "未命中缓存的输入 = 总输入 - 缓存命中");
    assert_eq!(
        u.pressure(),
        100,
        "压力口径 = input + cache_read（不含 output）"
    );
    assert_eq!(u.total(), 140);
}

/// 缺字段（上游换口径/老网关）不能 panic，退化为 0。
#[test]
fn usage_tolerates_missing_fields() {
    let u = TokenUsage::from_responses_usage(&serde_json::json!({}));
    assert!(u.is_zero());
    let u = TokenUsage::from_responses_usage(&serde_json::json!({"input_tokens": 5}));
    assert_eq!(u.pressure(), 5);
    assert_eq!(u.output, 0);
}

#[test]
fn usage_accumulates() {
    let mut a = TokenUsage {
        input: 10,
        output: 2,
        cache_read: 5,
        cache_write: 1,
    };
    a.add(&TokenUsage {
        input: 3,
        output: 4,
        cache_read: 0,
        cache_write: 2,
    });
    assert_eq!(a.pressure(), 21);
    assert_eq!(a.total(), 27);
}
