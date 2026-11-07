//! `WasmTtsPolicy` 测试：WASM 只参与控制策略，真实 PCM 仍由 native 引擎负责。

use super::wasm_engine::{TtsPolicyDecision, WasmTtsPolicy};
use crate::plugins::tts::{TtsChunkCallback, TtsEngine};
use anyhow::Result;
use std::sync::Arc;

struct FakeNative;

impl TtsEngine for FakeNative {
    fn synthesize_stream(
        &self,
        text: &str,
        speed: f32,
        speaker: i32,
        mut cb: TtsChunkCallback,
    ) -> Result<()> {
        assert_eq!(text, "native text");
        assert_eq!(speed, 1.25);
        assert_eq!(speaker, 7);
        assert!(cb(24_000, &[0.1, 0.2]));
        Ok(())
    }

    fn output_sample_rate(&self) -> u32 {
        24_000
    }
}

#[test]
fn policy_rewritten_controls_reach_native_engine() {
    let fetcher = Arc::new(crate::plugins::wasm::tests::ScriptedFetcher::new());
    let host = Arc::new(crate::plugins::wasm::tests::host_with(&fetcher));
    let policy = WasmTtsPolicy::with_host(Arc::new(FakeNative), host, "missing.provider");
    let received = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&received);
    policy
        .synthesize_decision(
            TtsPolicyDecision {
                text: "native text".into(),
                speed: 1.25,
                speaker: 7,
                route: Some("native".into()),
            },
            Box::new(move |rate, chunk| {
                sink.lock().unwrap().push((rate, chunk.to_vec()));
                true
            }),
        )
        .expect("策略结果应交给 native 流式合成");
    assert_eq!(*received.lock().unwrap(), vec![(24_000, vec![0.1, 0.2])]);
}

#[test]
fn policy_reports_unloaded_provider_and_keeps_native_output_rate() {
    let fetcher = Arc::new(crate::plugins::wasm::tests::ScriptedFetcher::new());
    let host = Arc::new(crate::plugins::wasm::tests::host_with(&fetcher));
    let policy = WasmTtsPolicy::with_host(Arc::new(FakeNative), host, "missing.provider");
    let err = policy
        .synthesize_stream("input", 1.0, 0, Box::new(|_, _| true))
        .unwrap_err();
    assert!(err.to_string().contains("策略插件未装载"), "{err:#}");
    assert_eq!(policy.output_sample_rate(), 24_000);
}

#[test]
fn policy_requires_provider_even_when_native_engine_exists() {
    let fetcher = Arc::new(crate::plugins::wasm::tests::ScriptedFetcher::new());
    let host = Arc::new(crate::plugins::wasm::tests::host_with(&fetcher));
    let policy = WasmTtsPolicy::with_host(Arc::new(FakeNative), host, "tts.provider");
    let err = policy
        .synthesize_stream("input", 1.0, 0, Box::new(|_, _| true))
        .unwrap_err();
    assert!(
        err.to_string().contains("策略插件未装载")
            || err.to_string().contains("WASM 插件平台未初始化"),
        "显式策略缺插件必须报错：{err:#}"
    );
}
