//! S3v：`[esp_home].engine = "wasm-session"`（生产接管）的真 TCP 回归。
//!
//! 被测面 = 宿主桥的连接编排（[`super::session`]）：真实 esphome.wasm 宿主
//! （`ensure_fixture` 重建产物）+ 真客户端（明文 = 裸 socket + 参考 prost 编码；
//! 加密 = S3u 同款同源 noise 原语 + 插件配置 psk）跑通：握手/Hello/目录（放行
//! 过滤生效）/订阅快照/状态推送/音量命令落到 actions/断开回收。
//!
//! 桥侧纪律对拍：客户端字节**原样**进 `op=feed`（桥不做协议解析）；psk 不经桥
//! ——加密密钥由插件配置命名空间的 `noise_psk`（config-secret）提供，桥侧
//! `[esp_home].encryption_key` 恒空；fail-closed（错 key 不降级明文）。

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use super::super::session as session_engine;
use super::tests::{contains, field_f32, field_u32, frame, read_frame};
use super::tests::{
    BINARY_SENSOR_STATE, DEVICE_INFO_REQUEST, DEVICE_INFO_RESPONSE, HELLO_REQUEST, HELLO_RESPONSE,
    LIST_ENTITIES_BINARY_SENSOR, LIST_ENTITIES_DONE, LIST_ENTITIES_NUMBER, LIST_ENTITIES_REQUEST,
    NUMBER_COMMAND, NUMBER_STATE, SUBSCRIBE_STATES_REQUEST,
};
use super::*;
use crate::plugins::wasm::tests::{ensure_fixture, host_with, ScriptedFetcher};
use crate::plugins::wasm::WasmHost;

use esphome_native_api::parser::{parse_proto_message, proto_to_vec, ProtoMessage};
use esphome_native_api::proto::{HelloRequest, HelloResponse, NumberCommandRequest};
use noise_protocol::{CipherState, HandshakeState};

// 消息类型号（server_tests 未收录的三条；取自 api.proto）。
const PING_REQUEST: u8 = 7;
const PING_RESPONSE: u8 = 8;
const DISCONNECT_REQUEST: u8 = 5;
const DISCONNECT_RESPONSE: u8 = 6;

const PLUGIN: &str = session_engine::SESSION_PLUGIN;
const STATE_PREFIX: &str = session_engine::STATE_PREFIX;

/// 全局状态（`state::speaking/muted/volume` 与命令记录器）是**进程级共享**的：
/// 改状态的全链测试必须串行执行，否则彼此的状态变化会经全局 broadcast 串进
/// 对方桥的推送（曾致偶发「speaking 应推为 true」读到复位值的假失败）。
static STATE_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 持有全局状态串行锁（RAII：guard 存活期独占 `state::*` 与命令记录器）。
async fn state_serial() -> tokio::sync::MutexGuard<'static, ()> {
    STATE_SERIAL.lock().await
}

/// wasm-session 测试配置（mdns 关闭；加密密钥恒空——密钥在插件配置里）。
fn session_cfg(port: u16) -> EspHomeConfig {
    EspHomeConfig {
        engine: "wasm-session".into(),
        port,
        mdns: false,
        ..Default::default()
    }
}

/// 装载真实 esphome.wasm 宿主；`noise_psk_b64` 非空时写入插件配置命名空间
/// （S3v：psk 的唯一来源，桥与 `[esp_home].encryption_key` 均不参与）。
fn session_host(noise_psk_b64: Option<&str>) -> Arc<WasmHost> {
    let fetcher = Arc::new(ScriptedFetcher::new());
    let host = host_with(&fetcher);
    if let Some(psk) = noise_psk_b64 {
        host.config_store()
            .set(PLUGIN, "noise_psk", psk)
            .expect("写入插件配置 noise_psk");
    }
    host.load(PLUGIN, &ensure_fixture("esphome"))
        .expect("load esphome.official");
    Arc::new(host)
}

/// 等待插件的会话状态 kv 被回收（连接结束/协议失败后 guest/桥侧销毁）。
async fn wait_session_state_gone(host: &WasmHost) {
    for _ in 0..60 {
        if host
            .kv()
            .keys(PLUGIN, &format!("{STATE_PREFIX}:"))
            .is_empty()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("会话状态未被回收（esphome-session-state:* 仍存在）");
}

/// base64 编码（测试侧组插件配置值用；标准字母表 + 填充）。
fn b64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// 读帧直到谓词命中（有界护栏；返回命中帧之前的全部帧——用于并发测试下的
/// 状态推送断言：广播是进程级单例，不假设「下一帧」必然是自己写的那个变化）。
async fn read_frame_until(
    stream: &mut TcpStream,
    mut pred: impl FnMut(u8, &[u8]) -> bool,
) -> (u8, Vec<u8>) {
    for _ in 0..32 {
        let (ty, payload) = read_frame(stream).await;
        if pred(ty, &payload) {
            return (ty, payload);
        }
    }
    panic!("读帧护栏耗尽（32 帧内未等到目标帧）");
}

// ============================================================
// ① 明文全链（真 TCP + accept_loop + 放行过滤 + 命令落 actions + 断开回收）
// ============================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wasm_session_plaintext_full_chain_over_real_tcp() {
    let _serial = state_serial().await;
    let _ = env_logger::builder().is_test(true).try_init();
    // 不配置 noise_psk → guest config 读不到 → 明文会话（桥不传 psk_hex）。
    let host = session_host(None);

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let cfg = session_cfg(addr.port());
    // 目录计划：隐藏「静音」实体（key=4），验证放行过滤贯穿目录/快照/状态推送。
    let plan = state::CatalogPlan::from_overrides([(state::MUTED, false, None::<String>)]);
    let loop_task = tokio::spawn(super::accept_loop(
        listener,
        cfg.clone(),
        super::ConnEngine::WasmSession(Arc::clone(&host)),
        plan,
    ));

    let mut stream = TcpStream::connect(addr).await.unwrap();
    // 1) Hello：参考 prost 编码；桥原样透传 → 插件 HelloResponse。
    let hello = proto_to_vec(&ProtoMessage::HelloRequest(HelloRequest {
        client_info: "home-assistant".to_string(),
        api_version_major: 1,
        api_version_minor: 10,
    }))
    .unwrap();
    stream
        .write_all(&frame(HELLO_REQUEST, &hello))
        .await
        .unwrap();
    let (ty, payload) = read_frame(&mut stream).await;
    assert_eq!(ty, HELLO_RESPONSE, "期望 HelloResponse");
    match parse_proto_message(usize::from(ty), &payload).unwrap() {
        ProtoMessage::HelloResponse(h) => {
            assert_eq!(h.name, "xiaozhi-dot2", "HelloResponse 应带配置节点名");
            assert_eq!(h.api_version_major, 1);
            assert_eq!(h.api_version_minor, 10);
        }
        other => panic!("期待 HelloResponse，收到 {other:?}"),
    }

    // 2) DeviceInfo：身份面来自桥的 open 载荷；明文会话不应声明支持加密。
    stream
        .write_all(&frame(DEVICE_INFO_REQUEST, &[]))
        .await
        .unwrap();
    let (ty, payload) = read_frame(&mut stream).await;
    assert_eq!(ty, DEVICE_INFO_RESPONSE);
    match parse_proto_message(usize::from(ty), &payload).unwrap() {
        ProtoMessage::DeviceInfoResponse(info) => {
            assert_eq!(info.name, "xiaozhi-dot2");
            assert_eq!(info.friendly_name, "EchoMuse Dot 2");
            assert_eq!(info.model, "EchoMuse Dot 2");
            assert_eq!(info.manufacturer, "xiaozhi");
            assert!(!info.api_encryption_supported, "明文会话不应声明支持加密");
            assert_eq!(
                info.legacy_bluetooth_proxy_version, 0,
                "wasm-session 不宣告 BLE 代理"
            );
        }
        other => panic!("期待 DeviceInfoResponse，收到 {other:?}"),
    }

    // 3) 目录：5 实体（隐藏 muted）+ Done；object_id 不含 dot2_muted。
    stream
        .write_all(&frame(LIST_ENTITIES_REQUEST, &[]))
        .await
        .unwrap();
    let mut listed: Vec<(u8, Vec<u8>)> = Vec::new();
    loop {
        let (ty, payload) = read_frame(&mut stream).await;
        if ty == LIST_ENTITIES_DONE {
            break;
        }
        listed.push((ty, payload));
    }
    assert_eq!(
        listed.len(),
        5,
        "4 binary + 1 number（muted 被目录计划隐藏）"
    );
    for (ty, payload) in &listed {
        assert!(
            !contains(payload, b"dot2_muted"),
            "被隐藏的实体不应出现在目录里"
        );
        assert!(
            *ty == LIST_ENTITIES_BINARY_SENSOR || *ty == LIST_ENTITIES_NUMBER,
            "目录消息类型异常：{ty}"
        );
    }
    // 音量定义的 key 恒为静态索引（不因放行过滤重排）。
    let volume_def = listed
        .iter()
        .find(|(_, p)| contains(p, b"dot2_volume"))
        .unwrap();
    assert_eq!(field_u32(&volume_def.1, 2), state::VOLUME as u64);

    // 4) 订阅：快照只含放行实体（4 binary + 1 number），key 为静态索引。
    stream
        .write_all(&frame(SUBSCRIBE_STATES_REQUEST, &[]))
        .await
        .unwrap();
    let mut snapshot: Vec<(u8, Vec<u8>)> = Vec::new();
    for _ in 0..5 {
        let (ty, payload) = read_frame(&mut stream).await;
        assert!(
            ty == BINARY_SENSOR_STATE || ty == NUMBER_STATE,
            "快照帧类型 {ty}"
        );
        snapshot.push((ty, payload));
    }
    let mut keys: Vec<u64> = snapshot.iter().map(|(_, p)| field_u32(p, 1)).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![0, 1, 2, 3, state::VOLUME as u64],
        "快照应只含放行实体（muted 被隐藏，key = 静态索引）"
    );

    // 5) 状态推送：speaking(key=3) 置 true 应推给客户端（广播是进程级单例，
    //    读到目标帧为准）；muted 被隐藏——读到音量帧为止不得出现 key=4。
    state::set_speaking(true);
    let (_ty, payload) = read_frame_until(&mut stream, |ty, p| {
        assert!(
            !(ty == BINARY_SENSOR_STATE && field_u32(p, 1) == state::MUTED as u64),
            "被目录计划隐藏的实体不应收到状态推送"
        );
        ty == BINARY_SENSOR_STATE && field_u32(p, 1) == 3
    })
    .await;
    assert_eq!(field_u32(&payload, 2), 1, "speaking 应推为 true");
    state::set_speaking(false);
    state::set_muted(true);
    state::set_volume(42.5);
    let (_ty, payload) = read_frame_until(&mut stream, |ty, p| {
        ty == NUMBER_STATE && field_u32(p, 1) == state::VOLUME as u64
    })
    .await;
    assert_eq!(field_f32(&payload, 2), 42.5, "音量应推为 42.5");
    state::set_speaking(false);
    state::set_muted(false);
    state::set_volume(0.0);

    // 6) 音量命令：NumberCommand(key=VOLUME, 30) → 命令落到 actions（观测替身），
    //    连接继续可用（Ping 有应答；读侧顺带消化复位状态帧）。
    let mut cmd = vec![0x0d, state::VOLUME as u8, 0x00, 0x00, 0x00, 0x15];
    cmd.extend_from_slice(&30.0f32.to_le_bytes());
    stream
        .write_all(&frame(NUMBER_COMMAND, &cmd))
        .await
        .unwrap();
    stream.write_all(&frame(PING_REQUEST, &[])).await.unwrap();
    let (ty, _) = read_frame_until(&mut stream, |ty, _| ty == PING_RESPONSE).await;
    assert_eq!(ty, PING_RESPONSE, "命令后连接应继续应答");
    assert!(
        session_engine::recorded_volume_commands()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&30),
        "音量命令应经桥落到 actions（档位 30）"
    );

    // 7) Disconnect：应答 DisconnectResponse，随后连接关闭、会话状态被回收。
    stream
        .write_all(&frame(DISCONNECT_REQUEST, &[]))
        .await
        .unwrap();
    let (ty, _) = read_frame_until(&mut stream, |ty, _| ty == DISCONNECT_RESPONSE).await;
    assert_eq!(ty, DISCONNECT_RESPONSE, "期望 DisconnectResponse");
    // 桥在 closed 后主动断开：读侧收到 EOF。
    let mut eof = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut eof)).await;
    assert!(
        matches!(read, Ok(Ok(0)) | Err(_)),
        "closed 后桥应断开连接（{read:?}）"
    );
    wait_session_state_gone(&host).await;

    // S3n 教训：对端应答后保持连接对象不立即 drop（避免 RST 假失败）。
    drop(stream);
    loop_task.abort();
}

// ============================================================
// ② 加密全链（psk 来自插件配置命名空间；同源 noise 原语客户端）
// ============================================================

/// 参考握手状态（NNpsk0 + X25519 + ChaCha20Poly1305 + SHA256，prologue 同 ESPHome）。
const PROLOGUE: &[u8] = b"NoiseAPIInit\0\0";
type RefState = HandshakeState<
    noise_rust_crypto::X25519,
    noise_rust_crypto::ChaCha20Poly1305,
    noise_rust_crypto::Sha256,
>;
type RefCipher = CipherState<noise_rust_crypto::ChaCha20Poly1305>;

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

/// 加密帧：`0x01 ‖ u16 BE(len) ‖ content`；握手帧内容 = `0x00 ‖ noise 消息`。
fn encrypted_frame(content: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x01, (content.len() >> 8) as u8, content.len() as u8];
    frame.extend_from_slice(content);
    frame
}

fn handshake_frame(message: &[u8]) -> Vec<u8> {
    let mut content = vec![0x00];
    content.extend_from_slice(message);
    encrypted_frame(&content)
}

fn frame_content(frame: &[u8]) -> Vec<u8> {
    assert_eq!(frame[0], 0x01, "加密帧标记必须 0x01");
    let len = u16::from_be_bytes([frame[1], frame[2]]) as usize;
    assert_eq!(frame.len(), 3 + len, "加密帧长度字段与实际不一致");
    frame[3..].to_vec()
}

fn transport_plaintext(message_type: u16, payload: &[u8]) -> Vec<u8> {
    let mut plaintext = Vec::with_capacity(4 + payload.len());
    plaintext.extend_from_slice(&message_type.to_be_bytes());
    plaintext.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    plaintext.extend_from_slice(payload);
    plaintext
}

fn transport_frame(cipher: &mut RefCipher, message_type: u16, payload: &[u8]) -> Vec<u8> {
    encrypted_frame(&cipher.encrypt_vec(&transport_plaintext(message_type, payload)))
}

/// 读一个完整加密帧（客户端视角），2s 超时护栏。
async fn read_encrypted_frame(reader: &mut (impl tokio::io::AsyncRead + Unpin)) -> Vec<u8> {
    let mut header = [0u8; 3];
    tokio::time::timeout(Duration::from_secs(2), reader.read_exact(&mut header))
        .await
        .expect("读加密帧头超时")
        .expect("socket 读失败");
    assert_eq!(header[0], 0x01, "设备侧应发加密帧");
    let len = u16::from_be_bytes([header[1], header[2]]) as usize;
    let mut body = vec![0u8; len];
    tokio::time::timeout(Duration::from_secs(2), reader.read_exact(&mut body))
        .await
        .expect("读加密帧体超时")
        .expect("socket 读失败");
    let mut frame = header.to_vec();
    frame.extend_from_slice(&body);
    frame
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wasm_session_encrypted_full_chain_with_plugin_config_psk() {
    let _serial = state_serial().await;
    let _ = env_logger::builder().is_test(true).try_init();
    // psk 只写在插件配置命名空间（config-secret 读取写审计）；桥侧
    // `[esp_home].encryption_key` 恒空——验证 psk 确实来自插件配置。
    let psk = [0x5au8; 32];
    let host = session_host(Some(&b64(&psk)));

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let loop_task = tokio::spawn(super::accept_loop(
        listener,
        session_cfg(addr.port()),
        super::ConnEngine::WasmSession(Arc::clone(&host)),
        state::CatalogPlan::passthrough(),
    ));

    let mut stream = TcpStream::connect(addr).await.unwrap();
    // 1) ServerHello 先行（open 产出，桥写回）。
    let server_hello = read_encrypted_frame(&mut stream).await;
    assert!(!server_hello.is_empty());
    let content = frame_content(&server_hello);
    assert_eq!(content[0], 0x01, "ServerHello 首字节 = 加密协议 0x01");
    assert!(
        contains(&content, b"xiaozhi-dot2"),
        "ServerHello 应带节点名"
    );

    // 2) 握手：ClientHello → msg1 → msg2（guest 用配置 psk 完成 NNpsk0）。
    let mut client = reference_client(&psk, [0x33u8; 32]);
    stream.write_all(&[0x01, 0x00, 0x00]).await.unwrap();
    let msg1 = client.write_message_vec(b"").unwrap();
    stream.write_all(&handshake_frame(&msg1)).await.unwrap();
    let msg2 = read_encrypted_frame(&mut stream).await;
    let msg2_content = frame_content(&msg2);
    assert_eq!(msg2_content[0], 0x00, "msg2 握手帧首字节");
    client
        .read_message_vec(&msg2_content[1..])
        .expect("客户端读 msg2");
    assert!(client.completed(), "参考客户端握手应完成");
    let (mut c1, mut c2) = client.get_ciphers();

    // 3) 加密 Hello + SubscribeStates 粘包：应答 = HelloResponse + 6 帧快照
    //    （一个 out_hex 一次写回；逐帧读，真实解密核对）。
    let hello = proto_to_vec(&ProtoMessage::HelloRequest(HelloRequest {
        client_info: "home-assistant".to_string(),
        api_version_major: 1,
        api_version_minor: 10,
    }))
    .unwrap();
    let mut glued = transport_frame(&mut c1, 1, &hello);
    glued.extend_from_slice(&transport_frame(
        &mut c1,
        SUBSCRIBE_STATES_REQUEST as u16,
        &[],
    ));
    stream.write_all(&glued).await.unwrap();
    let first = read_encrypted_frame(&mut stream).await;
    let plain = c2
        .decrypt_vec(&frame_content(&first))
        .expect("解密 HelloResponse");
    let hello_response = proto_to_vec(&ProtoMessage::HelloResponse(HelloResponse {
        api_version_major: 1,
        api_version_minor: 10,
        server_info: "xiaozhi:esphome-plugin".to_string(),
        name: "xiaozhi-dot2".to_string(),
    }))
    .unwrap();
    assert_eq!(
        plain,
        transport_plaintext(2, &hello_response),
        "HelloResponse 明文不一致"
    );

    // 快照 6 帧（5 binary + 1 number）：解密后核对 key 序列（key = 静态索引）。
    let mut keys: Vec<u64> = Vec::with_capacity(6);
    for _ in 0..6 {
        let frame = read_encrypted_frame(&mut stream).await;
        let plain = c2.decrypt_vec(&frame_content(&frame)).expect("解密快照帧");
        assert!(
            matches!(&plain[0..2], [0, 21] | [0, 50]),
            "快照帧类型异常：{:02X?}",
            &plain[0..2]
        );
        keys.push(field_u32(&plain[4..], 1));
    }
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![0, 1, 2, 3, 4, state::VOLUME as u64],
        "快照应覆盖全部 6 实体（key = 静态索引）"
    );

    // 4) 状态推送（加密）：speaking(key=3) 变化真实解密核对（有界护栏）。
    state::set_speaking(true);
    for _ in 0..32 {
        let frame = read_encrypted_frame(&mut stream).await;
        let plain = c2
            .decrypt_vec(&frame_content(&frame))
            .expect("解密状态推送");
        if plain[0..2] == [0, 21] && field_u32(&plain[4..], 1) == 3 {
            assert_eq!(field_u32(&plain[4..], 2), 1, "speaking 应推为 true");
            break;
        }
    }
    state::set_speaking(false);
    state::set_volume(0.0);

    // 5) 音量命令（加密面）落到 actions；Ping 确认连接可用
    //    （读侧顺带消化 speaking 复位帧）。
    let cmd = proto_to_vec(&ProtoMessage::NumberCommandRequest(NumberCommandRequest {
        key: state::VOLUME as u32,
        state: 55.0,
        device_id: 0,
    }))
    .unwrap();
    stream
        .write_all(&transport_frame(&mut c1, 51, &cmd))
        .await
        .unwrap();
    stream
        .write_all(&transport_frame(&mut c1, PING_REQUEST as u16, &[]))
        .await
        .unwrap();
    for _ in 0..16 {
        let frame = read_encrypted_frame(&mut stream).await;
        let plain = c2.decrypt_vec(&frame_content(&frame)).expect("解密应答帧");
        if plain[0..2] == [0, PING_RESPONSE] {
            assert_eq!(plain, transport_plaintext(PING_RESPONSE as u16, &[]));
            break;
        }
    }
    assert!(
        session_engine::recorded_volume_commands()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&55),
        "加密面的音量命令应经桥落到 actions（档位 55）"
    );

    // 6) Disconnect → 加密应答 → 桥断开 → 会话状态回收（有界护栏）。
    stream
        .write_all(&transport_frame(&mut c1, 5, &[]))
        .await
        .unwrap();
    for _ in 0..16 {
        let frame = read_encrypted_frame(&mut stream).await;
        let plain = c2
            .decrypt_vec(&frame_content(&frame))
            .expect("解密 DisconnectResponse");
        if plain[0..2] == [0, 6] {
            assert_eq!(
                plain,
                transport_plaintext(6, &[]),
                "DisconnectResponse 空载荷"
            );
            break;
        }
    }
    let mut eof = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut eof)).await;
    assert!(
        matches!(read, Ok(Ok(0)) | Err(_)),
        "closed 后桥应断开连接（{read:?}）"
    );
    wait_session_state_gone(&host).await;

    drop(stream);
    loop_task.abort();
    state::set_speaking(false);
    state::set_volume(0.0);
}

// ============================================================
// ③ 错误 psk：握手认证失败 → 桥 fail-closed 断开，不降级明文
// ============================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wasm_session_wrong_psk_fails_closed_without_plaintext_fallback() {
    let _serial = state_serial().await;
    let _ = env_logger::builder().is_test(true).try_init();
    let device_psk = [0x42u8; 32];
    let host = session_host(Some(&b64(&device_psk)));

    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let loop_task = tokio::spawn(super::accept_loop(
        listener,
        session_cfg(addr.port()),
        super::ConnEngine::WasmSession(Arc::clone(&host)),
        state::CatalogPlan::passthrough(),
    ));

    let mut stream = TcpStream::connect(addr).await.unwrap();
    let _server_hello = read_encrypted_frame(&mut stream).await;

    // 客户端用**错误 psk** 完成 msg1：guest AEAD 认证失败 → 桥显式失败断开。
    let wrong = [0x43u8; 32];
    let mut client = reference_client(&wrong, [0x44u8; 32]);
    stream.write_all(&[0x01, 0x00, 0x00]).await.unwrap();
    let msg1 = client.write_message_vec(b"").unwrap();
    stream.write_all(&handshake_frame(&msg1)).await.unwrap();

    // fail-closed 断言：连接被关闭（EOF/错误），且**绝不**出现明文帧（首字节 0x00）。
    let mut buf = vec![0u8; 64];
    loop {
        let read = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf)).await;
        match read {
            Ok(Ok(0)) | Err(_) => break, // EOF / 超时（连接已断不再有数据）
            Ok(Ok(n)) => assert_ne!(
                buf[0],
                0x00,
                "认证失败后不得降级明文（收到明文帧 {:#02X?}）",
                &buf[..n]
            ),
            Ok(Err(_)) => break,
        }
    }
    // 会话状态已被 fail-closed 销毁。
    wait_session_state_gone(&host).await;
    drop(stream);
    loop_task.abort();
}

// ============================================================
// ④ 启动期守卫（fail-closed：缺宿主 / 缺插件 / ble_proxy 组合）
// ============================================================

#[test]
fn wasm_session_guard_rejects_missing_host() {
    let cfg = EspHomeConfig {
        engine: "wasm-session".into(),
        ble_proxy: false,
        ..Default::default()
    };
    let err = super::ensure_wasm_session_ready(&cfg, None).unwrap_err();
    assert!(err.to_string().contains("WASM 宿主未装载"), "{err}");
}

#[test]
fn wasm_session_guard_rejects_missing_plugin() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    let host = Arc::new(host_with(&fetcher)); // 不装载任何插件
    let cfg = EspHomeConfig {
        engine: "wasm-session".into(),
        ble_proxy: false,
        ..Default::default()
    };
    let err = super::ensure_wasm_session_ready(&cfg, Some(&host)).unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("已装载并激活") && message.contains("esphome.official"),
        "{message}"
    );
}

#[test]
fn wasm_session_guard_rejects_ble_proxy_combination() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    let host = Arc::new(host_with(&fetcher));
    host.load(PLUGIN, &ensure_fixture("esphome"))
        .expect("load esphome.official");
    // 默认 ble_proxy=true：与 wasm-session 组合必须显式拒绝（BLE 消息面未进插件）。
    let cfg = EspHomeConfig {
        engine: "wasm-session".into(),
        ..Default::default()
    };
    let err = super::ensure_wasm_session_ready(&cfg, Some(&host)).unwrap_err();
    assert!(err.to_string().contains("ble_proxy"), "{err}");
}

#[test]
fn wasm_session_guard_passes_with_plugin_and_ble_off() {
    let fetcher = Arc::new(ScriptedFetcher::new());
    let host = Arc::new(host_with(&fetcher));
    host.load(PLUGIN, &ensure_fixture("esphome"))
        .expect("load esphome.official");
    let cfg = EspHomeConfig {
        engine: "wasm-session".into(),
        ble_proxy: false,
        ..Default::default()
    };
    super::ensure_wasm_session_ready(&cfg, Some(&host)).expect("守卫应放行");
}
