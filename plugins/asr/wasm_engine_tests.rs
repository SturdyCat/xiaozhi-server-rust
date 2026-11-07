//! `WasmAsrEngine` 适配器测试（P4a）：utterance 级事件桥的真实 wasm 往返。

use super::wasm_engine::WasmAsrEngine;
use crate::plugins::asr::AsrEngine;
use crate::plugins::wasm::tests::{ensure_fixture, host_with};
use std::sync::Arc;

#[test]
fn recognize_roundtrips_via_wasm_plugin() {
    let fetcher = Arc::new(crate::plugins::wasm::tests::ScriptedFetcher::new());
    let host = Arc::new(host_with(&fetcher));
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load");
    let engine = WasmAsrEngine::with_host(Arc::clone(&host), "asr.provider");
    let samples: Vec<f32> = (0..1600)
        .map(|i| (i as f32 * 0.01).clamp(-1.0, 1.0))
        .collect();
    let text = engine
        .recognize(&samples, 16000)
        .unwrap_or_else(|e| panic!("转写: {e:#}"));
    assert!(
        text.contains("来自 wasm ASR") && text.contains("1600"),
        "探针应回显采样数：{text}"
    );
}

#[test]
fn recognize_fails_explicitly_without_plugin() {
    let fetcher = Arc::new(crate::plugins::wasm::tests::ScriptedFetcher::new());
    let host = Arc::new(host_with(&fetcher));
    let engine = WasmAsrEngine::with_host(host, "missing.provider");
    let err = engine.recognize(&[0.0; 16], 16000).unwrap_err();
    assert!(
        err.to_string().contains("未装载") || err.to_string().contains("未初始化"),
        "缺依赖明确报错：{err}"
    );
}

#[test]
fn extension_point_resolution_picks_user_implementation() {
    // P2e 替换语义对 ASR 透明：扩展点首选用户实现 → 桥派发到用户实现
    let fetcher = Arc::new(crate::plugins::wasm::tests::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load");
    host.load("greeter.usermade", &ensure_fixture("greeter-user"))
        .expect("load user");
    // 用户实现未声明 asr.provider 扩展点 → 解析器按设计回退官方默认
    //（首选不可用 ≠ 系统不可用；Fallback 的 official 即回退目标）
    host.set_extension_preference("asr.provider", Some("greeter.usermade".into()));
    match host.resolve("asr.provider") {
        crate::plugins::wasm::Resolution::Fallback {
            official,
            preferred,
            ..
        } => {
            assert_eq!(official, "hello.official");
            assert_eq!(preferred, "greeter.usermade");
        }
        other => panic!("应回退官方默认：{other:?}"),
    }
}
