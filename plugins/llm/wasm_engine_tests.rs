//! `WasmLlmPolicy` 的控制面回归：策略插件只改请求元数据，native provider 保留流式/取消契约。

use super::wasm_engine::WasmLlmPolicy;
use super::{LlmEvent, LlmProvider, LlmTurnResult, ToolSpec};
use crate::plugins::prompt::TurnPrompt;
use anyhow::Result;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

struct FakeNative;

impl LlmProvider for FakeNative {
    fn chat_stream<'a>(
        &'a self,
        _history: &'a [(String, String)],
        user_text: &'a str,
        _prompt: &'a TurnPrompt,
        _tools: Option<&'a [ToolSpec]>,
        mut on_event: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmTurnResult>> + Send + 'a>> {
        let text = user_text.to_string();
        Box::pin(async move {
            let accepted = on_event(LlmEvent::Text(text.clone()));
            if !accepted {
                return Ok(LlmTurnResult {
                    text,
                    cancelled: true,
                    ..Default::default()
                });
            }
            let _ = on_event(LlmEvent::Done);
            Ok(LlmTurnResult {
                text,
                ..Default::default()
            })
        })
    }
}

fn host_with_hello() -> Arc<crate::plugins::wasm::WasmHost> {
    let fetcher = Arc::new(crate::plugins::wasm::tests::ScriptedFetcher::new());
    let host = Arc::new(crate::plugins::wasm::tests::host_with(&fetcher));
    host.load(
        "hello.official",
        &crate::plugins::wasm::tests::ensure_fixture("hello"),
    )
    .expect("hello fixture should load");
    host
}

#[test]
fn policy_preserves_native_stream_after_wasm_prepare() {
    let host = host_with_hello();
    let policy = WasmLlmPolicy::with_host(Arc::new(FakeNative), host, "llm.provider");
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let result = runtime.block_on(policy.chat_stream(
        &[("user".into(), "previous".into())],
        "hello",
        &TurnPrompt::default(),
        None,
        Box::new(|event| matches!(event, LlmEvent::Text(_) | LlmEvent::Done)),
    ));
    let result = result.expect("strategy + native provider");
    assert_eq!(result.text, "hello");
    assert!(!result.cancelled);
}

#[test]
fn policy_missing_provider_is_explicit_error() {
    let fetcher = Arc::new(crate::plugins::wasm::tests::ScriptedFetcher::new());
    let host = Arc::new(crate::plugins::wasm::tests::host_with(&fetcher));
    let policy = WasmLlmPolicy::with_host(Arc::new(FakeNative), host, "missing.provider");
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let err = runtime
        .block_on(policy.chat_stream(
            &[],
            "hello",
            &TurnPrompt::default(),
            None,
            Box::new(|_| true),
        ))
        .expect_err("missing strategy must fail closed");
    assert!(err.to_string().contains("策略插件未装载"), "{err:#}");
}

#[test]
fn policy_passes_cancellation_to_native_provider() {
    let host = host_with_hello();
    let policy = WasmLlmPolicy::with_host(Arc::new(FakeNative), host, "llm.provider");
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let result = runtime.block_on(policy.chat_stream(
        &[],
        "cancel me",
        &TurnPrompt::default(),
        None,
        Box::new(|_| false),
    ));
    assert!(result.expect("native cancellation").cancelled);
}
