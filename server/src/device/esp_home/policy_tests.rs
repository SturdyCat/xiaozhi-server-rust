//! `connector.esp-home` 实体目录策略回归：放行回环与缺插件 fail-closed。
//! 夹具 = hello.official（声明 connector.esp-home 的探针）。

use super::policy::esp_home_catalog;
use super::state::CatalogPlan;
use crate::plugins::wasm::tests::{ensure_fixture, host_with, ScriptedFetcher};
use std::sync::Arc;

#[test]
fn esp_home_catalog_roundtrip_is_full_passthrough() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load hello");
    let plan = esp_home_catalog(&host).expect("esp_home catalog");
    // 探针夹具是全放行 + 不改名：计划必须与 native 直通完全一致（行为不漂移）。
    assert_eq!(plan, CatalogPlan::passthrough());
}

#[test]
fn esp_home_catalog_fails_closed_without_plugin() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let err = esp_home_catalog(&host).expect_err("missing catalog policy must fail closed");
    assert!(err.contains("无可用实现"), "{err}");
}
