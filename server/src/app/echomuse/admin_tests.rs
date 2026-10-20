//! 管理面（`/api/echomuse/*`）响应决策的**外置测试**：纯函数直驱，不起真实 HTTP/WS。
//! 从 `tests.rs` 拆出（行数护栏）；跨文件共用的助手（`temp_registry`/`reg_load`/
//! `insert_probe`）与 hub 串行锁仍归 `tests.rs`/`hub_tests.rs`。

use super::admin::{
    approve as admin_approve, command_plan, device_payload, devices_payload, filter_settings,
    settings_plan, wait_version,
};
use super::hub_tests::{insert_online, HUB_LOCK};
use super::registry::Registry;
use super::tests::{insert_probe, reg_load, temp_registry};
use super::{hub_remove, ControlOut};

// ---- 管理面列表 / 详情 / 版本号 ----

#[test]
fn devices_payload_merges_hub_online() {
    let _g = HUB_LOCK.lock().unwrap();
    let (reg, path) = temp_registry("payload");
    reg.upsert(&reg_load("dev-on-1", "1.0"), 100);
    let ctl_rx = insert_online("dev-on-1");
    let v = devices_payload(true, &reg);
    assert_eq!(v["enabled"], true);
    // 设备面版本号随行（/wait 长挂的游标）：payload 与全局计数同源。
    assert_eq!(v["version"].as_u64(), Some(super::devices_version()));
    let devices = v["devices"].as_array().unwrap();
    let a = devices
        .iter()
        .find(|d| d["device_id"] == "dev-on-1")
        .unwrap();
    assert_eq!(a["online"], true);
    assert!(a["connected_secs"].is_u64());
    assert_eq!(a["label"], "dev-on-1");
    // 单台详情同口径；registry 没有的 hub 设备不出现在列表（registry 全表 left join hub）。
    assert_eq!(device_payload(&reg, "dev-on-1").unwrap()["online"], true);
    assert!(device_payload(&reg, "dev-missing").is_none());
    hub_remove("dev-on-1");
    drop(ctl_rx);
    let v = devices_payload(true, &reg);
    let a = v["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["device_id"] == "dev-on-1")
        .unwrap();
    assert_eq!(a["online"], false);
    assert!(a["connected_secs"].is_null());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn session_identity_changes_without_an_observed_offline_transition() {
    let _g = HUB_LOCK.lock().unwrap();
    let (reg, path) = temp_registry("session-identity");
    let id = "session-identity";
    reg.upsert(&reg_load(id, "same-firmware"), 100);
    let old_rx = insert_online(id);
    let (old_ctl, old_live) = super::hub_live(id).unwrap();
    old_live.update(&serde_json::json!({"type":"volume_state","level":20}));
    let before = device_payload(&reg, id).unwrap();
    assert_eq!(before["session_id"], old_live.payload(id)["session_id"]);
    let new_rx = insert_online(id); // Replaced directly, no offline snapshot.
    let (_, new_live) = super::hub_live(id).unwrap();
    super::hub_remove_session(id, &old_ctl); // Old cleanup must spare new state.
    let after = device_payload(&reg, id).unwrap();
    assert_eq!(before["online"], true);
    assert_eq!(after["online"], true);
    assert_ne!(before["session_id"], after["session_id"]);
    assert_eq!(after["session_id"], new_live.payload(id)["session_id"]);
    assert!(new_live.payload(id)["volume_state"].is_null());
    assert_eq!(
        devices_payload(true, &reg)["devices"][0]["session_id"],
        after["session_id"]
    );
    assert!(serde_json::to_value(reg.get(id).unwrap())
        .unwrap()
        .get("session_id")
        .is_none());
    hub_remove(id);
    assert!(device_payload(&reg, id).unwrap()["session_id"].is_null());
    drop((old_rx, new_rx));
    let _ = std::fs::remove_file(path);
}

#[test]
fn concurrent_device_events_never_lose_version_increments() {
    let _g = HUB_LOCK.lock().unwrap();
    let before = super::devices_version();
    let barrier = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let barrier = &barrier;
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..100 {
                    super::bump_devices_version();
                }
            });
        }
    });
    assert_eq!(super::devices_version(), before.wrapping_add(800));
}

// ---- /wait 事件保持（纯逻辑：自建 watch 通道，不碰进程级版本号） ----

#[test]
fn hub_lifecycle_bumps_devices_version() {
    let _g = HUB_LOCK.lock().unwrap();
    // hub 登记/移除（上线/下线）都要唤醒挂着的 /wait。
    let before = super::devices_version();
    insert_probe("dev-ver-1");
    assert_ne!(super::devices_version(), before);
    let after_insert = super::devices_version();
    hub_remove("dev-ver-1");
    assert_ne!(super::devices_version(), after_insert);
}

#[tokio::test]
async fn wait_version_returns_immediately_when_after_differs() {
    let (tx, mut rx) = tokio::sync::watch::channel(7u64);
    // after 落后（客户端漏掉一次通知）→ 立即回 changed。
    assert_eq!(
        wait_version(&mut rx, Some(6), std::time::Duration::from_secs(1)).await,
        (7, true)
    );
    // 服务端重启版本回绕（after > current）同样立即回，不空挂。
    assert_eq!(
        wait_version(&mut rx, Some(99), std::time::Duration::from_secs(1)).await,
        (7, true)
    );
    // 缺省 after（客户端首次没拿到版本）→ 立即回当前快照。
    assert_eq!(
        wait_version(&mut rx, None, std::time::Duration::from_secs(1)).await,
        (7, true)
    );
    drop(tx);
}

#[tokio::test]
async fn wait_version_wakes_on_bump_with_new_version() {
    let (tx, mut rx) = tokio::sync::watch::channel(3u64);
    let pusher = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let _ = tx.send(4);
    });
    assert_eq!(
        wait_version(&mut rx, Some(3), std::time::Duration::from_secs(2)).await,
        (4, true)
    );
    pusher.await.unwrap();
}

#[tokio::test]
async fn wait_version_times_out_without_change() {
    let (_tx, mut rx) = tokio::sync::watch::channel(3u64);
    assert_eq!(
        wait_version(&mut rx, Some(3), std::time::Duration::from_millis(20)).await,
        (3, false)
    );
}

// ---- 认领 / 指令下发决策 ----

#[test]
fn approve_payload_and_missing_404_path() {
    let (reg, path) = temp_registry("approve-payload");
    reg.upsert(&reg_load("dev-ap-2", "1.0"), 100);
    let v = admin_approve(&reg, "dev-ap-2", &serde_json::json!({"label":"卧室"})).unwrap();
    assert_eq!(v["approved"], true);
    assert_eq!(v["label"], "卧室");
    // registry 无记录 → None（handler 转 404）。
    assert!(admin_approve(&reg, "dev-missing", &serde_json::json!({})).is_none());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn command_plan_disabled_is_404() {
    let (code, body, out) = command_plan(false, true, &serde_json::json!({"action":"reboot"}));
    assert_eq!(code, axum::http::StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "echomuse disabled");
    assert!(out.is_none());
}

#[test]
fn command_plan_offline_is_409() {
    let (code, body, out) = command_plan(true, false, &serde_json::json!({"action":"reboot"}));
    assert_eq!(code, axum::http::StatusCode::CONFLICT);
    assert_eq!(body["error"], "device offline");
    assert!(out.is_none());
}

#[test]
fn command_plan_reboot_and_wifi_shapes() {
    let (code, body, out) = command_plan(true, true, &serde_json::json!({"action":"reboot"}));
    assert_eq!(code, axum::http::StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("confirm=true"));
    assert!(
        out.is_none(),
        "unsupported reboot message must never be queued"
    );
    let (code, _, out) = command_plan(
        true,
        true,
        &serde_json::json!({"action":"wifi","ssid":" home ","psk":"secret"}),
    );
    assert_eq!(code, axum::http::StatusCode::ACCEPTED);
    match out {
        Some(ControlOut::Json(s)) => {
            let v: serde_json::Value = serde_json::from_str(&s).unwrap();
            assert_eq!(v["type"], "wifi_change");
            assert_eq!(v["ssid"], "home"); // trim 生效
            assert_eq!(v["psk"], "secret");
        }
        other => panic!("wifi 应产出 Json 下发，实际 {other:?}"),
    }
}

#[test]
fn command_plan_bad_requests_are_400() {
    let (code, _, out) = command_plan(true, true, &serde_json::json!({"action":"leds"}));
    assert_eq!(code, axum::http::StatusCode::BAD_REQUEST);
    assert!(out.is_none());
    let (code, _, out) = command_plan(true, true, &serde_json::json!({"action":"wifi","psk":"x"}));
    assert_eq!(code, axum::http::StatusCode::BAD_REQUEST);
    assert!(out.is_none());
}

// ---- 管理面 settings（唤醒词配置） ----

use axum::http::StatusCode;

#[test]
fn filter_settings_allowlist_and_types() {
    let (ok, err) = filter_settings(&serde_json::json!({
        "owwThreshold": 0.55, "wakeWordEnabled": false, "owwModel": "alexa", "wakeMic": 0
    }));
    assert!(err.is_none());
    assert_eq!(ok.len(), 4);
    // 白名单外的键整体拒绝；类型错也拒绝。
    assert!(filter_settings(&serde_json::json!({"volume": 5}))
        .1
        .is_some());
    assert!(
        filter_settings(&serde_json::json!({"owwThreshold": "high"}))
            .1
            .is_some()
    );
    assert!(filter_settings(&serde_json::json!({"owwModel": true}))
        .1
        .is_some());
    assert!(filter_settings(&serde_json::json!([1])).1.is_some());
    // 蓝牙代理的每设备开关在白名单内（bool），类型错拒绝。
    let (ok, err) = filter_settings(
        &serde_json::json!({"bleProxyEnabled": true, "bleProxyConnections": false}),
    );
    assert!(err.is_none());
    assert_eq!(ok.len(), 2);
    assert!(
        filter_settings(&serde_json::json!({"bleProxyEnabled": "on"}))
            .1
            .is_some()
    );
}

#[test]
fn settings_plan_offline_persists_without_delivery() {
    let (code, body, outs) =
        settings_plan(true, false, true, &serde_json::json!({"owwThreshold": 0.4}));
    assert_eq!(code, StatusCode::OK);
    assert_eq!(body["persisted"], true);
    assert_eq!(body["delivered"], false);
    assert!(outs.is_empty());
}

#[test]
fn settings_plan_online_queues_config_with_wakewordenabled_separate() {
    let (code, body, outs) = settings_plan(
        true,
        true,
        true,
        &serde_json::json!({"owwOnDevice": "off", "wakeWordEnabled": false}),
    );
    assert_eq!(code, StatusCode::ACCEPTED);
    assert_eq!(body["delivered"], true);
    assert_eq!(outs.len(), 2, "wakeWordEnabled 按协议单独一条 config");
    let texts: Vec<String> = outs
        .into_iter()
        .map(|o| match o {
            ControlOut::Json(t) => t,
            _ => panic!("应为 Json"),
        })
        .collect();
    let first: serde_json::Value = serde_json::from_str(&texts[0]).unwrap();
    assert_eq!(first["type"], "config");
    assert_eq!(first["owwOnDevice"], "off");
    // 基线键仍在（streamReply），wakeWordEnabled 不混入存储配置条。
    assert_eq!(first["streamReply"], true);
    assert!(first.get("wakeWordEnabled").is_none());
    let second: serde_json::Value = serde_json::from_str(&texts[1]).unwrap();
    assert_eq!(second["wakeWordEnabled"], false);
}

#[test]
fn settings_plan_gates() {
    let body = serde_json::json!({"owwThreshold": 0.5});
    let (code, _, outs) = settings_plan(false, true, true, &body);
    assert_eq!(code, StatusCode::NOT_FOUND);
    assert!(outs.is_empty());
    let (code, _, _) = settings_plan(true, true, false, &body);
    assert_eq!(code, StatusCode::NOT_FOUND);
    let (code, _, _) = settings_plan(true, true, true, &serde_json::json!({"nope": 1}));
    assert_eq!(code, StatusCode::BAD_REQUEST);
}

#[test]
fn registry_settings_roundtrip_and_payload() {
    let (reg, path) = temp_registry("settings");
    reg.upsert(&reg_load("dev-set", "1.0"), 100);
    let mut m = std::collections::HashMap::new();
    m.insert("owwModel".to_string(), serde_json::json!("alexa"));
    let rec = reg.set_settings("dev-set", m.clone()).unwrap();
    assert_eq!(rec.settings["owwModel"], "alexa");
    // 落盘往返。
    let reloaded = Registry::load(path);
    assert_eq!(
        reloaded.get("dev-set").unwrap().settings["owwModel"],
        "alexa"
    );
    // payload 暴露 settings。
    assert_eq!(
        reloaded.get("dev-set").unwrap().payload(None)["settings"]["owwModel"],
        "alexa"
    );
    assert!(reg.set_settings("ghost", m).is_none());
}
