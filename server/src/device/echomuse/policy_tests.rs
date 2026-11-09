//! `device.echomuse` 会话入口策略回归：欢迎语放行回环与缺插件 fail-closed。
//! 夹具 = hello.official（声明 device.echomuse 的探针）。

use super::policy::echomuse_prepare;
use crate::plugins::wasm::tests::{ensure_fixture, host_with, ScriptedFetcher};
use std::sync::Arc;

#[test]
fn echomuse_prepare_roundtrip_keeps_native_route() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load("hello.official", &ensure_fixture("hello"))
        .expect("load hello");
    let welcome = echomuse_prepare(&host, "echo-1", "emos", "你好")
        .expect("echomuse prepare")
        .expect("fixture preserves welcome");
    assert_eq!(welcome, "你好");
}

#[test]
fn echomuse_prepare_fails_closed_without_plugin() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let err = echomuse_prepare(&host, "echo-1", "emos", "你好")
        .expect_err("missing EchoMuse strategy must fail closed");
    assert!(err.contains("无可用实现"), "{err}");
}
