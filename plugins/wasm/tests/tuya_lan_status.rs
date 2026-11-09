//! S3q：tuya.official **设备主动上报（DP 状态）端到端**——状态语义与事件面回归。
//!
//! 对端 = rustuya 本尊原语搭的 v3.4 模拟设备：握手成功后立刻推一帧 Status
//! （cmd 0x08，`"3.4"` 协议头 + `{"dps":{"1":true,"2":30,"3":"on"}}`，会话 blob
//! ECB+PKCS7 + HMAC 尾），随后回心跳 pong。断言：
//! ① 状态 kv 的 DP 快照/在线/计数正确；② `tuya.official.lan-status` 事件带同一
//! 快照；③ 心跳 pong（JSON 但非上报命令）**不污染** DP 缓存与状态计数——这是
//! 与 native `LanSink`（任意 JSON 都套 parse_dps，心跳回显会把 devId/gwId 当 DP）
//! 的语义修正，见 `lan_status` 模块注释。

use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::{ensure_fixture, host_with};
use crate::plugins::wasm::WasmHost;

const PLUGIN: &str = "tuya.official";
const LOCAL_KEY: &[u8] = b"0123456789abcdef";
const DEVICE_ID: &str = "ivf9id";
const REMOTE_NONCE: &[u8] = b"mock-remote-key1";
const STATUS_TOPIC: &str = "tuya.official.lan-status";
/// 设备上报明文（协议头由 mock 拼在 JSON 前）。
const STATUS_JSON: &str = r#"{"devId":"ivf9id","dps":{"1":true,"2":30,"3":"on"},"t":123}"#;

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn host() -> WasmHost {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    host_with(&fetcher)
}

async fn wait_state_where<F>(host: &WasmHost, what: &str, timeout_ms: u64, ok: F) -> Value
where
    F: Fn(&Value) -> bool,
{
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);
    loop {
        if let Some(bytes) = host
            .kv()
            .get(PLUGIN, &format!("tuya-lan-state:{DEVICE_ID}"))
        {
            let v: Value = serde_json::from_slice(&bytes).expect("状态 JSON");
            if ok(&v) {
                return v;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let cur = host
                .kv()
                .get(PLUGIN, &format!("tuya-lan-state:{DEVICE_ID}"))
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_else(|| "<无状态>".into());
            panic!("等待状态条件「{what}」超时；当前：{cur}");
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }
}

#[cfg(feature = "tuya-plugin")]
fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    use hmac::digest::Mac as _;
    let mut mac =
        <hmac::Hmac<sha2::Sha256> as hmac::digest::KeyInit>::new_from_slice(key).expect("hmac key");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

#[cfg(feature = "tuya-plugin")]
async fn read_frame_55aa(sock: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut header = [0u8; 16];
    sock.read_exact(&mut header).await.expect("读帧头");
    assert_eq!(&header[..4], &[0x00, 0x00, 0x55, 0xAA], "55AA 前缀");
    let len = u32::from_be_bytes([header[12], header[13], header[14], header[15]]) as usize;
    let mut rest = vec![0u8; len];
    sock.read_exact(&mut rest).await.expect("读帧体");
    [header.to_vec(), rest].concat()
}

#[cfg(feature = "tuya-plugin")]
async fn v34_handshake(sock: &mut tokio::net::TcpStream) -> Vec<u8> {
    use rustuya::crypto::TuyaCipher;
    use rustuya::protocol::{pack_message, TuyaMessage, PREFIX_55AA};

    let cipher = TuyaCipher::new(LOCAL_KEY).expect("cipher");
    let start_frame = read_frame_55aa(sock).await;
    let trailer = 36;
    let payload_end = start_frame.len() - trailer;
    let nonce = cipher
        .decrypt(&start_frame[16..payload_end], false, None, None, None)
        .expect("解 start nonce");

    let mut resp_payload = REMOTE_NONCE.to_vec();
    resp_payload.extend_from_slice(&hmac_sha256(LOCAL_KEY, &nonce));
    let ct_resp = cipher
        .encrypt(&resp_payload, false, None, None, true)
        .expect("加密 resp");
    let resp_frame = pack_message(
        &TuyaMessage {
            seqno: 1,
            cmd: 0x04,
            payload: ct_resp,
            retcode: None,
            prefix: PREFIX_55AA,
            ..Default::default()
        },
        Some(LOCAL_KEY),
    )
    .expect("打包 resp");
    sock.write_all(&resp_frame).await.expect("发 resp");

    let finish_frame = read_frame_55aa(sock).await;
    let payload_end = finish_frame.len() - trailer;
    let finish_hmac = cipher
        .decrypt(&finish_frame[16..payload_end], false, None, None, None)
        .expect("解 finish");
    assert_eq!(finish_hmac, hmac_sha256(LOCAL_KEY, REMOTE_NONCE));

    let mut blob_src = Vec::new();
    for (i, b) in nonce.iter().enumerate() {
        blob_src.push(b ^ REMOTE_NONCE[i % REMOTE_NONCE.len()]);
    }
    cipher
        .encrypt(&blob_src, false, None, None, false)
        .expect("blob")
}

/// 模拟 v3.4 设备：握手成功 → 推 Status → 心跳回 pong；保持连接。
#[cfg(feature = "tuya-plugin")]
async fn spawn_mock_v34_status_device() -> (u16, tokio::sync::oneshot::Receiver<()>) {
    use rustuya::crypto::TuyaCipher;
    use rustuya::protocol::{pack_message, TuyaMessage, PREFIX_55AA};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");
        let blob = v34_handshake(&mut sock).await;
        let session_cipher = TuyaCipher::new(&blob).expect("session cipher");

        // —— 设备主动上报：Status(0x08)，协议头 "3.4"+12×0 进明文（ECB+PKCS7）。
        let mut plain = b"3.4".to_vec();
        plain.extend_from_slice(&[0u8; 12]);
        plain.extend_from_slice(STATUS_JSON.as_bytes());
        let ct = session_cipher
            .encrypt(&plain, false, None, None, true)
            .expect("加密 status");
        let status_frame = pack_message(
            &TuyaMessage {
                seqno: 7,
                cmd: 0x08,
                payload: ct,
                retcode: None,
                prefix: PREFIX_55AA,
                ..Default::default()
            },
            Some(&blob),
        )
        .expect("打包 status");
        sock.write_all(&status_frame).await.expect("发 status");

        // —— 心跳回 pong（JSON 但非上报命令：不应污染 DP 缓存）。
        loop {
            let frame = read_frame_55aa(&mut sock).await;
            let cmd = u32::from_be_bytes([frame[8], frame[9], frame[10], frame[11]]);
            let payload_end = frame.len() - 36;
            let plain = session_cipher
                .decrypt(&frame[16..payload_end], false, None, None, None)
                .expect("解心跳载荷");
            if cmd != 0x09 {
                continue;
            }
            let ct = session_cipher
                .encrypt(&plain, false, None, None, true)
                .expect("加密 pong");
            let pong = pack_message(
                &TuyaMessage {
                    seqno: 99,
                    cmd: 0x09,
                    payload: ct,
                    retcode: None,
                    prefix: PREFIX_55AA,
                    ..Default::default()
                },
                Some(&blob),
            )
            .expect("打包 pong");
            sock.write_all(&pong).await.expect("发 pong");
            let _ = done_tx.send(());
            // 保持连接直到对端关闭（RST 丢包教训）。
            loop {
                let mut chunk = [0u8; 512];
                match sock.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            break;
        }
    });
    (port, done_rx)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(feature = "tuya-plugin")]
async fn lan_status_report_updates_dps_and_publishes_event() {
    let (port, done_rx) = spawn_mock_v34_status_device().await;
    let host = host();
    // 先订阅再激活：验证 guest 经 `publish` 发布到插件命名空间。
    let mut events = host.subscribe_topic(STATUS_TOPIC);
    host.load(PLUGIN, &ensure_fixture("tuya"))
        .expect("load tuya.official");

    host.handle_event(
        PLUGIN,
        "connector.tuya.lan-connect",
        &json!({
            "req_id": "q1",
            "device_id": DEVICE_ID,
            "ip": "127.0.0.1",
            "port": port,
            "local_key_hex": hex_encode(LOCAL_KEY),
            "version": "3.4",
            "heartbeat_ms": 200,
        })
        .to_string(),
    )
    .expect("lan-connect 派发");

    wait_state_where(&host, "Established", 5_000, |v| {
        v["state"] == json!("Established")
    })
    .await;

    // ① 状态 kv：DP 快照 + 在线 + 计数（单源 parse_dps 已在 native dps_tests 钉住形状）。
    let state = wait_state_where(&host, "DP 状态落 kv", 5_000, |v| {
        v["status_count"].as_u64().unwrap_or(0) >= 1
    })
    .await;
    assert_eq!(state["dps"]["1"], json!(true), "{state}");
    assert_eq!(state["dps"]["2"], json!(30), "{state}");
    assert_eq!(state["dps"]["3"], json!("on"), "{state}");
    assert_eq!(state["online"], json!(true));
    assert_eq!(state["last_status_dp_count"], json!(3));
    assert!(
        state["last_status_ms"].as_u64().unwrap_or(0) > 0,
        "应记录上报时间：{state}"
    );

    // ② 事件面：`tuya.official.lan-status` 携带同一 DP 快照。
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
        .await
        .expect("等待状态事件超时")
        .expect("状态事件通道关闭");
    assert_eq!(event.topic, STATUS_TOPIC);
    let payload: Value = serde_json::from_slice(&event.payload).expect("事件 JSON");
    assert_eq!(payload["device_id"], json!(DEVICE_ID));
    assert_eq!(payload["count"], json!(3));
    assert_eq!(payload["dps"]["1"], json!(true), "{payload}");
    assert_eq!(payload["online"], json!(true));

    // ③ 心跳 pong（JSON 载荷、非上报命令）不污染 DP：计数仍 1、键仍 3 个。
    let after_pong = wait_state_where(&host, "心跳应答", 5_000, |v| {
        v["last_pong_ms"].as_u64().unwrap_or(0) > 0
    })
    .await;
    assert_eq!(
        after_pong["status_count"],
        json!(1),
        "心跳 pong 不应计入状态上报：{after_pong}"
    );
    assert_eq!(
        after_pong["dps"].as_object().map(|m| m.len()),
        Some(3),
        "心跳 pong 不应污染 DP 缓存：{after_pong}"
    );
    assert!(
        after_pong["dps"].get("devId").is_none(),
        "devId 不应被当成 DP：{after_pong}"
    );

    tokio::time::timeout(std::time::Duration::from_secs(1), done_rx)
        .await
        .expect("等待模拟设备完成超时")
        .expect("完成信号通道关闭");
}
