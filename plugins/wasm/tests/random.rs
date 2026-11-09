//! S3s：宿主随机源（`host-core.random`）+ guest 全链探针回归。
//!
//! 覆盖三件事：① OS 熵源按长度填充且两次互异；② guest 夹具（hello 探针）经
//! wasm 回来的 `random-probe` 自检全过（含越界拒绝）；③ 宿主侧长度上限与
//! 插件侧调用语义一致（探针同时断言 `random(4096)` 必须被拒绝）。

use super::{ensure_fixture, fast_limits};
use crate::plugins::wasm::host_api::{OsRandom, RandomSource};
use crate::plugins::wasm::WasmHost;

#[test]
fn os_random_fills_requested_len_and_varies() {
    let rng = OsRandom;
    let a = rng.fill(16).expect("OS 随机源应可用");
    let b = rng.fill(16).expect("OS 随机源应可用");
    assert_eq!(a.len(), 16);
    assert_eq!(b.len(), 16);
    assert_ne!(a, b, "两次 16 字节随机数不应相同（概率上可忽略的偶合）");
}

#[test]
fn guest_random_probe_passes_via_validate() {
    let host = WasmHost::new(fast_limits()).expect("host");
    let results = host
        .validate("hello.official", &ensure_fixture("hello"))
        .expect("validate");
    let probe = results
        .iter()
        .find(|r| r.name == "random-probe")
        .unwrap_or_else(|| panic!("自检应含 random-probe：{results:?}"));
    assert!(probe.ok, "random-probe 应通过：{probe:?}");
    assert!(
        probe.detail.contains("越界拒绝=true"),
        "探针应验证越界拒绝：{probe:?}"
    );
}
