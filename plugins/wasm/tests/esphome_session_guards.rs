//! S3u：esphome.official 设备侧会话——事件面生命周期 / fail-closed 守卫 + guest 自检。
//!
//! 纯 guest 事件面（无 I/O）：与真 TCP 全链对拍（`esphome_session.rs` /
//! `esphome_session_encrypted.rs`）互补，复用其助手（`super::esphome_session`）。

use esphome_native_api::parser::{proto_to_vec, ProtoMessage};
use esphome_native_api::proto::HelloRequest;
use serde_json::{json, Value};
use std::sync::Arc;

use super::esphome_session::{
    call_session, err, hex_encode, host, ok, test_entities, test_identity, wire_frame, STATE_PREFIX,
};
use super::{ensure_fixture, host_with};

const PLUGIN: &str = "esphome.official";

// ==================================================================
// 事件面生命周期 / fail-closed 守卫（纯 guest 事件面，无 I/O）
// ==================================================================

#[test]
fn session_lifecycle_and_fail_closed_guards() {
    let host = host();
    let open_payload = |session: &str, entities: Value| {
        json!({
            "req_id": format!("g-{session}"),
            "op": "open",
            "session": session,
            "identity": test_identity(),
            "entities": entities,
        })
    };

    // 重复 open 拒绝覆盖（防会话状态被静默重置）。
    ok(&call_session(&host, open_payload("g1", test_entities())));
    let duplicate = call_session(&host, open_payload("g1", test_entities()));
    assert!(err(&duplicate).contains("已存在"), "{duplicate}");

    // 未 open 的会话 feed 显式失败（不静默建新状态）。
    let missing = call_session(
        &host,
        json!({"req_id": "g-feed", "op": "feed", "session": "g2", "data_hex": "000101"}),
    );
    assert!(err(&missing).contains("不存在"), "{missing}");

    // 非法 psk 长度 / 非法 session 字符集 / 未知 op 全部 fail-closed。
    let bad_psk = call_session(
        &host,
        json!({
            "req_id": "g-psk", "op": "open", "session": "g3",
            "psk_hex": "0011",
            "identity": test_identity(),
            "entities": test_entities(),
        }),
    );
    assert!(err(&bad_psk).contains("psk"), "{bad_psk}");
    let bad_session = call_session(
        &host,
        json!({
            "req_id": "g-session", "op": "open", "session": "bad session!",
            "identity": test_identity(),
            "entities": test_entities(),
        }),
    );
    assert!(err(&bad_session).contains("session"), "{bad_session}");
    let bad_op = call_session(
        &host,
        json!({"req_id": "g-op", "op": "explode", "session": "g1"}),
    );
    assert!(err(&bad_op).contains("未知 session op"), "{bad_op}");

    // 实体目录非法（key 重复）→ open 失败且不留状态。
    let dup_entities = json!([
        {"key": 1, "kind": "binary_sensor", "object_id": "a", "name": "a", "state": false},
        {"key": 1, "kind": "binary_sensor", "object_id": "b", "name": "b", "state": false},
    ]);
    let dup = call_session(&host, open_payload("g4", dup_entities));
    assert!(err(&dup).contains("重复"), "{dup}");
    assert!(
        host.kv()
            .get(PLUGIN, &format!("{STATE_PREFIX}:g4"))
            .is_none(),
        "失败的 open 不应留状态"
    );

    // 未知实体 key 的 state update 拒绝；会话不被误毁（后续 feed 正常）。
    ok(&call_session(&host, open_payload("g5", test_entities())));
    let unknown = call_session(
        &host,
        json!({
            "req_id": "g-unknown", "op": "state", "session": "g5",
            "updates": [{"key": 99, "state": true}],
        }),
    );
    assert!(err(&unknown).contains("未知实体 key"), "{unknown}");
    let feed = call_session(
        &host,
        json!({
            "req_id": "g-feed-ok", "op": "feed", "session": "g5",
            "data_hex": hex_encode(&hello_request_frame()),
        }),
    );
    assert_eq!(ok(&feed)["messages"][0]["type"], json!(1), "会话应仍存活");

    // 订阅前 state 推送：更新落目录但不产出帧。
    let state = ok(&call_session(
        &host,
        json!({
            "req_id": "g-state", "op": "state", "session": "g5",
            "updates": [{"key": 1, "state": true}],
        }),
    ));
    assert_eq!(state["out_hex"], json!(""), "未订阅不产出状态帧");

    // 显式 close → 状态回收；再 feed 报不存在；再 close 无可删。
    let close = ok(&call_session(
        &host,
        json!({"req_id": "g-close", "op": "close", "session": "g5"}),
    ));
    assert_eq!(close["removed"], json!(true));
    let after = call_session(
        &host,
        json!({"req_id": "g-after", "op": "feed", "session": "g5", "data_hex": "000101"}),
    );
    assert!(err(&after).contains("不存在"), "{after}");
    let close_again = ok(&call_session(
        &host,
        json!({"req_id": "g-close2", "op": "close", "session": "g5"}),
    ));
    assert_eq!(close_again["removed"], json!(false));
}

/// 守卫用例的最小 HelloRequest 帧（载荷走参考 prost 编码，与主链同源）。
fn hello_request_frame() -> Vec<u8> {
    let payload = proto_to_vec(&ProtoMessage::HelloRequest(HelloRequest {
        client_info: "home-assistant".to_string(),
        api_version_major: 1,
        api_version_minor: 10,
    }))
    .expect("prost 编码 HelloRequest");
    wire_frame(1, &payload)
}

// ==================================================================
// guest 内自检（validate 全链）：设备侧会话自检项在 wasm 内执行
// ==================================================================

#[test]
fn self_test_covers_device_session_in_wasm() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let results = host
        .validate(PLUGIN, &ensure_fixture("esphome"))
        .expect("validate");
    assert!(results.iter().all(|r| r.ok), "自检应全过：{results:?}");
    let session = results
        .iter()
        .find(|r| r.name == "esphome-device-session")
        .expect("自检应包含设备侧会话项");
    assert!(
        session.detail.contains("明文会话"),
        "会话自检 detail 不符合预期：{}",
        session.detail
    );
}
