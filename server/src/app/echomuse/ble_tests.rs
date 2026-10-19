//! 蓝牙代理测试：`0x06`/`0x08` 帧形状、`BleLink` 请求关联与采集、GATT 请求构造。
//!
//! 全部纯函数 / channel 直驱，不起真实 WS 与蓝牙栈；端点 gate 语义复用 admin 面
//! 既有决策路径，这里覆盖 BLE 特有的状态机与协议形状。

use tokio::sync::oneshot;

use super::ble::{build_gatt_request, BleLink, ADVERT_CAP, EVENT_CAP, STATE_EVENTS};
use super::frames::{frame_ble_gatt, parse_uplink, UplinkFrame, FRAME_BLE_ADVERTS, FRAME_BLE_GATT};

// ---- 能力协商 / config 基线（协议契约） ----

#[test]
fn ack_declares_ble_only_when_enabled() {
    // ble=true：追加 ble_connect + ble_adverts_data，其余能力不变。
    let v: serde_json::Value = serde_json::from_str(&super::ack_message("dev-1", true)).unwrap();
    let names: Vec<&str> = v["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f.as_str())
        .collect();
    assert!(names.contains(&"ble_connect") && names.contains(&"ble_adverts_data"));
    assert!(names.contains(&"mic") && !names.contains(&"listen_session"));
}

#[test]
fn config_baseline_defaults_and_overrides() {
    // 关闭：零键；开启：两个协议键默认打开，每设备 settings 覆盖基线。
    let off = super::config_baseline(false, &serde_json::json!({}));
    assert_eq!(off, serde_json::json!({}));
    let on = super::config_baseline(true, &serde_json::json!({}));
    assert_eq!(on["bleProxyEnabled"], true);
    assert_eq!(on["bleProxyConnections"], true);
    // 单台关连接留扫描：settings 覆盖基线，基线另一键仍在。
    let scan_only = super::config_baseline(
        true,
        &serde_json::json!({"bleProxyConnections": false, "owwThreshold": 0.5}),
    );
    assert_eq!(scan_only["bleProxyEnabled"], true);
    assert_eq!(scan_only["bleProxyConnections"], false);
    assert_eq!(scan_only["owwThreshold"], 0.5);
}

// ---- 帧形状 ----

#[test]
fn ble_adverts_frame_parses_with_payload() {
    let mut frame = vec![FRAME_BLE_ADVERTS];
    frame.extend_from_slice(br#"{"adverts":[]}"#);
    assert_eq!(
        parse_uplink(&frame),
        UplinkFrame::BleAdverts(br#"{"adverts":[]}"#.as_slice())
    );
}

#[test]
fn ble_gatt_frame_parses_both_directions() {
    // 上行方向解析（结果/事件）；下行用同一帧型由 frame_ble_gatt 组帧。
    assert_eq!(
        parse_uplink(&[FRAME_BLE_GATT, b'{', b'}']),
        UplinkFrame::BleGatt(b"{}".as_slice())
    );
    assert_eq!(frame_ble_gatt(r#"{"t":"slots"}"#), {
        let mut v = vec![FRAME_BLE_GATT];
        v.extend_from_slice(br#"{"t":"slots"}"#);
        v
    });
}

// ---- BleLink：请求关联 ----

fn result_ok(req: u32) -> Vec<u8> {
    format!(r#"{{"t":"result","req":{req},"ok":true,"mtu":247}}"#).into_bytes()
}

#[test]
fn alloc_req_starts_at_one_and_increments() {
    let mut link = BleLink::default();
    assert_eq!(link.alloc_req(), 1);
    assert_eq!(link.alloc_req(), 2);
}

#[tokio::test]
async fn result_resolves_matching_request() {
    let mut link = BleLink::default();
    let req = link.alloc_req();
    let (tx, mut rx) = oneshot::channel();
    link.register(req, tx);
    // 错误号不唤醒；同号唤醒并带原 result。
    link.on_gatt(&result_ok(req + 1));
    assert!(rx.try_recv().is_err());
    link.on_gatt(&result_ok(req));
    let v = rx.await.expect("登记的请求应被唤醒");
    assert_eq!(v["ok"], true);
    assert_eq!(v["mtu"], 247);
    // 重复结果无人等待：静默忽略（协议未知/迟到一律不报错）。
    link.on_gatt(&result_ok(req));
}

#[tokio::test]
async fn slots_event_answers_query_and_records_state() {
    let mut link = BleLink::default();
    let req = link.alloc_req();
    let (tx, mut rx) = oneshot::channel();
    link.register(req, tx);
    // 自发 slots（无 req）：只记录不唤醒。
    link.on_gatt(br#"{"t":"slots","free":3,"limit":3,"addrs":[]}"#);
    assert!(rx.try_recv().is_err());
    let (slots, _) = link.state_snapshot();
    assert_eq!(slots.expect("应记录槽位")["free"], 3);
    // 带 req 的 slots：记录且兼作查询应答。
    link.on_gatt(br#"{"t":"slots","req":9,"free":2,"limit":3,"addrs":["aa:bb:cc:dd:ee:ff"]}"#);
    // req 不匹配（9 ≠ 1）不唤醒，但状态已刷新。
    assert!(rx.try_recv().is_err());
    let (slots, _) = link.state_snapshot();
    assert_eq!(slots.expect("应刷新槽位")["free"], 2);
    let req2 = link.alloc_req();
    let (tx2, rx2) = oneshot::channel();
    link.register(req2, tx2);
    link.on_gatt(&format!(r#"{{"t":"slots","req":{req2},"free":1,"limit":3}}"#).into_bytes());
    let v = rx2.await.expect("slots 查询应由带 req 的事件应答");
    assert_eq!(v["free"], 1);
}

#[tokio::test]
async fn notify_and_disconnected_enter_event_ring_with_cap() {
    let mut link = BleLink::default();
    for i in 0..EVENT_CAP + 5 {
        link.on_gatt(
            &format!(
                r#"{{"t":"notify","addr":"aa:bb:cc:dd:ee:ff","handle":3,"value":"AQ==","ind":false,"i":{i}}}"#
            )
            .into_bytes(),
        );
    }
    link.on_gatt(br#"{"t":"disconnected","addr":"aa:bb:cc:dd:ee:ff","reason":19}"#);
    let (_, events) = link.state_snapshot();
    // 快照取环尾 STATE_EVENTS 条；末尾是最新事件（disconnected）。
    assert_eq!(events.len(), STATE_EVENTS);
    assert_eq!(events.last().expect("非空")["t"], "disconnected");
    assert_eq!(events.last().expect("非空")["reason"], 19);
    // 首条可见序号同时区分两层截断：环封顶 EVENT_CAP（未截则应为 57）+
    // 快照窗口 STATE_EVENTS（105 条 notify + 1 条 disconnected 的环尾 50 条）。
    assert_eq!(
        events.first().expect("非空")["i"],
        (EVENT_CAP + 5 - STATE_EVENTS + 1) as u64
    );
}

#[test]
fn garbage_and_unknown_types_are_ignored() {
    let mut link = BleLink::default();
    link.on_gatt(b"not json");
    link.on_gatt(br#"[]"#);
    link.on_gatt(br#"{"t":"mystery","req":1}"#);
    let (slots, events) = link.state_snapshot();
    assert!(slots.is_none());
    assert!(events.is_empty());
}

#[tokio::test]
async fn reset_drops_pending_requests() {
    let mut link = BleLink::default();
    let (tx, rx) = oneshot::channel();
    let req = link.alloc_req();
    link.register(req, tx);
    link.merge_adverts(
        &[serde_json::json!({"addr":"aa:bb:cc:dd:ee:ff","rssi":-60})],
        1,
    );
    link.reset();
    // 等待方立即得到关闭（离线口径），采集数据一并清空。
    assert!(rx.await.is_err());
    let (at, adverts) = link.adverts_snapshot();
    assert_eq!(at, 0);
    assert!(adverts.is_empty());
}

// ---- BleLink：广播采集 ----

fn advert(addr: &str, rssi: i64, seen: u64) -> serde_json::Value {
    serde_json::json!({"addr": addr, "addrType": 1, "rssi": rssi, "data": "AQID", "seen": seen})
}

#[test]
fn merge_adverts_stamps_replaces_and_skips_addrless() {
    let mut link = BleLink::default();
    link.merge_adverts(
        &[
            advert("aa:bb:cc:dd:ee:01", -60, 0),
            serde_json::json!({"rssi": -70}), // 缺 addr：忽略
        ],
        100,
    );
    // 同 addr 新批次覆盖并刷新 seenAt；设备地址原样保留（协议透传，不归一化）。
    link.merge_adverts(&[advert("aa:bb:cc:dd:ee:01", -55, 0)], 200);
    let (at, items) = link.adverts_snapshot();
    assert_eq!(at, 200);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["rssi"], -55);
    assert_eq!(items[0]["seenAt"], 200);
}

#[test]
fn merge_adverts_caps_by_evicting_oldest() {
    let mut link = BleLink::default();
    for i in 0..ADVERT_CAP {
        link.merge_adverts(
            &[advert(&format!("aa:bb:cc:dd:{:02x}:01", i % 256), -60, 0)],
            i as u64 + 1,
        );
    }
    assert_eq!(link.adverts_snapshot().1.len(), ADVERT_CAP);
    // 再进一批更老的 seenAt：总量不超限，最旧者被淘汰。
    link.merge_adverts(
        &[advert("aa:bb:cc:00:00:ff", -90, 0)],
        ADVERT_CAP as u64 + 1,
    );
    let items = link.adverts_snapshot().1;
    assert_eq!(items.len(), ADVERT_CAP);
    assert!(items.iter().any(|a| a["addr"] == "aa:bb:cc:00:00:ff"));
}

#[test]
fn adverts_snapshot_is_sorted_by_addr() {
    let mut link = BleLink::default();
    link.merge_adverts(
        &[
            advert("aa:bb:cc:00:00:02", -60, 0),
            advert("aa:bb:cc:00:00:01", -61, 0),
        ],
        7,
    );
    let (at, items) = link.adverts_snapshot();
    assert_eq!(at, 7);
    let addrs: Vec<String> = items
        .iter()
        .filter_map(|a| a["addr"].as_str().map(str::to_string))
        .collect();
    let mut sorted = addrs.clone();
    sorted.sort();
    assert_eq!(addrs, sorted);
}

// ---- GATT 请求构造 ----

#[test]
fn build_gatt_connect_normalizes_addr_and_defaults() {
    let v = build_gatt_request(&serde_json::json!({"t":"connect","addr":" AA:BB:CC:DD:EE:FF "}))
        .expect("合法 connect");
    assert_eq!(v["t"], "connect");
    assert_eq!(v["addr"], "aa:bb:cc:dd:ee:ff");
    assert_eq!(v["addr_type"], 0); // 缺省 public
    let v = build_gatt_request(
        &serde_json::json!({"t":"connect","addr":"aa:bb:cc:dd:ee:ff","addr_type":1}),
    )
    .unwrap();
    assert_eq!(v["addr_type"], 1);
}

#[test]
fn build_gatt_write_canonicalizes_base64_and_defaults_response() {
    let v = build_gatt_request(
        &serde_json::json!({"t":"write","addr":"aa:bb:cc:dd:ee:ff","handle":3,"value":"AQID"}),
    )
    .unwrap();
    assert_eq!(v["handle"], 3);
    assert_eq!(v["value"], "AQID");
    assert_eq!(v["response"], true);
}

#[test]
fn build_gatt_bad_requests_are_rejected() {
    // 未知 t / 缺 t。
    assert!(build_gatt_request(&serde_json::json!({"t":"pair"})).is_err());
    assert!(build_gatt_request(&serde_json::json!({"addr":"aa:bb:cc:dd:ee:ff"})).is_err());
    // 地址形状 / addr_type / handle 越界 / 非法 base64 / 缺字段。
    assert!(build_gatt_request(&serde_json::json!({"t":"connect","addr":"aabbccddeeff"})).is_err());
    assert!(
        build_gatt_request(&serde_json::json!({"t":"connect","addr":"aa:bb:cc:dd:ee:gg"})).is_err()
    );
    assert!(build_gatt_request(
        &serde_json::json!({"t":"connect","addr":"aa:bb:cc:dd:ee:ff","addr_type":2})
    )
    .is_err());
    assert!(build_gatt_request(
        &serde_json::json!({"t":"read","addr":"aa:bb:cc:dd:ee:ff","handle":70000})
    )
    .is_err());
    assert!(build_gatt_request(
        &serde_json::json!({"t":"write","addr":"aa:bb:cc:dd:ee:ff","handle":3,"value":"not base64!"})
    )
    .is_err());
    assert!(
        build_gatt_request(&serde_json::json!({"t":"read","addr":"aa:bb:cc:dd:ee:ff"})).is_err()
    );
    // 合法集合：disconnect/services/slots 不需要多余字段。
    assert!(
        build_gatt_request(&serde_json::json!({"t":"disconnect","addr":"aa:bb:cc:dd:ee:ff"}))
            .is_ok()
    );
    assert!(
        build_gatt_request(&serde_json::json!({"t":"services","addr":"aa:bb:cc:dd:ee:ff"})).is_ok()
    );
    assert!(build_gatt_request(&serde_json::json!({"t":"slots"})).is_ok());
}

#[test]
fn gatt_request_json_is_framed_with_type_byte() {
    // 请求体 + req 组帧后的完整形状（端点下发路径的最终字节）。
    let mut body = build_gatt_request(
        &serde_json::json!({"t":"write","addr":"aa:bb:cc:dd:ee:ff","handle":3,"value":"AQ==","response":false}),
    )
    .unwrap();
    body["req"] = serde_json::json!(42);
    let frame = frame_ble_gatt(&body.to_string());
    assert_eq!(frame[0], FRAME_BLE_GATT);
    let v: serde_json::Value = serde_json::from_slice(&frame[1..]).unwrap();
    assert_eq!(v["t"], "write");
    assert_eq!(v["req"], 42);
    assert_eq!(v["response"], false);
}
