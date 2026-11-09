//! S3u：esphome.official **设备侧会话（responder）**——真 TCP 全链对拍（Noise NNpsk0 加密）。
//!
//! 对拍权威 = `esphome-native-api` 3.0.0 的同源原语：其 `EspHomeApi` 内部即
//! `noise-protocol` + `noise-rust-crypto`（NNpsk0 + X25519 + ChaChaPoly + SHA256，
//! prologue `NoiseAPIInit\0\0`）。3.0.0 的 `EspHomeApi::start` 只实现**设备侧**握手
//! （先读对端 ClientHello 再回 ServerHello），无法充当主动连接的 HA 客户端；S3u 的
//! 插件正是设备侧，故客户端角色以同源原语搭建（S3t 已证明其与 EspHomeApi 设备侧
//! 真实互通）。加密应答帧与参考 CipherState 输出**逐字节对拍**（镜像密钥 + 同 nonce），
//! 并由客户端密钥真实解密还原 protobuf 明文。
//!
//! 全链路 = **真 TCP**（localhost 随机端口）：guest `connector.esp-home.session`
//! 事件步进会话，测试线程在客户端字节与 guest 事件之间泵；半包/粘包在 io 侧注入。
//! 明文全链见兄弟文件 `esphome_session.rs`（`super::` 复用泵与帧助手）。

use serde_json::json;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use esphome_native_api::parser::{proto_to_vec, ProtoMessage};
use esphome_native_api::proto::{HelloRequest, HelloResponse};
use noise_protocol::{CipherState, HandshakeState};

use super::esphome_session::{
    call_session, err, hex_decode, hex_encode, host, ok, pump, test_entities, test_identity,
    STATE_PREFIX,
};

const PLUGIN: &str = "esphome.official";
/// 与 ESPHome 固件一致的 prologue（aioesphomeapi `b"NoiseAPIInit\0\0"`）。
const PROLOGUE: &[u8] = b"NoiseAPIInit\0\0";

/// 参考握手状态（NNpsk0 + X25519 + ChaCha20Poly1305 + SHA256）。
type RefState = HandshakeState<
    noise_rust_crypto::X25519,
    noise_rust_crypto::ChaCha20Poly1305,
    noise_rust_crypto::Sha256,
>;
type RefCipher = CipherState<noise_rust_crypto::ChaCha20Poly1305>;

/// 参考客户端（initiator）：显式临时私钥（测试可复现）。
fn reference_client(psk: &[u8; 32], ephemeral: [u8; 32]) -> RefState {
    use noise_protocol::patterns::noise_nn_psk0;
    let seed =
        <noise_rust_crypto::sensitive::Sensitive<[u8; 32]> as noise_protocol::U8Array>::from_slice(
            &ephemeral,
        );
    let mut state = RefState::new(
        noise_nn_psk0(),
        true,
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

/// 解析加密帧 → 内容（校验标记/长度一致）。
fn frame_content(frame: &[u8]) -> Vec<u8> {
    assert_eq!(frame[0], 0x01, "加密帧标记必须 0x01");
    let len = u16::from_be_bytes([frame[1], frame[2]]) as usize;
    assert_eq!(frame.len(), 3 + len, "加密帧长度字段与实际不一致");
    frame[3..].to_vec()
}

/// 切分字节流 → 完整加密帧列表（校验标记与长度）。
fn split_encrypted_frames(buf: Vec<u8>) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut rest: &[u8] = &buf;
    while !rest.is_empty() {
        assert!(rest.len() >= 3 && rest[0] == 0x01, "加密帧标记/头不完整");
        let len = u16::from_be_bytes([rest[1], rest[2]]) as usize;
        assert!(
            rest.len() >= 3 + len,
            "加密帧不完整（{} < {}）",
            rest.len(),
            3 + len
        );
        frames.push(rest[..3 + len].to_vec());
        rest = &rest[3 + len..];
    }
    frames
}

/// 传输帧明文：`type(u16 BE) ‖ len(u16 BE) ‖ payload`。
fn transport_plaintext(message_type: u16, payload: &[u8]) -> Vec<u8> {
    let mut plaintext = Vec::with_capacity(4 + payload.len());
    plaintext.extend_from_slice(&message_type.to_be_bytes());
    plaintext.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    plaintext.extend_from_slice(payload);
    plaintext
}

/// 参考客户端发送：CipherState 封装 + 加密帧包装。
fn transport_frame(cipher: &mut RefCipher, message_type: u16, payload: &[u8]) -> Vec<u8> {
    encrypted_frame(&cipher.encrypt_vec(&transport_plaintext(message_type, payload)))
}

/// 镜像密钥：取参考密钥 + 指定 nonce 的新 CipherState（不干扰对拍方的 nonce 计数）。
fn mirror_cipher(cipher: &RefCipher, nonce: u64) -> RefCipher {
    let (key, _) = cipher.clone().extract();
    CipherState::new(&*key, nonce)
}

/// 从 socket 读一个完整加密帧（客户端视角），2s 超时护栏。
async fn read_encrypted_frame(reader: &mut (impl tokio::io::AsyncRead + Unpin)) -> Vec<u8> {
    let mut header = [0u8; 3];
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reader.read_exact(&mut header),
    )
    .await
    .expect("读加密帧头超时")
    .expect("socket 读失败");
    assert_eq!(header[0], 0x01, "设备侧应发加密帧");
    let len = u16::from_be_bytes([header[1], header[2]]) as usize;
    let mut body = vec![0u8; len];
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reader.read_exact(&mut body),
    )
    .await
    .expect("读加密帧体超时")
    .expect("socket 读失败");
    let mut frame = header.to_vec();
    frame.extend_from_slice(&body);
    frame
}

fn assert_server_hello_identity(frame: &[u8]) {
    let content = frame_content(frame);
    assert_eq!(content[0], 0x01, "ServerHello 首字节 = 加密协议 0x01");
    let name_end = content[1..]
        .iter()
        .position(|b| *b == 0)
        .expect("ServerHello name 终止符")
        + 1;
    assert_eq!(
        &content[1..name_end],
        b"xiaozhi-bridge",
        "ServerHello name 与 identity 不一致"
    );
    let mac = &content[name_end + 1..];
    let mac_end = mac.iter().position(|b| *b == 0).expect("mac 终止符");
    assert_eq!(
        &mac[..mac_end],
        b"AA:BB:CC:DD:EE:FF",
        "ServerHello mac 不一致"
    );
}

// ==================================================================
// ② 加密全链（真 TCP：ServerHello → noise 握手 → 加密封装往返 → 状态推送 → 断开）
// ==================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn encrypted_responder_full_chain_over_real_tcp() {
    let psk = [0x42u8; 32];
    let host = host();
    let session = "e1";

    // —— open（加密）：psk_hex → ServerHello 帧；临时密钥经宿主随机源。
    let open = ok(&call_session(
        &host,
        json!({
            "req_id": "e1-open",
            "op": "open",
            "session": session,
            "psk_hex": hex_encode(&psk),
            "identity": test_identity(),
            "entities": test_entities(),
        }),
    ));
    assert_eq!(open["mode"], json!("encrypted"));
    assert_eq!(open["phase"], json!("await-msg1"));
    let server_hello = hex_decode(open["out_hex"].as_str().unwrap());
    assert!(!server_hello.is_empty(), "加密 open 应返回 ServerHello 帧");
    assert_server_hello_identity(&server_hello);
    assert!(
        host.kv()
            .get(PLUGIN, &format!("{STATE_PREFIX}:{session}"))
            .is_some(),
        "open 后会话状态应落 kv"
    );

    // —— 真 TCP：客户端（HA 角色，参考原语搭建）连入，先收 ServerHello。
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let mut connect = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("客户端 connect");
    let (mut sock, _) = listener.accept().await.expect("accept");
    sock.write_all(&server_hello).await.expect("写 ServerHello");
    let hello_frame = read_encrypted_frame(&mut connect).await;
    assert_server_hello_identity(&hello_frame);

    // —— 握手：ClientHello（空帧）→ msg1 **半包**两写 → guest 回 msg2（经泵写回）。
    let mut client = reference_client(&psk, [0x11u8; 32]);
    connect
        .write_all(&[0x01, 0x00, 0x00])
        .await
        .expect("写 ClientHello");
    let feed = pump(&host, &mut sock, session, "e1-ch").await;
    assert_eq!(ok(&feed)["phase"], json!("await-msg1"));
    assert_eq!(feed["out_hex"], json!(""), "仅 ClientHello 不产出帧");

    let msg1 = client.write_message_vec(b"").expect("客户端写 msg1");
    let msg1_frame = handshake_frame(&msg1);
    connect
        .write_all(&msg1_frame[..msg1_frame.len() / 2])
        .await
        .expect("写 msg1 前半");
    let feed = pump(&host, &mut sock, session, "e1-m1a").await;
    assert_eq!(ok(&feed)["phase"], json!("await-msg1"));
    assert_eq!(feed["out_hex"], json!(""), "半包 msg1 不产出帧");
    connect
        .write_all(&msg1_frame[msg1_frame.len() / 2..])
        .await
        .expect("写 msg1 后半");
    let feed = pump(&host, &mut sock, session, "e1-m1b").await;
    assert_eq!(ok(&feed)["phase"], json!("ready"), "msg1 完整后握手应完成");

    // 客户端读 msg2（泵已写回 socket）→ 参考握手完成。
    let msg2 = read_encrypted_frame(&mut connect).await;
    let msg2_content = frame_content(&msg2);
    assert_eq!(msg2_content[0], 0x00, "msg2 握手帧首字节");
    client
        .read_message_vec(&msg2_content[1..])
        .expect("参考客户端读 msg2");
    assert!(client.completed(), "参考客户端握手应完成");
    let (mut c1, mut c2) = client.get_ciphers();

    // —— 加密封装往返：HelloRequest + PingRequest 粘包一次写；应答逐字节对拍。
    let hello_request = proto_to_vec(&ProtoMessage::HelloRequest(HelloRequest {
        client_info: "home-assistant".to_string(),
        api_version_major: 1,
        api_version_minor: 10,
    }))
    .expect("prost 编码 HelloRequest");
    let mut glued = transport_frame(&mut c1, 1, &hello_request);
    glued.extend_from_slice(&transport_frame(&mut c1, 7, &[]));
    connect.write_all(&glued).await.expect("写加密 Hello+Ping");
    let feed = pump(&host, &mut sock, session, "e1-hp").await;
    assert_eq!(ok(&feed)["messages"].as_array().map(Vec::len), Some(2));
    assert_eq!(feed["messages"][0]["type"], json!(1));
    assert_eq!(
        feed["messages"][0]["hello_request"]["client_info"],
        json!("home-assistant")
    );
    assert_eq!(feed["messages"][1]["type"], json!(7));
    let frames = split_encrypted_frames(hex_decode(feed["out_hex"].as_str().unwrap()));
    assert_eq!(frames.len(), 2, "应答 = HelloResponse + PingResponse 两帧");
    let hello_response = proto_to_vec(&ProtoMessage::HelloResponse(HelloResponse {
        api_version_major: 1,
        api_version_minor: 10,
        server_info: "xiaozhi:esphome-plugin".to_string(),
        name: "xiaozhi-bridge".to_string(),
    }))
    .expect("prost 编码 HelloResponse");
    let expected_hello = encrypted_frame(
        &mirror_cipher(&c2, 0).encrypt_vec(&transport_plaintext(2, &hello_response)),
    );
    assert_eq!(
        frames[0], expected_hello,
        "插件加密 HelloResponse 帧与参考 AEAD 输出不一致"
    );
    let plain = c2
        .decrypt_vec(&frame_content(&frames[0]))
        .expect("客户端解密 HelloResponse");
    assert_eq!(plain, transport_plaintext(2, &hello_response));
    let expected_ping =
        encrypted_frame(&mirror_cipher(&c2, 1).encrypt_vec(&transport_plaintext(8, &[])));
    assert_eq!(
        frames[1], expected_ping,
        "PingResponse 帧与参考 AEAD 不一致"
    );
    let plain = c2
        .decrypt_vec(&frame_content(&frames[1]))
        .expect("客户端解密 PingResponse");
    assert_eq!(plain, transport_plaintext(8, &[]), "PingResponse 空载荷");

    // —— 订阅：SubscribeStatesRequest（加密）→ 全量快照 2 帧。
    connect
        .write_all(&transport_frame(&mut c1, 20, &[]))
        .await
        .expect("写 SubscribeStates");
    let feed = pump(&host, &mut sock, session, "e1-sub").await;
    assert_eq!(ok(&feed)["subscribed"], json!(true));
    let frames = split_encrypted_frames(hex_decode(feed["out_hex"].as_str().unwrap()));
    assert_eq!(frames.len(), 2, "快照 = binary_sensor + number 各一帧");
    let bs_snapshot = proto_to_vec(&ProtoMessage::BinarySensorStateResponse(
        esphome_native_api::proto::BinarySensorStateResponse {
            key: 1,
            state: false,
            missing_state: false,
            device_id: 0,
        },
    ))
    .expect("prost 编码 binary_sensor 快照");
    let plain = c2
        .decrypt_vec(&frame_content(&frames[0]))
        .expect("客户端解密 binary_sensor 快照");
    assert_eq!(plain, transport_plaintext(21, &bs_snapshot));
    let number_snapshot = proto_to_vec(&ProtoMessage::NumberStateResponse(
        esphome_native_api::proto::NumberStateResponse {
            key: 2,
            state: 30.0,
            missing_state: false,
            device_id: 0,
        },
    ))
    .expect("prost 编码 number 快照");
    let plain = c2
        .decrypt_vec(&frame_content(&frames[1]))
        .expect("客户端解密 number 快照");
    assert_eq!(plain, transport_plaintext(50, &number_snapshot));

    // —— state op 推送状态变化（加密；客户端真实解密核对）。
    let state = ok(&call_session(
        &host,
        json!({
            "req_id": "e1-state",
            "op": "state",
            "session": session,
            "updates": [{"key": 1, "state": true}],
        }),
    ));
    let frames = split_encrypted_frames(hex_decode(state["out_hex"].as_str().unwrap()));
    assert_eq!(frames.len(), 1);
    let bs_on = proto_to_vec(&ProtoMessage::BinarySensorStateResponse(
        esphome_native_api::proto::BinarySensorStateResponse {
            key: 1,
            state: true,
            missing_state: false,
            device_id: 0,
        },
    ))
    .expect("prost 编码 binary_sensor 状态");
    let plain = c2
        .decrypt_vec(&frame_content(&frames[0]))
        .expect("客户端解密状态推送");
    assert_eq!(plain, transport_plaintext(21, &bs_on), "状态推送明文不一致");

    // —— 命令类只解码不回包（加密面）。
    let number_cmd = proto_to_vec(&ProtoMessage::NumberCommandRequest(
        esphome_native_api::proto::NumberCommandRequest {
            key: 2,
            state: 60.0,
            device_id: 0,
        },
    ))
    .expect("prost 编码 NumberCommandRequest");
    connect
        .write_all(&transport_frame(&mut c1, 51, &number_cmd))
        .await
        .expect("写加密 NumberCommand");
    let feed = pump(&host, &mut sock, session, "e1-cmd").await;
    assert_eq!(ok(&feed)["out_hex"], json!(""), "命令类不回包");
    assert_eq!(feed["messages"][0]["number_command"]["key"], json!(2));
    assert_eq!(feed["messages"][0]["number_command"]["state"], json!(60.0));
    assert_eq!(feed["messages"][0]["number_command"]["device_id"], json!(0));

    // —— Disconnect：加密封装应答 + closed → fail-closed 销毁状态。
    connect
        .write_all(&transport_frame(&mut c1, 5, &[]))
        .await
        .expect("写加密 Disconnect");
    let feed = pump(&host, &mut sock, session, "e1-dc").await;
    assert_eq!(ok(&feed)["closed"], json!(true));
    let frames = split_encrypted_frames(hex_decode(feed["out_hex"].as_str().unwrap()));
    assert_eq!(frames.len(), 1);
    let plain = c2
        .decrypt_vec(&frame_content(&frames[0]))
        .expect("客户端解密 DisconnectResponse");
    assert_eq!(
        plain,
        transport_plaintext(6, &[]),
        "DisconnectResponse 空载荷"
    );

    connect
        .write_all(&transport_frame(&mut c1, 7, &[]))
        .await
        .expect("写加密 Ping");
    let feed = pump(&host, &mut sock, session, "e1-after").await;
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
        .write_all(&transport_frame(&mut c1, 7, &[]))
        .await
        .expect("再写加密 Ping");
    let feed = pump(&host, &mut sock, session, "e1-gone").await;
    assert!(err(&feed).contains("会话不存在"), "{feed}");
}

// ==================================================================
// ④ 错误 psk：握手认证失败显式报错并销毁状态（fail-closed，不降级明文）
// ==================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn encrypted_responder_rejects_wrong_psk_fail_closed() {
    let device_psk = [0x42u8; 32];
    let client_psk = [0x43u8; 32];
    let host = host();
    let session = "e2";

    let open = ok(&call_session(
        &host,
        json!({
            "req_id": "e2-open",
            "op": "open",
            "session": session,
            "psk_hex": hex_encode(&device_psk),
            "identity": test_identity(),
            "entities": test_entities(),
        }),
    ));
    let server_hello = hex_decode(open["out_hex"].as_str().unwrap());
    assert!(!server_hello.is_empty());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let mut connect = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("客户端 connect");
    let (mut sock, _) = listener.accept().await.expect("accept");
    sock.write_all(&server_hello).await.expect("写 ServerHello");
    let _ = read_encrypted_frame(&mut connect).await;

    // 客户端用**错误 psk** 完成 msg1：guest AEAD 认证失败显式报错。
    let mut client = reference_client(&client_psk, [0x22u8; 32]);
    let msg1 = client.write_message_vec(b"").expect("客户端写 msg1");
    let mut first = vec![0x01, 0x00, 0x00];
    first.extend_from_slice(&handshake_frame(&msg1));
    connect
        .write_all(&first)
        .await
        .expect("写 ClientHello+msg1（错误 psk）");
    let feed = pump(&host, &mut sock, session, "e2-handshake").await;
    let message = err(&feed);
    assert!(
        message.contains("AEAD 认证失败"),
        "错误 psk 应显式认证失败：{message}"
    );
    assert!(
        host.kv()
            .get(PLUGIN, &format!("{STATE_PREFIX}:{session}"))
            .is_none(),
        "认证失败后状态应销毁"
    );

    // 会话已销毁：后续 feed 显式报不存在；close 无可删。
    connect
        .write_all(&handshake_frame(&msg1))
        .await
        .expect("再写 msg1");
    let feed = pump(&host, &mut sock, session, "e2-gone").await;
    assert!(err(&feed).contains("会话不存在"), "{feed}");
    let close = ok(&call_session(
        &host,
        json!({"req_id": "e2-close", "op": "close", "session": session}),
    ));
    assert_eq!(close["removed"], json!(false));

    // S3n 教训：保持连接不立即 drop（避免 RST 丢未读字节造成假失败）。
    drop(connect);
    drop(sock);
}
