use std::path::Path;

use super::wasm::bounded_result;
use super::TuyaEngine;
use crate::plugins::wasm::{SandboxLimits, WasmHost};

#[test]
fn oversized_result_is_replaced_by_bounded_error() {
    let value = serde_json::json!({ "payload": "x".repeat(20_000) });
    let result = bounded_result(value);
    assert!(result.len() < 256);
    assert!(std::str::from_utf8(&result)
        .unwrap()
        .contains("结果超出宿主上限"));
}

#[tokio::test]
async fn native_consumer_routes_missing_device_failure_to_plugin_kv() {
    let host = WasmHost::new(SandboxLimits::default()).expect("host");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wasm/hello.wasm");
    host.load(
        "hello.official",
        &std::fs::read(fixture).expect("hello fixture"),
    )
    .expect("load hello");
    let errors = host
        .handle_event(
            "hello.official",
            "connector.tuya.status",
            r#"{"req_id":"req-1"}"#,
        )
        .expect("dispatch status effect");
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(host.pending_lan_commands(), 1);

    let data_dir =
        std::env::temp_dir().join(format!("xiaozhi-tuya-wasm-test-{}", uuid::Uuid::new_v4()));
    let engine = TuyaEngine::new(&data_dir);
    assert_eq!(engine.drain_wasm_commands(&host, 8).await, 1);
    let bytes = host
        .kv()
        .get("hello.official", "lan-result:req-1")
        .expect("native result");
    let result: serde_json::Value = serde_json::from_slice(&bytes).expect("result json");
    assert_eq!(result["ok"], false);
    assert_eq!(result["error"]["kind"], "offline");
}
