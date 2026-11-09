//! `connector.tuya` WASM 控制策略回归：有界候选选择、缺插件 fail-closed、越界拒绝。
//! 夹具 = hello.official（声明 connector.tuya 的探针，保持 Registered 不激活的生产
//! 纪律在单测里以 `load` 直接装载验证决策回环）。

use super::policy::tuya_prepare;
use crate::plugins::wasm::tests::{ensure_fixture, host_with, ScriptedFetcher};
use std::sync::Arc;

#[test]
fn tuya_connector_prepare_roundtrip_uses_bounded_decision() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load hello");
    let candidates = vec![serde_json::json!({
        "index": 0,
        "label": "客厅插座",
        "online": true,
        "model": "outlet",
        "has_switch_dp": true,
    })];
    let result = tuya_prepare(&host, "客厅插座", "off", &candidates).expect("tuya prepare");
    assert_eq!(result.candidate_index, 0);
    assert_eq!(result.action, "off");
    assert_eq!(result.route.as_deref(), Some("native"));
}

#[test]
fn tuya_connector_prepare_fails_closed_without_plugin() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let err = tuya_prepare(
        &host,
        "客厅插座",
        "on",
        &[serde_json::json!({"index": 0, "label": "客厅插座"})],
    )
    .expect_err("missing connector must fail closed");
    assert!(err.contains("无可用实现"), "{err}");
}

#[test]
fn tuya_connector_prepare_rejects_out_of_range_decision() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load hello");
    let err = tuya_prepare(&host, "客厅插座", "on", &[])
        .expect_err("empty candidate list must reject fixture decision");
    assert!(err.contains("超出") || err.contains("越界"), "{err}");
}
