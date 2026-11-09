//! S3c：tuya.official 云签名链路端到端回归（真实插件源 plugins-src/tuya）。
//!
//! 链路 = 读插件自身 config 凭证（config / config-secret 权限）→ 单源协议
//! （#[path] 共用的 server/src/device/tuya/protocol.rs）构造官方 v1.0 签名的
//! token 请求 → 宿主代发（脚本化应答）→ `http.responses` 回灌 → 解析 token
//! 落 kv 结果通道。凭证永不离开插件配置命名空间；宿主只见签名后的请求。

use serde_json::{json, Value};
use std::sync::Arc;

use super::{ensure_fixture, fixture, host_with, ScriptedFetcher};
use crate::plugins::wasm::WasmHost;

const PLUGIN: &str = "tuya.official";
const TOKEN_TOPIC: &str = "connector.tuya.cloud-token";

fn seeded_host(fetcher: &Arc<ScriptedFetcher>) -> WasmHost {
    let host = host_with(fetcher);
    let store = host.config_store();
    store.set(PLUGIN, "region", "cn").expect("seed region");
    store
        .set(PLUGIN, "access_id", "aid-test")
        .expect("seed access_id");
    store
        .set(PLUGIN, "access_secret", "asec-test")
        .expect("seed access_secret");
    host.load(PLUGIN, &ensure_fixture("tuya"))
        .expect("load tuya.official");
    host
}

/// token 回包（真实形状：result.{access_token, expire_time}）。
const TOKEN_OK: &str =
    r#"{"code":0,"success":true,"msg":"ok","result":{"access_token":"tok-1","expire_time":7200}}"#;

fn result_of(host: &WasmHost, req_id: &str) -> Value {
    let bytes = host
        .kv()
        .get(PLUGIN, &format!("tuya-cloud-token:{req_id}"))
        .unwrap_or_else(|| panic!("token 结果未落 kv（req_id={req_id}）"));
    serde_json::from_slice(&bytes).expect("结果应为合法 JSON")
}

#[test]
fn cloud_token_full_chain_signs_fetches_and_parses() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    fetcher.push_ok(200, TOKEN_OK);
    let host = seeded_host(&fetcher);

    host.handle_event(
        PLUGIN,
        TOKEN_TOPIC,
        &json!({"req_id": "tk1", "t": "1700000000000"}).to_string(),
    )
    .expect("cloud-token dispatch");

    // ① 请求确实发往官方 token 端点（region=cn → cn host）
    let first_url = fetcher.urls.lock().expect("urls lock").first().cloned();
    assert_eq!(
        first_url.as_deref(),
        Some("https://openapi.tuyacn.com/v1.0/token?grant_type=1"),
        "应发往 region 对应的官方端点，实际 {:?}",
        fetcher.urls
    );
    // ② 回灌解析后结果落 kv：token / 有效期 / route=plugin
    let v = result_of(&host, "tk1");
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["access_token"], json!("tok-1"), "{v}");
    assert_eq!(v["expires_in"], json!(7200), "{v}");
    assert_eq!(v["route"], json!("plugin"), "{v}");
}

#[test]
fn cloud_token_fails_closed_without_credentials() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    // 不种配置：host_with 默认 MemoryConfigStore 为空
    let host = host_with(&fetcher);
    host.load(PLUGIN, &ensure_fixture("tuya"))
        .expect("load tuya.official");

    host.handle_event(
        PLUGIN,
        TOKEN_TOPIC,
        &json!({"req_id": "tk2", "t": "1700000000000"}).to_string(),
    )
    .expect("dispatch completes");

    // fail-closed：未配置凭证 → 业务性 Err → 不发 HTTP、不写结果键
    assert!(
        fetcher.urls.lock().expect("urls lock").is_empty(),
        "缺凭证不得发起 HTTP：{:?}",
        fetcher.urls
    );
    assert!(
        host.kv().get(PLUGIN, "tuya-cloud-token:tk2").is_none(),
        "缺凭证不得写出 token 结果"
    );
}

#[test]
fn cloud_token_reports_upstream_error_payload() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    fetcher.push_ok(200, r#"{"code":1001,"success":false,"msg":"sign invalid"}"#);
    let host = seeded_host(&fetcher);

    host.handle_event(
        PLUGIN,
        TOKEN_TOPIC,
        &json!({"req_id": "tk3", "t": "1700000000000"}).to_string(),
    )
    .expect("dispatch completes");

    // 云端业务错误也走结果通道（ok:false + 错误文本），不静默、不 panic
    let v = result_of(&host, "tk3");
    assert_eq!(v["ok"], json!(false), "{v}");
    let err = v["error"].as_str().expect("error text");
    assert!(
        err.contains("1001") && err.contains("sign invalid"),
        "{err}"
    );
}

// ---- S3d：设备清单分页（自动 token 链 + 显式 token + 游标形状）----

/// 真机形状设备清单页（2026-11-03：result.{devices[], has_more, last_row_key}）。
const DEVICES_PAGE: &str = r#"{"code":0,"success":true,"msg":"ok","result":{
  "devices":[
    {"id":"ivf9id1","name":"客厅开关","model":"m1","local_key":"k1","online":true,"ip":"192.168.1.2","product_id":"p1"},
    {"id":"ivf9id2","name":"卧室插座","model":"m2","local_key":"k2","online":false,"ip":"","product_id":"p2"}],
  "has_more":true,"last_row_key":"cur1","total":3}}"#;

fn result_of_devices(host: &WasmHost, req_id: &str) -> Value {
    let bytes = host
        .kv()
        .get(PLUGIN, &format!("tuya-devices:{req_id}"))
        .unwrap_or_else(|| panic!("devices 结果未落 kv（req_id={req_id}）"));
    serde_json::from_slice(&bytes).expect("结果应为合法 JSON")
}

#[test]
fn cloud_devices_auto_chain_fetches_token_then_page() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    fetcher.push_ok(200, TOKEN_OK);
    fetcher.push_ok(200, DEVICES_PAGE);
    let host = seeded_host(&fetcher);

    host.handle_event(
        PLUGIN,
        "connector.tuya.cloud-devices",
        &json!({"req_id": "dv1", "t": "1700000000000"}).to_string(),
    )
    .expect("cloud-devices dispatch");

    // ① 自动链两步：先 token 端点、再设备清单端点（query 字典序 size 在后）
    let urls = fetcher.urls.lock().expect("urls lock").clone();
    assert_eq!(
        urls.first().map(String::as_str),
        Some("https://openapi.tuyacn.com/v1.0/token?grant_type=1"),
        "第一步应取 token，实际 {urls:?}"
    );
    assert_eq!(
        urls.get(1).map(String::as_str),
        Some("https://openapi.tuyacn.com/v1.0/iot-01/associated-users/devices?size=50"),
        "第二步应取设备清单，实际 {urls:?}"
    );
    // ② 页形状解析落结果通道：count / has_more / 游标 / 原始设备行
    let v = result_of_devices(&host, "dv1");
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["count"], json!(2), "{v}");
    assert_eq!(v["has_more"], json!(true), "{v}");
    assert_eq!(v["last_row_key"], json!("cur1"), "{v}");
    assert_eq!(v["route"], json!("plugin"), "{v}");
    assert_eq!(v["devices"][0]["id"], json!("ivf9id1"), "{v}");
    assert_eq!(v["devices"][1]["online"], json!(false), "{v}");
}

#[test]
fn cloud_devices_with_explicit_token_and_cursor_skips_token_step() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    fetcher.push_ok(200, DEVICES_PAGE);
    let host = seeded_host(&fetcher);

    host.handle_event(
        PLUGIN,
        "connector.tuya.cloud-devices",
        &json!({
            "req_id": "dv2",
            "token": "tok-explicit",
            "t": "1700000000001",
            "last_row_key": "cur1"
        })
        .to_string(),
    )
    .expect("dispatch completes");

    // 显式 token：只有一步设备清单请求；游标在 query 且字典序 last_row_key < size
    let urls = fetcher.urls.lock().expect("urls lock").clone();
    assert_eq!(urls.len(), 1, "显式 token 不得再取 token：{urls:?}");
    assert_eq!(
        urls.first().map(String::as_str),
        Some("https://openapi.tuyacn.com/v1.0/iot-01/associated-users/devices?last_row_key=cur1&size=50"),
        "游标翻页 URL 应字典序携带 last_row_key，实际 {urls:?}"
    );
    let v = result_of_devices(&host, "dv2");
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["last_row_key"], json!("cur1"), "{v}");
}

#[test]
fn cloud_devices_token_error_lands_in_result_channel() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    fetcher.push_ok(
        200,
        r#"{"code":1004,"success":false,"msg":"token invalid"}"#,
    );
    let host = seeded_host(&fetcher);

    host.handle_event(
        PLUGIN,
        "connector.tuya.cloud-devices",
        &json!({"req_id": "dv3", "t": "1700000000000"}).to_string(),
    )
    .expect("dispatch completes");

    // token 步失败 → 结果通道 ok:false（不再发第二步请求）
    let urls = fetcher.urls.lock().expect("urls lock").clone();
    assert_eq!(urls.len(), 1, "token 失败不得继续拉页：{urls:?}");
    let v = result_of_devices(&host, "dv3");
    assert_eq!(v["ok"], json!(false), "{v}");
    assert!(v["error"].as_str().unwrap().contains("1004"), "{v}");
}
