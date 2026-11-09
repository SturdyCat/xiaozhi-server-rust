//! S3u：esphome.official **设备侧会话（responder）**——真 TCP 全链对拍（明文）。
//!
//! 插件在 S3u 中是 ESPHome **设备侧**：Home Assistant 客户端连进来。对拍权威 =
//! 外部 crate `esphome-native-api` 3.0.0：客户端→设备方向的载荷经参考 crate 本尊的
//! `parser::proto_to_vec`（prost）编码成明文帧发给插件；插件的应答载荷与参考 prost
//! 编码**逐字节对拍**，并经 `parse_proto_message` 还原字段核对身份面。
//!
//! 全链路 = **真 TCP**（localhost 随机端口）：测试线程在客户端字节与 guest
//! `connector.esp-home.session` 事件之间手工泵（客户端字节 → `feed.data_hex`；
//! `out_hex` → 写回 socket），半包/粘包在 io 侧注入。noise 加密全链见兄弟文件
//! `esphome_session_encrypted.rs`（EspHomeApi 3.0.0 的 `start` 只实现设备侧握手
//! ——先读对端 ClientHello 再回 ServerHello，无法充当主动连接的 HA 客户端，
//! 故加密对拍用其同源原语 noise-protocol + noise-rust-crypto 搭建客户端角色；
//! S3t 已证明该原语组合与 EspHomeApi 设备侧真实互通）。

use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::{ensure_fixture, host_with};
use crate::plugins::wasm::WasmHost;

use esphome_native_api::parser::{parse_proto_message, proto_to_vec, ProtoMessage};
use esphome_native_api::proto::{
    BinarySensorStateResponse, HelloRequest, HelloResponse, ListEntitiesBinarySensorResponse,
    ListEntitiesNumberResponse, NumberCommandRequest, NumberStateResponse, SwitchCommandRequest,
};

pub(crate) const PLUGIN: &str = "esphome.official";
/// 会话状态 kv 前缀（与 guest `session_event` 同口径）。
pub(crate) const STATE_PREFIX: &str = "esphome-session-state";
/// 会话侧 `server_info`（HelloResponse 字段，guest `session::SERVER_INFO` 同值）。
pub(crate) const SERVER_INFO: &str = "xiaozhi:esphome-plugin";

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

pub(crate) fn hex_decode(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex"))
        .collect()
}

pub(crate) fn host() -> WasmHost {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load(PLUGIN, &ensure_fixture("esphome"))
        .expect("load esphome.official");
    host
}

/// 派发 `connector.esp-home.session` 并读回结果通道。
pub(crate) fn call_session(host: &WasmHost, payload: Value) -> Value {
    let req_id = payload
        .get("req_id")
        .and_then(Value::as_str)
        .expect("测试载荷自带 req_id")
        .to_string();
    host.handle_event(PLUGIN, "connector.esp-home.session", &payload.to_string())
        .expect("session dispatch");
    let bytes = host
        .kv()
        .get(PLUGIN, &format!("esphome-session:{req_id}"))
        .unwrap_or_else(|| panic!("session 结果未落 kv（{req_id}）"));
    serde_json::from_slice(&bytes).expect("结果应为合法 JSON")
}

/// 断言成功并返回结果（克隆值，便于调用方绑定使用）。
pub(crate) fn ok(value: &Value) -> Value {
    assert_eq!(value["ok"], json!(true), "期望成功：{value}");
    value.clone()
}

pub(crate) fn err(value: &Value) -> &str {
    assert_eq!(value["ok"], json!(false), "期望业务性失败：{value}");
    value["error"].as_str().unwrap_or_default()
}

/// 对拍设备身份（ServerHello / HelloResponse / DeviceInfoResponse 共用）。
pub(crate) fn test_identity() -> Value {
    json!({
        "name": "xiaozhi-bridge",
        "mac": "AA:BB:CC:DD:EE:FF",
        "esphome_version": "2026.6.2",
        "model": "xiaozhi-server",
        "manufacturer": "xiaozhi",
        "friendly_name": "小智桥",
        "suggested_area": "客厅",
    })
}

/// 对拍实体目录：binary_sensor + number 各一（≤64；key 静态索引）。
pub(crate) fn test_entities() -> Value {
    json!([
        {"key": 1, "kind": "binary_sensor", "object_id": "door", "name": "门磁",
         "device_class": "door", "state": false},
        {"key": 2, "kind": "number", "object_id": "volume", "name": "音量",
         "min_value": 0.0, "max_value": 100.0, "step": 5.0,
         "unit_of_measurement": "%", "mode": 1, "state": 30.0},
    ])
}

// ==================================================================
// 明文帧边界（与参考 FrameCodec 明文分支同口径：0x00 + varint(载荷长) + type）
// ==================================================================

pub(crate) fn varint_of(buf: &[u8]) -> (usize, usize) {
    let mut value = 0usize;
    let mut shift = 0;
    for (i, &b) in buf.iter().enumerate() {
        value |= usize::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return (value, i + 1);
        }
        shift += 7;
    }
    panic!("varint 不完整");
}

fn varint_encode(mut value: usize, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// 客户端手组明文帧：`0x00 ‖ varint(载荷长度) ‖ type ‖ payload`。
pub(crate) fn wire_frame(type_id: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x00];
    varint_encode(payload.len(), &mut frame);
    frame.push(type_id);
    frame.extend_from_slice(payload);
    frame
}

/// 切分插件回包字节流 → [(类型, 载荷)]（布局与 guest `frame_encode` 同口径）。
pub(crate) fn split_frames(buf: Vec<u8>) -> Vec<(u8, Vec<u8>)> {
    let mut frames = Vec::new();
    let mut rest: &[u8] = &buf;
    while !rest.is_empty() {
        let (len, used) = varint_of(&rest[1..]);
        let total = 1 + used + len + 1; // +1 = 类型字节（解码器侧还原，同参考实现）
        assert!(rest.len() >= total, "帧不完整（{} < {total}）", rest.len());
        let type_id = rest[1 + used];
        let payload = rest[2 + used..total].to_vec();
        frames.push((type_id, payload));
        rest = &rest[total..];
    }
    frames
}

/// 泵一轮：读客户端本轮到达的字节（半包/粘包原样透传）→ `feed` → `out_hex` 写回。
///
/// 读侧先等首个可读（2s 超时护栏），再合并 20ms 内已到达的后续字节；不做协议
/// 解析——重组语义由插件（guest 流式缓冲）承担，这正是被测面。
pub(crate) async fn pump(
    host: &WasmHost,
    sock: &mut tokio::net::TcpStream,
    session: &str,
    req_id: &str,
) -> Value {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let n = tokio::time::timeout(
        std::time::Duration::from_millis(2_000),
        sock.read(&mut chunk),
    )
    .await
    .unwrap_or_else(|_| panic!("读客户端字节超时（客户端未发数据）session={session} step={req_id}"))
    .expect("socket 读失败");
    assert!(n > 0, "客户端连接提前关闭（读到 EOF）");
    buf.extend_from_slice(&chunk[..n]);
    loop {
        match tokio::time::timeout(std::time::Duration::from_millis(20), sock.read(&mut chunk))
            .await
        {
            Ok(Ok(m)) if m > 0 => buf.extend_from_slice(&chunk[..m]),
            _ => break,
        }
    }
    let feed = call_session(
        host,
        json!({
            "req_id": req_id,
            "op": "feed",
            "session": session,
            "data_hex": hex_encode(&buf),
        }),
    );
    let out = hex_decode(feed["out_hex"].as_str().unwrap_or_default());
    if !out.is_empty() {
        sock.write_all(&out).await.expect("写会话回包");
    }
    feed
}

// ==================================================================
// ① 明文全链（真 TCP + 半包/粘包注入 + 逐字节对拍 + fail-closed 收尾）
// ==================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plaintext_responder_full_chain_over_real_tcp() {
    let host = host();
    let session = "p1";

    // —— open（明文）：无 psk_hex，无出站帧，状态落插件隔离 kv。
    let open = ok(&call_session(
        &host,
        json!({
            "req_id": "p1-open",
            "op": "open",
            "session": session,
            "identity": test_identity(),
            "entities": test_entities(),
        }),
    ));
    assert_eq!(open["mode"], json!("plaintext"));
    assert_eq!(open["phase"], json!("ready"));
    assert_eq!(open["out_hex"], json!(""), "明文 open 不产出帧");
    assert!(
        host.kv()
            .get(PLUGIN, &format!("{STATE_PREFIX}:{session}"))
            .is_some(),
        "open 后会话状态应落 kv"
    );

    // —— 真 TCP：客户端（HA 角色）连入。
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let mut connect = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("客户端 connect");
    let (mut sock, _) = listener.accept().await.expect("accept");

    // —— HelloRequest **半包**两写：第一半不产出帧，第二半补齐后回 HelloResponse。
    let hello_payload = proto_to_vec(&ProtoMessage::HelloRequest(HelloRequest {
        client_info: "home-assistant".to_string(),
        api_version_major: 1,
        api_version_minor: 10,
    }))
    .expect("prost 编码 HelloRequest");
    let hello_frame = wire_frame(1, &hello_payload);
    connect
        .write_all(&hello_frame[..hello_frame.len() / 2])
        .await
        .expect("写 HelloRequest 前半");
    let feed = pump(&host, &mut sock, session, "p1-hello-a").await;
    assert_eq!(ok(&feed)["phase"], json!("ready"));
    assert_eq!(feed["out_hex"], json!(""), "半包不应产出帧");
    connect
        .write_all(&hello_frame[hello_frame.len() / 2..])
        .await
        .expect("写 HelloRequest 后半");
    let feed = pump(&host, &mut sock, session, "p1-hello-b").await;
    assert_eq!(feed["messages"][0]["type"], json!(1));
    assert_eq!(
        feed["messages"][0]["hello_request"]["client_info"],
        json!("home-assistant")
    );
    let frames = split_frames(hex_decode(feed["out_hex"].as_str().unwrap()));
    assert_eq!(frames.len(), 1, "HelloRequest 只应回一帧");
    assert_eq!(frames[0].0, 2, "应答类型 = HelloResponse(2)");
    let expected = proto_to_vec(&ProtoMessage::HelloResponse(HelloResponse {
        api_version_major: 1,
        api_version_minor: 10,
        server_info: SERVER_INFO.to_string(),
        name: "xiaozhi-bridge".to_string(),
    }))
    .expect("prost 编码 HelloResponse");
    assert_eq!(
        frames[0].1, expected,
        "HelloResponse 载荷与参考 prost 编码不一致"
    );
    let parsed = parse_proto_message(2, &frames[0].1).expect("参考解析器还原 HelloResponse");
    match parsed {
        ProtoMessage::HelloResponse(h) => {
            assert_eq!(h.api_version_major, 1);
            assert_eq!(h.api_version_minor, 10);
            assert_eq!(h.server_info, SERVER_INFO);
            assert_eq!(h.name, "xiaozhi-bridge");
        }
        other => panic!("期待 HelloResponse，收到 {other:?}"),
    }

    // —— ListEntities + SubscribeStates **粘包**一次写：目录 3 帧 + 全量快照 2 帧。
    let mut glued = wire_frame(11, &[]);
    glued.extend_from_slice(&wire_frame(20, &[]));
    connect.write_all(&glued).await.expect("写目录+订阅粘包");
    let feed = pump(&host, &mut sock, session, "p1-list-sub").await;
    assert_eq!(feed["subscribed"], json!(true), "SubscribeStates 置订阅态");
    let frames = split_frames(hex_decode(feed["out_hex"].as_str().unwrap()));
    let types: Vec<u8> = frames.iter().map(|(t, _)| *t).collect();
    assert_eq!(
        types,
        vec![12, 49, 19, 21, 50],
        "目录 2 实体 + done + 快照 2 帧"
    );
    let bs_def = proto_to_vec(&ProtoMessage::ListEntitiesBinarySensorResponse(
        ListEntitiesBinarySensorResponse {
            object_id: "door".to_string(),
            key: 1,
            name: "门磁".to_string(),
            device_class: "door".to_string(),
            ..Default::default()
        },
    ))
    .expect("prost 编码 binary_sensor 定义");
    let number_def = proto_to_vec(&ProtoMessage::ListEntitiesNumberResponse(
        ListEntitiesNumberResponse {
            object_id: "volume".to_string(),
            key: 2,
            name: "音量".to_string(),
            min_value: 0.0,
            max_value: 100.0,
            step: 5.0,
            unit_of_measurement: "%".to_string(),
            mode: 1,
            ..Default::default()
        },
    ))
    .expect("prost 编码 number 定义");
    assert_eq!(frames[0].1, bs_def, "binary_sensor 定义与 prost 不一致");
    assert_eq!(frames[1].1, number_def, "number 定义与 prost 不一致");
    assert!(
        frames[2].1.is_empty(),
        "ListEntitiesDoneResponse 应为空载荷"
    );
    let bs_snapshot = proto_to_vec(&ProtoMessage::BinarySensorStateResponse(
        BinarySensorStateResponse {
            key: 1,
            state: false,
            missing_state: false,
            device_id: 0,
        },
    ))
    .expect("prost 编码 binary_sensor 快照");
    let number_snapshot = proto_to_vec(&ProtoMessage::NumberStateResponse(NumberStateResponse {
        key: 2,
        state: 30.0,
        missing_state: false,
        device_id: 0,
    }))
    .expect("prost 编码 number 快照");
    assert_eq!(
        frames[3].1, bs_snapshot,
        "binary_sensor 快照与 prost 不一致"
    );
    assert_eq!(frames[4].1, number_snapshot, "number 快照与 prost 不一致");

    // —— DeviceInfo + Ping 粘包：身份面字段核对（经参考解析器还原）。
    let mut glued = wire_frame(9, &[]);
    glued.extend_from_slice(&wire_frame(7, &[]));
    connect
        .write_all(&glued)
        .await
        .expect("写 device_info+ping 粘包");
    let feed = pump(&host, &mut sock, session, "p1-info-ping").await;
    let frames = split_frames(hex_decode(feed["out_hex"].as_str().unwrap()));
    let types: Vec<u8> = frames.iter().map(|(t, _)| *t).collect();
    assert_eq!(types, vec![10, 8], "DeviceInfoResponse + PingResponse");
    assert!(frames[1].1.is_empty(), "PingResponse 应为空载荷");
    let parsed = parse_proto_message(10, &frames[0].1).expect("参考解析器还原 DeviceInfoResponse");
    match parsed {
        ProtoMessage::DeviceInfoResponse(info) => {
            assert_eq!(info.name, "xiaozhi-bridge");
            assert_eq!(info.mac_address, "AA:BB:CC:DD:EE:FF");
            assert_eq!(info.esphome_version, "2026.6.2");
            assert_eq!(info.model, "xiaozhi-server");
            assert_eq!(info.manufacturer, "xiaozhi");
            assert_eq!(info.friendly_name, "小智桥");
            assert_eq!(info.suggested_area, "客厅");
            assert!(!info.api_encryption_supported, "明文会话不应声明支持加密");
        }
        other => panic!("期待 DeviceInfoResponse，收到 {other:?}"),
    }

    // —— state op 推送状态变化：订阅态下产出状态帧（客户端收到的即写回的字节）。
    let state = ok(&call_session(
        &host,
        json!({
            "req_id": "p1-state",
            "op": "state",
            "session": session,
            "updates": [{"key": 1, "state": true}, {"key": 2, "state": 45.0}],
        }),
    ));
    let frames = split_frames(hex_decode(state["out_hex"].as_str().unwrap()));
    let types: Vec<u8> = frames.iter().map(|(t, _)| *t).collect();
    assert_eq!(types, vec![21, 50], "两条状态更新各一帧");
    let bs_on = proto_to_vec(&ProtoMessage::BinarySensorStateResponse(
        BinarySensorStateResponse {
            key: 1,
            state: true,
            missing_state: false,
            device_id: 0,
        },
    ))
    .expect("prost 编码 binary_sensor 状态");
    let number_new = proto_to_vec(&ProtoMessage::NumberStateResponse(NumberStateResponse {
        key: 2,
        state: 45.0,
        missing_state: false,
        device_id: 0,
    }))
    .expect("prost 编码 number 状态");
    assert_eq!(frames[0].1, bs_on, "binary_sensor 状态推送与 prost 不一致");
    assert_eq!(frames[1].1, number_new, "number 状态推送与 prost 不一致");

    // —— 命令类只解码不回包：NumberCommand + SwitchCommand 粘包一次写。
    let number_cmd = proto_to_vec(&ProtoMessage::NumberCommandRequest(NumberCommandRequest {
        key: 2,
        state: 60.0,
        device_id: 0,
    }))
    .expect("prost 编码 NumberCommandRequest");
    let switch_cmd = proto_to_vec(&ProtoMessage::SwitchCommandRequest(SwitchCommandRequest {
        key: 0x1234_5678,
        state: true,
        device_id: 1,
    }))
    .expect("prost 编码 SwitchCommandRequest");
    let mut glued = wire_frame(51, &number_cmd);
    glued.extend_from_slice(&wire_frame(33, &switch_cmd));
    connect.write_all(&glued).await.expect("写命令粘包");
    let feed = pump(&host, &mut sock, session, "p1-commands").await;
    assert_eq!(
        feed["out_hex"],
        json!(""),
        "命令类消息不应回包（数据面仍 native）"
    );
    assert_eq!(feed["messages"][0]["type"], json!(51));
    assert_eq!(feed["messages"][0]["number_command"]["key"], json!(2));
    assert_eq!(feed["messages"][0]["number_command"]["state"], json!(60.0));
    assert_eq!(feed["messages"][0]["number_command"]["device_id"], json!(0));
    assert_eq!(feed["messages"][1]["type"], json!(33));
    assert_eq!(
        feed["messages"][1]["switch_command"]["key"],
        json!(0x1234_5678u32)
    );
    assert_eq!(feed["messages"][1]["switch_command"]["state"], json!(true));
    assert_eq!(feed["messages"][1]["switch_command"]["device_id"], json!(1));

    // —— Disconnect：回 DisconnectResponse 并置 closed（状态 kv 仍在，closed=true）。
    connect
        .write_all(&wire_frame(5, &[]))
        .await
        .expect("写 Disconnect");
    let feed = pump(&host, &mut sock, session, "p1-disconnect").await;
    assert_eq!(feed["closed"], json!(true), "DisconnectRequest 关闭会话");
    let frames = split_frames(hex_decode(feed["out_hex"].as_str().unwrap()));
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].0, 6, "应答类型 = DisconnectResponse(6)");
    assert!(frames[0].1.is_empty());
    let state_bytes = host
        .kv()
        .get(PLUGIN, &format!("{STATE_PREFIX}:{session}"))
        .expect("closed 状态仍在 kv");
    let state_value: Value = serde_json::from_slice(&state_bytes).expect("状态 JSON");
    assert_eq!(state_value["closed"], json!(true));

    // —— fail-closed：关闭后继续 feed → 显式失败并销毁状态 → 再 feed 报会话不存在。
    connect
        .write_all(&wire_frame(7, &[]))
        .await
        .expect("写 Ping");
    let feed = pump(&host, &mut sock, session, "p1-after-close").await;
    assert!(
        err(&feed).contains("会话已关闭"),
        "closed 会话的 feed 应显式失败：{feed}"
    );
    assert!(
        host.kv()
            .get(PLUGIN, &format!("{STATE_PREFIX}:{session}"))
            .is_none(),
        "协议失败后状态应销毁"
    );
    connect
        .write_all(&wire_frame(7, &[]))
        .await
        .expect("再写 Ping");
    let feed = pump(&host, &mut sock, session, "p1-gone").await;
    assert!(err(&feed).contains("会话不存在"), "{feed}");
    let close = ok(&call_session(
        &host,
        json!({"req_id": "p1-close", "op": "close", "session": session}),
    ));
    assert_eq!(close["removed"], json!(false), "状态已销毁，close 无可删");

    // S3n 教训：对端应答后保持连接不立即 drop（避免 RST 丢未读字节造成假失败）。
    drop(connect);
    drop(sock);
}
