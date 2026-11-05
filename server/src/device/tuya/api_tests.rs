//! `api.rs` 响应体合约测试：**secret 永不回显**、掩码、payload 形状、凭证写入决策。
//! 纯函数直驱，无 HTTP、无网络。

use serde_json::json;

use super::api::{
    credentials_apply, credentials_clear, device_detail_payload, devices_payload, failure_payload,
    mask_access_id, status_payload,
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
    let body = status_payload(true, &cfg, None, false);
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
