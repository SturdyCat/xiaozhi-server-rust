//! S3k：esphome.official 明文帧编解码 **esphome-native-api 真连接对拍**。
//!
//! 参考侧 = 外部 crate 本尊（`EspHomeApi::start` 跑在内存 duplex 上，无加密 →
//! 明文帧路径）：guest 编码的帧被真实握手消费、真连接发出的帧被 guest 解码、
//! 真连接发出的 PingResponse 帧与 guest 输出**逐字节对拍**。帧内层 protobuf 另经
//! 公开 `parser::proto_to_vec` 与 guest 手写编码对拍（prost 声明序 + 默认值跳过）。
//! noise/加密帧不在本片范围（guest 遇 `0x01` 显式报错）。

use serde_json::{json, Value};
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;

use super::{ensure_fixture, host_with};
use crate::plugins::wasm::WasmHost;

const PLUGIN: &str = "esphome.official";
const FRAME_PREFIX_KEY: &str = "esphome-frame";

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(feature = "esp_home")]
fn hex_decode(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex"))
        .collect()
}

fn host() -> WasmHost {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load(PLUGIN, &ensure_fixture("esphome"))
        .expect("load esphome.official");
    host
}

fn call_frame(host: &WasmHost, topic: &str, payload: Value, req_id: &str) -> Value {
    host.handle_event(PLUGIN, topic, &payload.to_string())
        .expect("frame dispatch");
    let bytes = host
        .kv()
        .get(PLUGIN, &format!("{FRAME_PREFIX_KEY}:{req_id}"))
        .unwrap_or_else(|| panic!("帧结果未落 kv（{req_id}）"));
    serde_json::from_slice(&bytes).expect("结果应为合法 JSON")
}

/// 参考帧边界解析（frame.rs 明文分支同口径）：返回 (整帧字节, 载荷)。
#[cfg(feature = "esp_home")]
fn frame_boundary(buf: &[u8]) -> Option<(usize, Vec<u8>)> {
    if buf.first() != Some(&0x00) {
        return None;
    }
    let mut i = 1;
    loop {
        if buf.len() <= i {
            return None; // varint 未完整
        }
        if buf[i] & 0x80 == 0 {
            break;
        }
        i += 1;
        if i > 4 {
            return None; // 参考实现同款守卫
        }
    }
    let mut len: usize = 0;
    let mut shift = 0;
    for &b in &buf[1..=i] {
        len |= usize::from(b & 0x7f) << shift;
        shift += 7;
    }
    // 参考实现解码器侧 +1：wire 标记 = 载荷长度，帧体 = 类型字节 + 载荷。
    let total = 1 + i + len + 1;
    if buf.len() < total {
        return None;
    }
    Some((total, buf[..total].to_vec()))
}

/// 从 duplex 读一帧（增量读 + 边界判断），2s 超时护栏。
#[cfg(feature = "esp_home")]
async fn read_one_frame(reader: &mut (impl tokio::io::AsyncRead + Unpin)) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        if let Some((_consumed, frame)) = frame_boundary(&buf) {
            return frame;
        }
        let mut chunk = [0u8; 256];
        let n = tokio::time::timeout(std::time::Duration::from_secs(2), reader.read(&mut chunk))
            .await
            .expect("读帧超时（参考侧未发帧）")
            .expect("duplex 读失败");
        assert!(n > 0, "参考侧流提前关闭");
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// 与 esphome.official 建一条**真实 ESPHome API 连接**（无加密 → 明文路径）：
/// guest 的 hello_request 帧作为客户端首帧写入，参考侧（设备角色）消费后回
/// HelloResponse。返回 (连接句柄, 服务端侧 duplex)。
#[cfg(feature = "esp_home")]
async fn open_real_connection(
    host: &WasmHost,
) -> (
    esphome_native_api::connection::Connection,
    tokio::io::DuplexStream,
) {
    use esphome_native_api::esphomeapi::EspHomeApi;

    let (client_side, mut server_side) = tokio::io::duplex(4096);
    let api = EspHomeApi::builder()
        .name("esphome-satellite".to_string())
        .api_version_major(1)
        .api_version_minor(10)
        .build()
        .expect("api build");
    let task = tokio::spawn(async move { api.start(client_side).await });

    // guest 编码握手首帧（HelloRequest）并写入真连接。
    let hello = call_frame(
        host,
        "connector.esp-home.frame-pack",
        json!({
            "req_id": "hello",
            "message": "hello_request",
            "client_info": "xiaozhi-bridge",
            "api_version_major": 1,
            "api_version_minor": 10,
        }),
        "hello",
    );
    assert_eq!(hello["ok"], json!(true), "{hello}");

    server_side
        .write_all(&hex_decode(hello["frame_hex"].as_str().unwrap()))
        .await
        .expect("写 hello 帧");

    let connection = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .expect("握手超时")
        .expect("连接任务崩溃")
        .expect("参考侧握手失败（guest 帧未被真实解析器接受）");
    (connection, server_side)
}

/// 帧内层 protobuf 对拍：guest 手写载荷 ↔ 参考公开 parser 的 prost 编码。
#[test]
#[cfg(feature = "esp_home")]
fn guest_hello_payload_matches_prost_reference() {
    use esphome_native_api::parser::{proto_to_vec, ProtoMessage};
    use esphome_native_api::proto::HelloRequest;

    let reference = proto_to_vec(&ProtoMessage::HelloRequest(HelloRequest {
        client_info: "xiaozhi-bridge".to_string(),
        api_version_major: 1,
        api_version_minor: 10,
    }))
    .expect("proto_to_vec");

    let host = host();
    let out = call_frame(
        &host,
        "connector.esp-home.frame-pack",
        json!({
            "req_id": "p1",
            "message": "hello_request",
            "client_info": "xiaozhi-bridge",
            "api_version_major": 1,
            "api_version_minor": 10,
        }),
        "p1",
    );
    let frame = hex_decode(out["frame_hex"].as_str().unwrap());
    // 布局：0x00 + varint(len+1) + type(1) + payload → 载荷 = 去 2 字节头与类型。
    assert_eq!(frame[0], 0x00);
    assert_eq!(frame[2], 1, "类型字节应为 HelloRequest(1)");
    assert_eq!(
        &frame[3..],
        &reference[..],
        "HelloRequest 载荷与 prost 不一致"
    );
}

#[tokio::test]
#[cfg(feature = "esp_home")]
async fn real_handshake_consumes_guest_frames_and_guest_parses_real_hello_response() {
    let host = host();
    let (connection, mut server_side) = open_real_connection(&host).await;

    // 方向一（已验证）：参考侧接受了 guest 的 hello_request 帧（握手成功）。
    // 方向二：参考侧发出的 HelloResponse 帧被 guest 解码回读。
    let mut rx = connection.receiver();
    let hello_response = read_one_frame(&mut server_side).await;
    let out = call_frame(
        &host,
        "connector.esp-home.frame-parse",
        json!({"req_id": "d1", "data_hex": hex_encode(&hello_response)}),
        "d1",
    );
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["type"], json!(2));
    assert_eq!(out["message"], json!("hello_response"));
    let hello = &out["hello"];
    assert_eq!(hello["api_version_major"], json!(1));
    assert_eq!(hello["api_version_minor"], json!(10));
    assert_eq!(
        hello["server_info"],
        json!("Rust: esphome-native-api"),
        "server_info 应为参考侧默认值"
    );
    assert_eq!(hello["name"], json!("esphome-satellite"));
    let _ = rx; // receiver 订阅保持连接活跃
}

#[tokio::test]
#[cfg(feature = "esp_home")]
async fn guest_ping_response_frame_matches_real_emit_byte_for_byte() {
    use esphome_native_api::parser::ProtoMessage;
    use esphome_native_api::proto::PingResponse;

    let host = host();
    let (connection, mut server_side) = open_real_connection(&host).await;

    // 排空握手期参考侧已写入的 HelloResponse 帧（协议顺序：先 hello 后业务帧）。
    let _hello_frame = read_one_frame(&mut server_side).await;

    // 真连接经参考 FrameCodec 发出 PingResponse —— 拿到执行态参考帧。
    connection
        .sender()
        .send(ProtoMessage::PingResponse(PingResponse {}))
        .await
        .expect("send ping_response");
    let reference = read_one_frame(&mut server_side).await;

    let out = call_frame(
        &host,
        "connector.esp-home.frame-pack",
        json!({"req_id": "d2", "message": "ping_response"}),
        "d2",
    );
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["type"], json!(8));
    let guest = hex_decode(out["frame_hex"].as_str().unwrap());
    assert_eq!(
        guest, reference,
        "guest PingResponse 帧与参考实现真实输出不一致"
    );
}

#[tokio::test]
#[cfg(feature = "esp_home")]
async fn real_side_consumes_guest_switch_command() {
    use esphome_native_api::parser::ProtoMessage;
    use esphome_native_api::proto::SwitchCommandRequest;

    let host = host();
    let (connection, mut server_side) = open_real_connection(&host).await;
    let mut rx = connection.receiver();

    // 排空握手期 HelloResponse（协议顺序）后再发业务帧。
    let _hello_frame = read_one_frame(&mut server_side).await;

    // guest 编码 SwitchCommandRequest 写入真连接 → 参考侧广播应还原同值。
    let key: u32 = 0x1234_5678;
    let out = call_frame(
        &host,
        "connector.esp-home.frame-pack",
        json!({
            "req_id": "d3",
            "message": "switch_command_request",
            "key": key,
            "state": true,
            "device_id": 1,
        }),
        "d3",
    );
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["type"], json!(33));

    server_side
        .write_all(&hex_decode(out["frame_hex"].as_str().unwrap()))
        .await
        .expect("写 switch 帧");

    let msg = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("等广播超时")
        .expect("广播通道关闭");
    match msg {
        ProtoMessage::SwitchCommandRequest(req) => {
            assert_eq!(req.key, key);
            assert_eq!(req.state, true);
            assert_eq!(req.device_id, 1);
        }
        other => panic!("期待 SwitchCommandRequest，收到 {other:?}"),
    }
}

#[test]
#[cfg(feature = "esp_home")]
fn frame_parse_rejects_encrypted_marker_and_garbage() {
    let host = host();
    let out = call_frame(
        &host,
        "connector.esp-home.frame-parse",
        json!({"req_id": "n1", "data_hex": "010020"}),
        "n1",
    );
    assert_eq!(out["ok"], json!(false), "{out}");
    assert!(
        out["error"].as_str().unwrap_or_default().contains("加密"),
        "应显式报加密帧越界：{out}"
    );
    let out = call_frame(
        &host,
        "connector.esp-home.frame-parse",
        json!({"req_id": "n2", "data_hex": "02010203"}),
        "n2",
    );
    assert_eq!(out["ok"], json!(false), "{out}");
    assert!(
        out["error"]
            .as_str()
            .unwrap_or_default()
            .contains("非法起始字节"),
        "应显式报非法起始字节：{out}"
    );
}

// Arc 供 host() 使用（与其它测试文件一致的导入面）。
use std::sync::Arc;

// ==================================================================
// S3r：服务端面消息（实体目录 / 状态 / 控制应答）与参考 prost 编解码对拍。
// 覆盖 native `esp_home` 桥实际使用的消息全集；参考侧直接用 crate 本尊的
// `proto_to_vec`（prost 编码）与 `parse_proto_message`（公开解析器）。
// ==================================================================

#[cfg(feature = "esp_home")]
fn varint_of(buf: &[u8]) -> (usize, usize) {
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

#[cfg(feature = "esp_home")]
fn varint_encode(mut value: usize, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// 按 wire 布局手工组帧：`0x00 || varint(payload_len) || type || payload`。
#[cfg(feature = "esp_home")]
fn wire_frame(type_id: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x00];
    varint_encode(payload.len(), &mut frame);
    frame.push(type_id);
    frame.extend_from_slice(payload);
    frame
}

/// 从 guest 帧取 (类型, 载荷)；布局与 guest `frame_encode` 同口径。
#[cfg(feature = "esp_home")]
fn split_guest_frame(frame: &[u8]) -> (u8, Vec<u8>) {
    assert_eq!(frame[0], 0x00, "明文帧起始字节");
    let (len, used) = varint_of(&frame[1..]);
    let type_at = 1 + used;
    let payload = &frame[type_at + 1..];
    assert_eq!(payload.len(), len, "帧声明长度应与载荷一致");
    (frame[type_at], payload.to_vec())
}

#[cfg(feature = "esp_home")]
fn reference_server_cases() -> Vec<(Value, esphome_native_api::parser::ProtoMessage)> {
    use esphome_native_api::parser::ProtoMessage;
    use esphome_native_api::proto::{
        BinarySensorStateResponse, HelloResponse, ListEntitiesBinarySensorResponse,
        ListEntitiesDoneResponse, ListEntitiesNumberResponse, NumberStateResponse, PingResponse,
    };

    vec![
        (
            json!({"message": "hello_response", "api_version_major": 1, "api_version_minor": 10,
                   "server_info": "Rust: esphome-native-api", "name": "xiaozhi-bridge"}),
            ProtoMessage::HelloResponse(HelloResponse {
                api_version_major: 1,
                api_version_minor: 10,
                server_info: "Rust: esphome-native-api".to_string(),
                name: "xiaozhi-bridge".to_string(),
            }),
        ),
        (
            json!({"message": "ping_response"}),
            ProtoMessage::PingResponse(PingResponse {}),
        ),
        (
            json!({"message": "list_entities_binary_sensor_response", "object_id": "volume_mute",
                   "key": 7, "name": "静音", "device_class": "mute"}),
            ProtoMessage::ListEntitiesBinarySensorResponse(ListEntitiesBinarySensorResponse {
                object_id: "volume_mute".to_string(),
                key: 7,
                name: "静音".to_string(),
                device_class: "mute".to_string(),
                ..Default::default()
            }),
        ),
        (
            // min 用 0.0（默认值跳过）+ 其余非零，覆盖 prost 声明序与默认跳过。
            json!({"message": "list_entities_number_response", "object_id": "volume",
                   "key": 3, "name": "音量", "min_value": 0.0, "max_value": 1.0,
                   "step": 0.25, "unit_of_measurement": "", "mode": 1}),
            ProtoMessage::ListEntitiesNumberResponse(ListEntitiesNumberResponse {
                object_id: "volume".to_string(),
                key: 3,
                name: "音量".to_string(),
                min_value: 0.0,
                max_value: 1.0,
                step: 0.25,
                unit_of_measurement: String::new(),
                mode: 1,
                ..Default::default()
            }),
        ),
        (
            json!({"message": "binary_sensor_state_response", "key": 7, "state": true,
                   "missing_state": false, "device_id": 0}),
            ProtoMessage::BinarySensorStateResponse(BinarySensorStateResponse {
                key: 7,
                state: true,
                missing_state: false,
                device_id: 0,
            }),
        ),
        (
            json!({"message": "number_state_response", "key": 3, "state": 0.75,
                   "missing_state": false, "device_id": 0}),
            ProtoMessage::NumberStateResponse(NumberStateResponse {
                key: 3,
                state: 0.75,
                missing_state: false,
                device_id: 0,
            }),
        ),
        (
            json!({"message": "list_entities_done_response"}),
            ProtoMessage::ListEntitiesDoneResponse(ListEntitiesDoneResponse {}),
        ),
    ]
}

#[cfg(feature = "esp_home")]
fn assert_reference_matches(
    msg: &esphome_native_api::parser::ProtoMessage,
    expect: &esphome_native_api::parser::ProtoMessage,
) {
    use esphome_native_api::parser::ProtoMessage;
    match (msg, expect) {
        (ProtoMessage::HelloResponse(a), ProtoMessage::HelloResponse(b)) => assert_eq!(a, b),
        (ProtoMessage::PingResponse(_), ProtoMessage::PingResponse(_)) => {}
        (
            ProtoMessage::ListEntitiesBinarySensorResponse(a),
            ProtoMessage::ListEntitiesBinarySensorResponse(b),
        ) => assert_eq!(a, b),
        (
            ProtoMessage::ListEntitiesNumberResponse(a),
            ProtoMessage::ListEntitiesNumberResponse(b),
        ) => assert_eq!(a, b),
        (
            ProtoMessage::BinarySensorStateResponse(a),
            ProtoMessage::BinarySensorStateResponse(b),
        ) => assert_eq!(a, b),
        (ProtoMessage::NumberStateResponse(a), ProtoMessage::NumberStateResponse(b)) => {
            assert_eq!(a, b)
        }
        (ProtoMessage::ListEntitiesDoneResponse(_), ProtoMessage::ListEntitiesDoneResponse(_)) => {}
        (other, expect) => panic!("参考解析结果与预期变体不一致：{other:?} vs {expect:?}"),
    }
}

#[test]
#[cfg(feature = "esp_home")]
fn guest_server_payloads_match_prost_reference_byte_for_byte() {
    use esphome_native_api::parser::proto_to_vec;

    let host = host();
    for (i, (payload, expected)) in reference_server_cases().into_iter().enumerate() {
        let rid = format!("s{i}");
        let mut request = payload;
        request["req_id"] = json!(rid);
        let out = call_frame(&host, "connector.esp-home.frame-pack", request, &rid);
        assert_eq!(out["ok"], json!(true), "{out}");
        let frame = hex_decode(out["frame_hex"].as_str().unwrap());
        let (type_id, guest_payload) = split_guest_frame(&frame);
        let reference = proto_to_vec(&expected).expect("proto_to_vec");
        assert_eq!(
            type_id,
            out["type"].as_u64().unwrap() as u8,
            "类型字节应与结果一致"
        );
        assert_eq!(
            guest_payload, reference,
            "第 {i} 条服务端消息负载与参考 prost 编码不一致"
        );
    }
}

#[test]
#[cfg(feature = "esp_home")]
fn reference_parser_consumes_guest_server_payloads() {
    use esphome_native_api::parser::parse_proto_message;

    let host = host();
    for (i, (payload, expected)) in reference_server_cases().into_iter().enumerate() {
        let rid = format!("r{i}");
        let mut request = payload;
        request["req_id"] = json!(rid);
        let out = call_frame(&host, "connector.esp-home.frame-pack", request, &rid);
        assert_eq!(out["ok"], json!(true), "{out}");
        let frame = hex_decode(out["frame_hex"].as_str().unwrap());
        let (type_id, guest_payload) = split_guest_frame(&frame);
        let parsed = parse_proto_message(type_id as usize, &guest_payload)
            .expect("参考解析器应接受 guest 服务端消息载荷");
        assert_reference_matches(&parsed, &expected);
    }
}

#[test]
#[cfg(feature = "esp_home")]
fn guest_decodes_client_messages_from_reference_bytes() {
    use esphome_native_api::parser::{proto_to_vec, ProtoMessage};
    use esphome_native_api::proto::{HelloRequest, NumberCommandRequest, SwitchCommandRequest};

    let host = host();
    // HelloRequest：参考 prost 编码 → 手组明文帧 → guest 解码回字段。
    let hello_payload = proto_to_vec(&ProtoMessage::HelloRequest(HelloRequest {
        client_info: "home-assistant".to_string(),
        api_version_major: 1,
        api_version_minor: 10,
    }))
    .expect("proto_to_vec");
    let out = call_frame(
        &host,
        "connector.esp-home.frame-parse",
        json!({"req_id": "c1", "data_hex": hex_encode(&wire_frame(1, &hello_payload))}),
        "c1",
    );
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["hello_request"]["client_info"], json!("home-assistant"));
    assert_eq!(out["hello_request"]["api_version_major"], json!(1));

    // NumberCommandRequest（HA 拖音量滑块的真实方向）。
    let number_payload = proto_to_vec(&ProtoMessage::NumberCommandRequest(NumberCommandRequest {
        key: 3,
        state: 0.75,
        device_id: 0,
    }))
    .expect("proto_to_vec");
    let out = call_frame(
        &host,
        "connector.esp-home.frame-parse",
        json!({"req_id": "c2", "data_hex": hex_encode(&wire_frame(51, &number_payload))}),
        "c2",
    );
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["number_command"]["key"], json!(3));
    assert_eq!(out["number_command"]["state"], json!(0.75));
    assert_eq!(out["number_command"]["device_id"], json!(0));

    // SwitchCommandRequest（客户端命令方向，与既有 frame-pack 编码对称）。
    let switch_payload = proto_to_vec(&ProtoMessage::SwitchCommandRequest(SwitchCommandRequest {
        key: 0x1234_5678,
        state: true,
        device_id: 1,
    }))
    .expect("proto_to_vec");
    let out = call_frame(
        &host,
        "connector.esp-home.frame-parse",
        json!({"req_id": "c3", "data_hex": hex_encode(&wire_frame(33, &switch_payload))}),
        "c3",
    );
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["switch_command"]["key"], json!(0x1234_5678));
    assert_eq!(out["switch_command"]["state"], json!(true));
    assert_eq!(out["switch_command"]["device_id"], json!(1));
}
