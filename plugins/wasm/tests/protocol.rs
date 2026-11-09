//! 阶段三 S3b 证据：涂鸦云协议语义**单源 guest 对拍**。
//!
//! `server/src/device/tuya/protocol.rs`（官方 v1.0 签名 + 请求构造）经 `#[path]`
//! 被 tuyasign 夹具（tools/wasm-fixtures/tuyasign）原样编译进 guest，在 WASM 内
//! 执行签名并写回 kv。本文件把它与宿主侧黄金向量逐字节对拍：向量与
//! `device::tuya::cloud_tests` 同源（v1=2026-11-03 python 参照实现、v2 覆盖
//! Content-SHA256 段），证明「协议语义在插件侧可执行且与宿主一致」有回归护栏。
//!
//! 边界不变：`access_secret` 永不跨 WASM 边界——夹具只用测试假凭证；生产签名
//! 调用方仍是持有凭证的 native `CloudClient`。

use serde_json::json;

use super::{fast_limits, fixture};
use crate::plugins::wasm::WasmHost;

const PLUGIN: &str = "tuyasign.lab";
const SIGN_TOPIC: &str = "demo.tuya-sign.sign";

fn host() -> WasmHost {
    let host = WasmHost::new(fast_limits()).expect("host");
    host.load(PLUGIN, &fixture("tuyasign"))
        .expect("load tuyasign fixture");
    host
}

fn guest_sign(host: &WasmHost, req_id: &str, body: &[(String, String)]) -> String {
    let payload = json!({
        "req_id": req_id,
        "secret": "asec",
        "access_id": "aid",
        "token": body.iter().find(|(k, _)| k == "token").map(|(_, v)| v.as_str()).unwrap_or_default(),
        "t": body.iter().find(|(k, _)| k == "t").map(|(_, v)| v.as_str()).unwrap_or_default(),
        "method": body.iter().find(|(k, _)| k == "method").map(|(_, v)| v.as_str()).unwrap_or_default(),
        "path_query": body.iter().find(|(k, _)| k == "path_query").map(|(_, v)| v.as_str()).unwrap_or_default(),
        "body": body.iter().find(|(k, _)| k == "body").map(|(_, v)| v.as_str()).unwrap_or_default(),
    })
    .to_string();
    host.handle_event(PLUGIN, SIGN_TOPIC, &payload)
        .expect("guest protocol dispatch");
    let bytes = host
        .kv()
        .get(PLUGIN, &format!("tuyasign-result:{req_id}"))
        .expect("guest sign result");
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("result json");
    value["sign"].as_str().expect("sign string").to_string()
}

#[test]
fn guest_single_source_sign_matches_token_golden() {
    let host = host();
    let sign = guest_sign(
        &host,
        "t1",
        &[
            ("t".into(), "1700000000000".into()),
            ("method".into(), "GET".into()),
            ("path_query".into(), "/v1.0/token?grant_type=1".into()),
            ("body".into(), String::new()),
        ],
    );
    // 与 device::tuya::cloud_tests::official_sign_matches_offline_golden_vector 同源
    assert_eq!(
        sign,
        "42AD3292C57626B9ED5B83615A5F3735CBFA4C9BC9F9F512C8A22A24BDE0C0E2"
    );
}

#[test]
fn guest_single_source_sign_matches_post_with_body_golden() {
    let host = host();
    let sign = guest_sign(
        &host,
        "t2",
        &[
            ("token".into(), "tok".into()),
            ("t".into(), "1700000000100".into()),
            ("method".into(), "POST".into()),
            ("path_query".into(), "/v1.0/devices/ivf9id/dp".into()),
            ("body".into(), r#"{"dp":{"1":true}}"#.into()),
        ],
    );
    // 覆盖签名字符串的 Content-SHA256 段（body 非空路径）；python 参照实现计算
    assert_eq!(
        sign,
        "E0D6A6B659DC7B28BF5FCB5BF85F96B30B6D1FD83197C9DA6ED3A8BD7AB09BA1"
    );
}

#[test]
fn guest_sign_fails_closed_on_malformed_payload() {
    let host = host();
    // 宿主契约：guest 业务性 Err 记日志、不写结果键、不降级（dispatch 早退 Ok）。
    // 调用方（policy seam 同款）以 kv 结果缺失判定失败——这里钉死"缺参 ⇒ 无结果"。
    let errors = host
        .handle_event(PLUGIN, SIGN_TOPIC, r#"{"req_id":"e1"}"#)
        .expect("dispatch completes");
    assert!(errors.is_empty(), "业务性 Err 不产生效果错误：{errors:?}");
    assert!(
        host.kv().get(PLUGIN, "tuyasign-result:e1").is_none(),
        "缺参载荷不得写出签名结果键"
    );
}
