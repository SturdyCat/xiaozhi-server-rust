//! `api.rs` 响应体合约测试：**secret 永不回显**、掩码、payload 形状、凭证写入决策。
//! 纯函数直驱，无 HTTP、无网络。

use serde_json::json;

use super::api::{
    cloud_proxy_call, cloud_proxy_error, cloud_proxy_payload, credentials_apply, credentials_clear,
    device_detail_payload, devices_payload, failure_payload, mask_access_id, status_payload,
};
use super::lan::ControlFailure;
use super::store::DeviceRow;
use super::{SyncRecordView, TuyaConfig};

fn configured_cfg() -> TuyaConfig {
    TuyaConfig {
        engine: "native".into(),
        enabled: true,
        region: "cn".into(),
        access_id: "abcdefgh".into(),
        access_secret: "SUPER-SECRET-VALUE".into(),
        sync_interval_secs: 86_400,
    }
}

fn row(id: &str, online: bool) -> DeviceRow {
    DeviceRow {
        device_id: id.into(),
        local_key: format!("LOCAL-KEY-{id}"),
        name: format!("设备{id}"),
        model: "plug.m1".into(),
        product_id: "p".into(),
        ip: "192.168.1.5".into(),
        protocol_version: "3.4".into(),
        online,
        last_seen_ms: Some(1),
        alias: String::new(),
        updated_at: 123,
    }
}

// ---- secret 不回显（P0 铁律，最高优先级断言）----

#[test]
fn status_payload_never_contains_secret() {
    let cfg = configured_cfg();
    let body = status_payload(true, &cfg, None, false, &[]);
    let text = body.to_string();
    assert!(
        !text.contains("SUPER-SECRET-VALUE"),
        "status 响应绝不能包含 access_secret 原值"
    );
    assert!(
        body.get("access_secret").is_none(),
        "status 响应不应有 access_secret 键"
    );
    assert_eq!(body["configured"], json!(true));
    assert_eq!(body["access_id_masked"], json!("ab****gh"));
    assert_eq!(body["lan_unregistered"], json!([]));
}

/// LAN 发现的未登记设备透出到 status（管理台状态条据此显示，不再只有日志可见）。
#[test]
fn status_payload_carries_unregistered_devices() {
    let cfg = configured_cfg();
    let found = vec![super::UnregisteredDevice {
        device_id: "80246244f4cfa2cd0fc8".into(),
        ip: "192.168.99.50".into(),
        discovered_at_ms: super::store::now_ms() - 5_000,
    }];
    let body = status_payload(true, &cfg, None, false, &found);
    assert_eq!(
        body["lan_unregistered"][0]["device_id"],
        json!("80246244f4cfa2cd0fc8")
    );
    assert_eq!(body["lan_unregistered"][0]["ip"], json!("192.168.99.50"));
    let age = body["lan_unregistered"][0]["age_ms"]
        .as_i64()
        .expect("age_ms");
    assert!((0..=60_000).contains(&age), "age_ms 应在发现后一分钟内");
}

#[test]
fn devices_payload_never_contains_local_key() {
    let body = devices_payload(&[row("d1", true)]);
    let text = body.to_string();
    assert!(!text.contains("LOCAL-KEY-d1"), "设备列表绝不回显 local_key");
    assert_eq!(body["total"], json!(1));
    assert_eq!(body["online"], json!(1));
    assert_eq!(body["devices"][0]["device_id"], json!("d1"));
}

#[test]
fn device_detail_payload_never_contains_local_key() {
    let dps = std::collections::HashMap::from([("1".to_string(), json!(true))]);
    let body = device_detail_payload(&row("d1", true), Some(&dps), &|_| true);
    let text = body.to_string();
    assert!(!text.contains("LOCAL-KEY-d1"), "设备详情绝不回显 local_key");
    assert_eq!(body["dps"][0]["dp"], json!("1"));
    assert_eq!(body["dps"][0]["value"], json!(true));
    assert_eq!(body["dps"][0]["writable"], json!(true));
    assert_eq!(
        body["dps"][0]["control"],
        json!("switch"),
        "布尔 DP → 开关控件"
    );
}

// ---- 掩码 ----

#[test]
fn mask_access_id_rules() {
    assert_eq!(mask_access_id("abcdefgh"), "ab****gh");
    assert_eq!(mask_access_id("abc"), "****");
    assert_eq!(mask_access_id(""), "****");
    assert_eq!(mask_access_id("abcd"), "****");
}

// ---- 凭证写入决策 ----

#[test]
fn credentials_apply_sets_all_fields() {
    let mut cfg = TuyaConfig::default();
    credentials_apply(
        &mut cfg,
        &json!({"region":"eu","access_id":"id1","access_secret":"sec1"}),
    )
    .expect("apply");
    assert_eq!(
        (
            cfg.region.as_str(),
            cfg.access_id.as_str(),
            cfg.access_secret.as_str()
        ),
        ("eu", "id1", "sec1")
    );
    assert!(cfg.credentials_ready());
}

#[test]
fn credentials_apply_keeps_existing_secret_when_blank() {
    // 已配置过 → 只改区域/ID，secret 留空 = 保留旧值（不回显场景下的安全更新路径）
    let mut cfg = configured_cfg();
    credentials_apply(
        &mut cfg,
        &json!({"region":"cn","access_id":"id1","access_secret":""}),
    )
    .expect("apply");
    assert_eq!(cfg.access_secret, "SUPER-SECRET-VALUE");
}

#[test]
fn credentials_apply_rejects_bad_region_and_empty_id() {
    let mut cfg = configured_cfg();
    let err = credentials_apply(&mut cfg, &json!({"region":"mars","access_id":"id"}))
        .expect_err("bad region");
    assert!(err.contains("region"));
    let err =
        credentials_apply(&mut cfg, &json!({"region":"cn","access_id":""})).expect_err("empty id");
    assert!(err.contains("access_id"));
}

#[test]
fn credentials_apply_requires_secret_when_unconfigured() {
    let mut cfg = TuyaConfig::default();
    let err = credentials_apply(
        &mut cfg,
        &json!({"region":"cn","access_id":"id1","access_secret":""}),
    )
    .expect_err("must fail");
    assert!(err.contains("access_secret"));
}

#[test]
fn credentials_clear_wipes_only_credentials() {
    let mut cfg = configured_cfg();
    credentials_clear(&mut cfg);
    assert!(cfg.access_id.is_empty() && cfg.access_secret.is_empty());
    assert!(!cfg.credentials_ready());
    assert_eq!(cfg.region, "cn");
    assert_eq!(cfg.sync_interval_secs, 86_400, "清除凭证不动开关与周期");
}

// ---- 失败响应（三归类 + 建议动作）----

#[test]
fn failure_payload_carries_kind_and_advice() {
    let body = failure_payload(&ControlFailure::offline("设备离线".into()));
    assert_eq!(body["kind"], json!("offline"));
    assert_eq!(
        body["advice"],
        json!(ControlFailure::offline("x".into()).advice)
    );
    let timeout = ControlFailure {
        kind: "timeout",
        message: "无响应".into(),
        advice: "稍后重试",
    };
    assert_eq!(failure_payload(&timeout)["kind"], json!("timeout"));
}

// ---- last_sync 投影 ----

#[test]
fn sync_record_view_projects_age() {
    use super::store::{now_ms, SyncRecord};
    let record = SyncRecord {
        started_at: now_ms() - 1_500,
        finished_at: now_ms() - 1_000,
        result: "ok".into(),
        device_count: 3,
        message: String::new(),
    };
    let view = SyncRecordView::from_record(&record);
    assert_eq!(view.result, "ok");
    assert_eq!(view.device_count, 3);
    assert!((1_000..=2_000).contains(&view.age_ms), "age_ms 应约等于 1s");
}

// ---- 云调用调试代理（cloud_proxy_call / payload）----

#[test]
fn proxy_call_whitelists_methods_and_path_prefix() {
    let err = cloud_proxy_call(&json!({"method": "PUT", "path": "/v1.0/devices"}))
        .expect_err("PUT 应被拒");
    assert!(err.contains("GET/POST/DELETE"));

    let err = cloud_proxy_call(&json!({"path": "/api/other"})).expect_err("非 /v1. 路径应被拒");
    assert!(err.contains("/v1."));

    let call = cloud_proxy_call(&json!({"path": "/v1.0/token?grant_type=1"}))
        .expect_err("path 内嵌 ? 应被拒");
    let _ = call; // 恒为 Err，上面 expect_err 已断言
}

#[test]
fn proxy_call_defaults_to_get_and_sorts_query_keys() {
    // query 字典序：last_row_key 在 size 前；数值 50 序列化为 "50"
    let call = cloud_proxy_call(&json!({
        "path": "/v1.0/iot-01/associated-users/devices",
        "query": {"size": 50, "last_row_key": "ABC123"}
    }))
    .expect("ok");
    assert_eq!(call.method, "GET");
    assert_eq!(
        call.path_query,
        "/v1.0/iot-01/associated-users/devices?last_row_key=ABC123&size=50"
    );
    assert_eq!(call.body_json, "");
}

#[test]
fn proxy_call_encodes_special_query_values() {
    let call = cloud_proxy_call(&json!({
        "path": "/v1.0/devices",
        "query": {"name": "客厅 电源"}
    }))
    .expect("ok");
    assert_eq!(
        call.path_query,
        "/v1.0/devices?name=%E5%AE%A2%E5%8E%85%20%E7%94%B5%E6%BA%90"
    );
}

#[test]
fn proxy_call_body_only_allowed_for_post() {
    let err = cloud_proxy_call(&json!({
        "path": "/v1.0/devices/d1", "body": {"x": 1}
    }))
    .expect_err("GET 带 body 应被拒");
    assert!(err.contains("POST"));

    let call = cloud_proxy_call(&json!({
        "method": "POST",
        "path": "/v1.0/devices/d1/commands",
        "body": {"commands": [{"code": "switch_1", "value": true}]}
    }))
    .expect("POST 带 body 应通过");
    assert_eq!(call.method, "POST");
    assert_eq!(
        call.body_json,
        r#"{"commands":[{"code":"switch_1","value":true}]}"#
    );
}

#[test]
fn proxy_payload_passthrough_and_non_json_fallback() {
    let payload = cloud_proxy_payload(200, r#"{"success":true,"result":{"total":2}}"#);
    assert_eq!(payload["status"], 200);
    assert_eq!(payload["body"]["result"]["total"], 2);

    let raw = cloud_proxy_payload(502, "gateway hiccup（非 JSON）");
    assert_eq!(raw["body"], "gateway hiccup（非 JSON）");
}

#[test]
fn proxy_error_carries_kind_without_secret() {
    let e = cloud_proxy_error("auth", "云鉴权失败（code 1004）: sign invalid");
    assert_eq!(e["kind"], "auth");
    assert_eq!(e["error"], "云鉴权失败（code 1004）: sign invalid");
    assert!(
        !e["error"].to_string().contains("SUPER-SECRET-VALUE"),
        "消息不含 secret"
    );
}
