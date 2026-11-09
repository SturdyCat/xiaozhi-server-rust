//! S3t：esphome.official **Noise NNpsk0 加密会话** 对拍测试。
//!
//! 参考侧有两层，都是独立实现：
//! 1. `noise-protocol` + `noise-rust-crypto`（ESPHome Rust 设备栈 `esphome-native-api`
//!    的同源参考实现）——逐字节对拍 guest 的 ClientHello/msg1/msg2、ServerHello 解析与
//!    AEAD 传输帧（双向），覆盖 initiator / responder 两个角色；
//! 2. `esphome-native-api` 本尊的**真实加密连接**（`EspHomeApi` + base64 encryption_key，
//!    内存 duplex）：guest 做 initiator 完成真实握手并收发加密 protobuf 消息，证明线格式
//!    与 ESPHome 设备实现互通（错误 psk 场景按 aioesphomeapi 口径识别
//!    `Handshake MAC failure` 错误帧）。
//!
//! 事件面：`connector.esp-home.noise`（op = start/step/seal/open/close），结果通道
//! `esphome-noise:<req_id>`，会话状态在插件隔离 kv（事件间无内存态）。

use serde_json::{json, Value};
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;

use super::{ensure_fixture, host_with};
use crate::plugins::wasm::WasmHost;

// Arc 供 host() 使用（与其它测试文件一致的导入面）。
use std::sync::Arc;

const PLUGIN: &str = "esphome.official";
const RESULT_PREFIX: &str = "esphome-noise";
/// 与 ESPHome 固件一致的 prologue（aioesphomeapi `b"NoiseAPIInit\0\0"`）。
const PROLOGUE: &[u8] = b"NoiseAPIInit\0\0";
const PSK: [u8; 32] = [0x42; 32];

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

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

/// 派发 `connector.esp-home.noise` 并读回结果通道。
fn call_noise(host: &WasmHost, payload: Value) -> Value {
    let req_id = payload
        .get("req_id")
        .and_then(Value::as_str)
        .expect("测试载荷自带 req_id")
        .to_string();
    host.handle_event(PLUGIN, "connector.esp-home.noise", &payload.to_string())
        .expect("noise dispatch");
    let bytes = host
        .kv()
        .get(PLUGIN, &format!("{RESULT_PREFIX}:{req_id}"))
        .unwrap_or_else(|| panic!("noise 结果未落 kv（{req_id}）"));
    serde_json::from_slice(&bytes).expect("结果应为合法 JSON")
}

fn ok(value: &Value) -> &Value {
    assert_eq!(value["ok"], json!(true), "期望成功：{value}");
    value
}

fn err(value: &Value) -> &str {
    assert_eq!(value["ok"], json!(false), "期望业务性失败：{value}");
    value["error"].as_str().unwrap_or_default()
}

// ==================================================================
// 参考侧（noise-protocol 本尊）
// ==================================================================

/// 参考 HandshakeState 别名（ESPHome 会话参数：NNpsk0 + X25519 + ChaChaPoly + SHA256）。
type RefState = noise_protocol::HandshakeState<
    noise_rust_crypto::X25519,
    noise_rust_crypto::ChaCha20Poly1305,
    noise_rust_crypto::Sha256,
>;

/// 建参考握手：显式临时私钥（与 guest 用固定值对拍，测试可复现）。
fn reference_state(initiator: bool, ephemeral: [u8; 32], psk: &[u8; 32]) -> RefState {
    use noise_protocol::patterns::noise_nn_psk0;
    let seed =
        <noise_rust_crypto::sensitive::Sensitive<[u8; 32]> as noise_protocol::U8Array>::from_slice(
            &ephemeral,
        );
    let mut state = RefState::new(
        noise_nn_psk0(),
        initiator,
        PROLOGUE,
        None,
        Some(seed),
        None,
        None,
    );
    state.push_psk(psk);
    state
}

/// 加密帧：`0x01 ‖ u16 BE(len) ‖ content`。
fn encrypted_frame(content: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x01, (content.len() >> 8) as u8, content.len() as u8];
    frame.extend_from_slice(content);
    frame
}

/// 握手帧：内容 = `0x00 ‖ noise 消息`。
fn handshake_frame(message: &[u8]) -> Vec<u8> {
    let mut content = vec![0x00];
    content.extend_from_slice(message);
    encrypted_frame(&content)
}

/// ServerHello 帧（参考设备侧布局：`0x01 ‖ name ‖ 00 [‖ mac ‖ 00]`）。
fn server_hello_frame(name: &str, mac: Option<&str>) -> Vec<u8> {
    let mut content = vec![0x01];
    content.extend_from_slice(name.as_bytes());
    content.push(0);
    if let Some(mac) = mac {
        content.extend_from_slice(mac.as_bytes());
        content.push(0);
    }
    encrypted_frame(&content)
}

/// 解析加密帧 → 内容（校验标记/长度一致）。
fn frame_content(frame: &[u8]) -> Vec<u8> {
    assert_eq!(frame[0], 0x01, "加密帧标记必须 0x01");
    let len = u16::from_be_bytes([frame[1], frame[2]]) as usize;
    assert_eq!(frame.len(), 3 + len, "加密帧长度字段与实际不一致");
    frame[3..].to_vec()
}

/// 参考 CipherState 把传输明文封装成**加密帧**：`type(u16 BE) ‖ len(u16 BE) ‖ payload`。
fn reference_transport_frame(
    cipher: &mut noise_protocol::CipherState<noise_rust_crypto::ChaCha20Poly1305>,
    message_type: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut plaintext = Vec::with_capacity(4 + payload.len());
    plaintext.extend_from_slice(&message_type.to_be_bytes());
    plaintext.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    plaintext.extend_from_slice(payload);
    encrypted_frame(&cipher.encrypt_vec(&plaintext))
}

// ==================================================================
// initiator（guest）↔ responder（参考）方向
// ==================================================================

#[test]
fn guest_initiator_handshake_and_transport_match_reference_responder() {
    let host = host();
    let ephemeral = [0x11u8; 32];
    let start = call_noise(
        &host,
        json!({
            "req_id": "i-start",
            "op": "start",
            "session": "init-1",
            "role": "initiator",
            "psk_hex": hex_encode(&PSK),
        }),
    );
    let start = ok(&start).clone();
    assert_eq!(start["phase"], json!("await-server-hello"));

    // guest 首包 = ClientHello(`01 00 00`) + msg1 帧；参考 responder 消费 msg1。
    let hello = hex_decode(start["out_hex"].as_str().unwrap());
    assert_eq!(&hello[..3], &[0x01, 0x00, 0x00], "ClientHello 固定 3 字节");
    let msg1_content = frame_content(&hello[3..]);
    assert_eq!(msg1_content[0], 0x00, "握手帧内容首字节 0x00");
    let msg1 = &msg1_content[1..];
    assert_eq!(msg1.len(), 48, "NNpsk0 msg1 = e(32) + tag(16)");

    let mut reference = reference_state(false, ephemeral, &PSK);
    let request = reference
        .read_message_vec(msg1)
        .expect("参考 responder 读 msg1");
    assert!(request.is_empty(), "ESPHome 握手载荷为空");

    // 参考 responder 回 msg2 → 与 guest 的 ServerHello + msg2 拼接后喂回。
    let msg2 = reference
        .write_message_vec(b"")
        .expect("参考 responder 写 msg2");
    let mut inbound = server_hello_frame("esphome-satellite", Some("AA:BB:CC:DD:EE:FF"));
    inbound.extend_from_slice(&handshake_frame(&msg2));
    // 半包路径：先喂前半，再喂后半（流式缓冲必须正确拼接）。
    let split_at = inbound.len() / 2;
    let first = call_noise(
        &host,
        json!({
            "req_id": "i-step-1",
            "op": "step",
            "session": "init-1",
            "data_hex": hex_encode(&inbound[..split_at]),
        }),
    );
    ok(&first);
    assert_eq!(
        first["phase"],
        json!("await-msg2"),
        "ServerHello 先到，msg2 未完整"
    );
    let second = call_noise(
        &host,
        json!({
            "req_id": "i-step-2",
            "op": "step",
            "session": "init-1",
            "data_hex": hex_encode(&inbound[split_at..]),
        }),
    );
    let second = ok(&second).clone();
    assert_eq!(second["phase"], json!("ready"));
    assert_eq!(second["peer_name"], json!("esphome-satellite"));
    assert_eq!(second["peer_mac"], json!("AA:BB:CC:DD:EE:FF"));

    // 传输帧逐字节对拍：guest initiator 发送侧 = 参考 c1（initiator→responder）。
    let (mut c1, mut c2) = reference.get_ciphers();
    use esphome_native_api::parser::{proto_to_vec, ProtoMessage};
    use esphome_native_api::proto::HelloRequest;
    let hello_request = proto_to_vec(&ProtoMessage::HelloRequest(HelloRequest {
        client_info: "xiaozhi-bridge".to_string(),
        api_version_major: 1,
        api_version_minor: 10,
    }))
    .expect("prost 编码 HelloRequest");
    let expected = reference_transport_frame(&mut c1, 1, &hello_request);
    let seal = call_noise(
        &host,
        json!({
            "req_id": "i-seal",
            "op": "seal",
            "session": "init-1",
            "message": "hello_request",
            "client_info": "xiaozhi-bridge",
            "api_version_major": 1,
            "api_version_minor": 10,
        }),
    );
    let seal = ok(&seal).clone();
    assert_eq!(seal["type"], json!(1));
    assert_eq!(
        hex_decode(seal["frame_hex"].as_str().unwrap()),
        expected,
        "guest 加密传输帧与参考实现不一致（initiator→responder）"
    );

    // 反向：参考 c2（responder→initiator）加密 HelloResponse → guest open 解出。
    use esphome_native_api::proto::HelloResponse;
    let hello_response = proto_to_vec(&ProtoMessage::HelloResponse(HelloResponse {
        api_version_major: 1,
        api_version_minor: 10,
        server_info: "Rust: esphome-native-api".to_string(),
        name: "esphome-satellite".to_string(),
    }))
    .expect("prost 编码 HelloResponse");
    let inbound = reference_transport_frame(&mut c2, 2, &hello_response);
    let open = call_noise(
        &host,
        json!({
            "req_id": "i-open",
            "op": "open",
            "session": "init-1",
            "data_hex": hex_encode(&inbound),
        }),
    );
    let open = ok(&open).clone();
    assert_eq!(open["messages"][0]["type"], json!(2));
    assert_eq!(open["messages"][0]["message"], json!("hello_response"));
    assert_eq!(
        open["messages"][0]["hello"]["name"],
        json!("esphome-satellite")
    );
    assert_eq!(
        open["messages"][0]["hello"]["server_info"],
        json!("Rust: esphome-native-api")
    );
}

// ==================================================================
// responder（guest）↔ initiator（参考）方向 —— 本仓 native 桥的真实角色
// ==================================================================

#[test]
fn guest_responder_handshake_and_transport_match_reference_initiator() {
    let host = host();
    let ephemeral = [0x22u8; 32];
    let start = call_noise(
        &host,
        json!({
            "req_id": "r-start",
            "op": "start",
            "session": "resp-1",
            "role": "responder",
            "psk_hex": hex_encode(&PSK),
            "server_name": "xiaozhi-bridge",
            "server_mac": "AA:BB:CC:DD:EE:FF",
        }),
    );
    let start = ok(&start).clone();
    assert_eq!(start["phase"], json!("await-msg1"));

    // ServerHello 布局：`0x01 ‖ name ‖ 00 ‖ mac ‖ 00`（参考实现 generate_server_hello_frame）。
    let hello_content = frame_content(&hex_decode(start["out_hex"].as_str().unwrap()));
    assert_eq!(hello_content[0], 0x01, "ServerHello 首字节 = 加密协议 0x01");
    assert_eq!(
        &hello_content[1..],
        b"xiaozhi-bridge\0AA:BB:CC:DD:EE:FF\0",
        "ServerHello name/mac 布局与参考实现不一致"
    );

    // 参考 initiator 写 msg1（ClientHello 空帧 + 握手帧）；guest responder 回 msg2。
    let mut reference = reference_state(true, ephemeral, &PSK);
    let msg1 = reference
        .write_message_vec(b"")
        .expect("参考 initiator 写 msg1");
    let mut inbound = vec![0x01, 0x00, 0x00];
    inbound.extend_from_slice(&handshake_frame(&msg1));
    let step = call_noise(
        &host,
        json!({
            "req_id": "r-step",
            "op": "step",
            "session": "resp-1",
            "data_hex": hex_encode(&inbound),
        }),
    );
    let step = ok(&step).clone();
    assert_eq!(
        step["phase"],
        json!("ready"),
        "responder 读到 msg1 后应直接 ready"
    );
    let msg2_content = frame_content(&hex_decode(step["out_hex"].as_str().unwrap()));
    assert_eq!(msg2_content[0], 0x00);
    reference
        .read_message_vec(&msg2_content[1..])
        .expect("参考 initiator 读 msg2");
    assert!(reference.completed(), "参考侧握手应完成");

    // 传输帧对拍：guest responder 发送侧 = 参考 c2（responder→initiator）。
    let (mut c1, mut c2) = reference.get_ciphers();
    use esphome_native_api::parser::{proto_to_vec, ProtoMessage};
    use esphome_native_api::proto::BinarySensorStateResponse;
    let state_response = proto_to_vec(&ProtoMessage::BinarySensorStateResponse(
        BinarySensorStateResponse {
            key: 0x1234_5678,
            state: true,
            missing_state: false,
            device_id: 7,
        },
    ))
    .expect("prost 编码 BinarySensorStateResponse");
    let expected = reference_transport_frame(&mut c2, 21, &state_response);
    let seal = call_noise(
        &host,
        json!({
            "req_id": "r-seal",
            "op": "seal",
            "session": "resp-1",
            "message": "binary_sensor_state_response",
            "key": 0x1234_5678u32,
            "state": true,
            "device_id": 7,
        }),
    );
    let seal = ok(&seal).clone();
    assert_eq!(seal["type"], json!(21));
    assert_eq!(
        hex_decode(seal["frame_hex"].as_str().unwrap()),
        expected,
        "guest 加密传输帧与参考实现不一致（responder→initiator）"
    );

    // 反向：参考 c1 加密 NumberCommandRequest → guest responder open 解出命令字段。
    use esphome_native_api::proto::NumberCommandRequest;
    let command = proto_to_vec(&ProtoMessage::NumberCommandRequest(NumberCommandRequest {
        key: 0x0A0B_0C0Du32,
        state: 42.5,
        device_id: 3,
    }))
    .expect("prost 编码 NumberCommandRequest");
    let inbound = reference_transport_frame(&mut c1, 51, &command);
    let open = call_noise(
        &host,
        json!({
            "req_id": "r-open",
            "op": "open",
            "session": "resp-1",
            "data_hex": hex_encode(&inbound),
        }),
    );
    let open = ok(&open).clone();
    assert_eq!(open["messages"][0]["type"], json!(51));
    assert_eq!(
        open["messages"][0]["number_command"]["key"],
        json!(0x0A0B_0C0Du32)
    );
    assert_eq!(open["messages"][0]["number_command"]["device_id"], json!(3));
    assert_eq!(open["messages"][0]["number_command"]["state"], json!(42.5));
}

// ==================================================================
// esphome-native-api 真实加密连接（guest = initiator）
// ==================================================================

/// 从 duplex 读一个加密帧（3 字节头 + 载荷），2s 超时护栏。
async fn read_encrypted_frame(reader: &mut (impl tokio::io::AsyncRead + Unpin)) -> Vec<u8> {
    let mut header = [0u8; 3];
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reader.read_exact(&mut header),
    )
    .await
    .expect("读加密帧头超时")
    .expect("duplex 读失败");
    assert_eq!(header[0], 0x01, "参考设备侧应发加密帧");
    let len = u16::from_be_bytes([header[1], header[2]]) as usize;
    let mut body = vec![0u8; len];
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reader.read_exact(&mut body),
    )
    .await
    .expect("读加密帧体超时")
    .expect("duplex 读失败");
    let mut frame = header.to_vec();
    frame.extend_from_slice(&body);
    frame
}

/// 建一条**真实加密 ESPHome API 连接**（参考 crate 作设备/responder，guest 作客户端）。
/// 返回 (参考侧连接句柄, 服务端侧 duplex)；返回时握手已完成（guest phase=ready）。
async fn open_encrypted_connection(
    host: &WasmHost,
    psk: &[u8; 32],
    session: &str,
) -> (esphome_native_api::Connection, tokio::io::DuplexStream) {
    use base64::Engine as _;
    use esphome_native_api::esphomeapi::EspHomeApi;

    let key_b64 = base64::engine::general_purpose::STANDARD.encode(psk);
    let (client_side, mut server_side) = tokio::io::duplex(64 * 1024);
    let api = EspHomeApi::builder()
        .name("esphome-satellite".to_string())
        .api_version_major(1)
        .api_version_minor(10)
        .encryption_key(key_b64)
        .build()
        .expect("api build（encryption_key base64 合法）");
    let task = tokio::spawn(async move { api.start(client_side).await });

    // guest 首包 = ClientHello + msg1（与 aioesphomeapi 客户端首写同布局，一次写入）。
    let start = call_noise(
        host,
        json!({
            "req_id": format!("{session}-start"),
            "op": "start",
            "session": session,
            "role": "initiator",
            "psk_hex": hex_encode(psk),
        }),
    );
    let start = ok(&start).clone();
    server_side
        .write_all(&hex_decode(start["out_hex"].as_str().unwrap()))
        .await
        .expect("写 ClientHello+msg1");

    // 参考设备先回 ServerHello，再回 msg2；guest 分两步消费。
    let server_hello = read_encrypted_frame(&mut server_side).await;
    let step = call_noise(
        host,
        json!({
            "req_id": format!("{session}-step1"),
            "op": "step",
            "session": session,
            "data_hex": hex_encode(&server_hello),
        }),
    );
    let step = ok(&step).clone();
    assert_eq!(step["phase"], json!("await-msg2"));
    assert_eq!(step["peer_name"], json!("esphome-satellite"));

    let msg2 = read_encrypted_frame(&mut server_side).await;
    let step2 = call_noise(
        host,
        json!({
            "req_id": format!("{session}-step2"),
            "op": "step",
            "session": session,
            "data_hex": hex_encode(&msg2),
        }),
    );
    let step2 = ok(&step2).clone();
    assert_eq!(step2["phase"], json!("ready"), "真实加密握手应完成");

    let connection = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .expect("参考侧握手超时")
        .expect("连接任务崩溃")
        .expect("参考侧加密握手失败");
    (connection, server_side)
}

#[tokio::test]
async fn real_encrypted_connection_roundtrip_with_guest_initiator() {
    let host = host();
    let (connection, mut server_side) = open_encrypted_connection(&host, &PSK, "real-1").await;

    // guest 加密发送 HelloRequest → 真设备回 HelloResponse（加密帧可被 guest 解出）。
    let seal = call_noise(
        &host,
        json!({
            "req_id": "real-seal",
            "op": "seal",
            "session": "real-1",
            "message": "hello_request",
            "client_info": "xiaozhi-bridge",
            "api_version_major": 1,
            "api_version_minor": 10,
        }),
    );
    let seal = ok(&seal).clone();
    server_side
        .write_all(&hex_decode(seal["frame_hex"].as_str().unwrap()))
        .await
        .expect("写加密 HelloRequest");

    let hello_response = read_encrypted_frame(&mut server_side).await;
    let open = call_noise(
        &host,
        json!({
            "req_id": "real-open",
            "op": "open",
            "session": "real-1",
            "data_hex": hex_encode(&hello_response),
        }),
    );
    let open = ok(&open).clone();
    assert_eq!(open["messages"][0]["type"], json!(2));
    assert_eq!(open["messages"][0]["message"], json!("hello_response"));
    assert_eq!(
        open["messages"][0]["hello"]["name"],
        json!("esphome-satellite")
    );
    assert_eq!(open["messages"][0]["hello"]["api_version_major"], json!(1));
    drop(connection);
}

/// 错误 psk 场景：完成 guest start + ServerHello 消费，返回设备侧任务句柄等待失败。
async fn open_encrypted_connection_wrong_psk(
    host: &WasmHost,
    device_psk: &[u8; 32],
    guest_psk: &[u8; 32],
    session: &str,
) -> (
    tokio::io::DuplexStream,
    tokio::task::JoinHandle<Result<esphome_native_api::Connection, esphome_native_api::Error>>,
) {
    use base64::Engine as _;
    use esphome_native_api::esphomeapi::EspHomeApi;

    let key_b64 = base64::engine::general_purpose::STANDARD.encode(device_psk);
    let (client_side, mut server_side) = tokio::io::duplex(64 * 1024);
    let api = EspHomeApi::builder()
        .name("esphome-satellite".to_string())
        .api_version_major(1)
        .api_version_minor(10)
        .encryption_key(key_b64)
        .build()
        .expect("api build");
    let task = tokio::spawn(async move { api.start(client_side).await });

    let start = call_noise(
        host,
        json!({
            "req_id": format!("{session}-start"),
            "op": "start",
            "session": session,
            "role": "initiator",
            "psk_hex": hex_encode(guest_psk),
        }),
    );
    let start = ok(&start).clone();
    server_side
        .write_all(&hex_decode(start["out_hex"].as_str().unwrap()))
        .await
        .expect("写 ClientHello+msg1");
    let server_hello = read_encrypted_frame(&mut server_side).await;
    let step = call_noise(
        host,
        json!({
            "req_id": format!("{session}-step1"),
            "op": "step",
            "session": session,
            "data_hex": hex_encode(&server_hello),
        }),
    );
    let step = ok(&step).clone();
    assert_eq!(step["phase"], json!("await-msg2"));
    (server_side, task)
}

#[tokio::test]
async fn real_encrypted_connection_rejects_wrong_psk_with_mac_failure() {
    let host = host();
    // 设备配 psk A，guest 用 psk B：设备读完 msg1 必须发错误帧并断链。
    let device_psk = [0x42u8; 32];
    let guest_psk = [0x43u8; 32];
    let (mut server_side, task) =
        open_encrypted_connection_wrong_psk(&host, &device_psk, &guest_psk, "wrong-1").await;

    // 设备已发 ServerHello，随后发的是错误帧（内容首字节 0x01 + 文本）。
    let error_frame = read_encrypted_frame(&mut server_side).await;
    let error_content = frame_content(&error_frame);
    assert_eq!(error_content[0], 0x01, "错误帧首字节 0x01（非握手消息）");
    let step = call_noise(
        &host,
        json!({
            "req_id": "wrong-step",
            "op": "step",
            "session": "wrong-1",
            "data_hex": hex_encode(&error_frame),
        }),
    );
    let message = err(&step);
    assert!(
        message.contains("Handshake MAC failure") || message.contains("握手失败"),
        "应识别对端 MAC 失败错误帧：{message}"
    );

    // 参考设备侧以 MAC 失败结束握手（不会返回 Connection）。
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .expect("参考侧失败超时")
        .expect("连接任务崩溃");
    assert!(outcome.is_err(), "错误 psk 不应建立连接：{outcome:?}");
}

// ==================================================================
// 会话状态 kv / 错误路径（纯 guest 事件面）
// ==================================================================

#[test]
fn noise_session_lifecycle_and_fail_closed_guards() {
    let host = host();
    let start_payload = json!({
        "req_id": "l-start",
        "op": "start",
        "session": "life-1",
        "role": "responder",
        "psk_hex": hex_encode(&PSK),
        "server_name": "xiaozhi-bridge",
    });
    ok(&call_noise(&host, start_payload.clone()));

    // 同 session 重复 start 拒绝覆盖（防握手状态被重置）。
    let duplicate = call_noise(
        &host,
        json!({
            "req_id": "l-dup",
            "op": "start",
            "session": "life-1",
            "role": "responder",
            "psk_hex": hex_encode(&PSK),
            "server_name": "xiaozhi-bridge",
        }),
    );
    assert!(err(&duplicate).contains("已存在"));

    // 未完成握手时 seal 显式拒绝。
    let premature = call_noise(
        &host,
        json!({
            "req_id": "l-seal",
            "op": "seal",
            "session": "life-1",
            "message": "ping_request",
        }),
    );
    assert!(err(&premature).contains("未就绪"));

    // 不存在的会话显式报错（不静默建新状态）。
    let missing = call_noise(
        &host,
        json!({
            "req_id": "l-missing",
            "op": "step",
            "session": "nope",
            "data_hex": "010000",
        }),
    );
    assert!(err(&missing).contains("不存在"));

    // 非法 psk 长度 / 非法 session 字符集 / 未知 op 全部 fail-closed。
    let bad_psk = call_noise(
        &host,
        json!({
            "req_id": "l-psk",
            "op": "start",
            "session": "life-2",
            "role": "initiator",
            "psk_hex": "0011",
        }),
    );
    assert!(err(&bad_psk).contains("psk"));
    let bad_session = call_noise(
        &host,
        json!({
            "req_id": "l-session",
            "op": "start",
            "session": "bad session!",
            "role": "initiator",
            "psk_hex": hex_encode(&PSK),
        }),
    );
    assert!(err(&bad_session).contains("session"));
    let bad_op = call_noise(
        &host,
        json!({"req_id": "l-op", "op": "explode", "session": "life-1"}),
    );
    assert!(err(&bad_op).contains("未知 noise op"));

    // 关闭会话：状态 kv 被回收，再次 step 显式失败。
    let close = call_noise(
        &host,
        json!({"req_id": "l-close", "op": "close", "session": "life-1"}),
    );
    assert_eq!(ok(&close)["removed"], json!(true));
    let after_close = call_noise(
        &host,
        json!({
            "req_id": "l-after-close",
            "op": "step",
            "session": "life-1",
            "data_hex": "010000",
        }),
    );
    assert!(err(&after_close).contains("不存在"));
}

#[test]
fn self_test_covers_noise_handshake_in_wasm() {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    let results = host
        .validate(PLUGIN, &ensure_fixture("esphome"))
        .expect("validate");
    assert!(results.iter().all(|r| r.ok), "自检应全过：{results:?}");
    let noise = results
        .iter()
        .find(|r| r.name == "noise-nnpsk0-handshake")
        .expect("自检应包含 noise 握手项");
    assert!(
        noise.detail.contains("双角色握手"),
        "noise 自检 detail 不符合预期：{}",
        noise.detail
    );
}
