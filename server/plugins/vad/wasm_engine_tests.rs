//! `WasmVadPolicy` 测试：策略只改 native VAD 构造参数，不接收 PCM。

use super::wasm_engine::WasmVadPolicy;
use crate::plugins::vad::VadConfig;
use std::sync::Arc;

#[test]
fn missing_provider_is_explicit_error() {
    let fetcher = Arc::new(crate::plugins::wasm::tests::ScriptedFetcher::new());
    let host = Arc::new(crate::plugins::wasm::tests::host_with(&fetcher));
    let policy = WasmVadPolicy::with_host(host, "vad.provider");
    let err = policy.prepare(&VadConfig::default()).unwrap_err();
    assert!(err.to_string().contains("插件未装载"), "{err:#}");
}

#[test]
fn fixture_rewrites_native_controls_without_pcm() {
    let fetcher = Arc::new(crate::plugins::wasm::tests::ScriptedFetcher::new());
    let host = Arc::new(crate::plugins::wasm::tests::host_with(&fetcher));
    host.load(
        "hello.official",
        &crate::plugins::wasm::tests::ensure_fixture("hello"),
    )
    .expect("load");
    let policy = WasmVadPolicy::with_host(host, "vad.provider");
    let cfg = policy.prepare(&VadConfig::default()).expect("prepare");
    assert_eq!(cfg.threshold, 0.65);
    assert_eq!(cfg.min_silence_duration, 0.4);
    assert_eq!(cfg.min_speech_duration, 0.3);
}
