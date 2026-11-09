//! S3p：tuya.official **自动重连 / 退避端到端**——guest 侧恢复策略的宿主回归。
//!
//! 证据面（对端仍是 rustuya 本尊原语搭的 v3.4 模拟设备）：
//! ① 握手成功后设备**主动断链** → guest `io.tcp.closed` → 退避 timeout →
//!    自动重连并再次握手成功（`reconnects` 计数、attempt 归零、换新 conn_id），
//!    新会话心跳/pong 正常；
//! ② 端口拒绝（无监听）→ 首次失败起退避 → 重试耗尽策略上限 → `Failed`，
//!    且 `next_retry_ms` 呈指数递增（50 → 100）。

use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::{ensure_fixture, host_with};
use crate::plugins::wasm::WasmHost;

const PLUGIN: &str = "tuya.official";
const LOCAL_KEY: &[u8] = b"0123456789abcdef";
const DEVICE_ID: &str = "ivf9id";
const REMOTE_NONCE: &[u8] = b"mock-remote-key1"; // 恰 16 字节（协议固定）

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

/// 轮询设备状态 kv 直到闭包成立（泵任务需要运行时让出）。
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

/// 读一个完整 0x55AA 帧（头 16 + len 内 body）。
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

/// v3.4 三步握手（设备侧）：返回会话密钥 blob（ECB(local_key, nonce⊕remote)）。
#[cfg(feature = "tuya-plugin")]
async fn v34_handshake(sock: &mut tokio::net::TcpStream) -> Vec<u8> {
    use rustuya::crypto::TuyaCipher;
    use rustuya::protocol::{pack_message, TuyaMessage, PREFIX_55AA};

    let cipher = TuyaCipher::new(LOCAL_KEY).expect("cipher");
    let start_frame = read_frame_55aa(sock).await;
    let trailer = 36; // hmac(32) + suffix(4)
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
    assert_eq!(
        finish_hmac,
        hmac_sha256(LOCAL_KEY, REMOTE_NONCE),
        "finish HMAC 应等于 HMAC(local_key, remote_nonce)"
    );

    let mut blob_src = Vec::new();
    for (i, b) in nonce.iter().enumerate() {
        blob_src.push(b ^ REMOTE_NONCE[i % REMOTE_NONCE.len()]);
    }
    let blob = cipher
        .encrypt(&blob_src, false, None, None, false)
        .expect("blob");
    assert_eq!(blob.len(), 16);
    blob
}

/// rustuya 模拟 v3.4 设备（支持两次连接）：第一次握手成功即**主动断链**，
/// 第二次接管后完成握手并回 pong，然后保持连接。
#[cfg(feature = "tuya-plugin")]
async fn spawn_mock_v34_reconnect_device() -> (u16, tokio::sync::oneshot::Receiver<()>) {
    use rustuya::crypto::TuyaCipher;
    use rustuya::protocol::{pack_message, TuyaMessage, PREFIX_55AA};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        // —— 第 1 次连接：完成握手后立刻断链（触发 guest 自动重连）。
        let (mut sock, _) = listener.accept().await.expect("accept#1");
        let _blob = v34_handshake(&mut sock).await;
        // 停留一拍再断链：让宿主回归能观测到「首连 Established」这一相位。
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        drop(sock);

        // —— 第 2 次连接：重连后的新会话，等心跳回 pong。
        let (mut sock, _) = listener.accept().await.expect("accept#2");
        let blob = v34_handshake(&mut sock).await;
        let session_cipher = TuyaCipher::new(&blob).expect("session cipher");
        loop {
            let frame = read_frame_55aa(&mut sock).await;
            let cmd = u32::from_be_bytes([frame[8], frame[9], frame[10], frame[11]]);
            let payload_end = frame.len() - 36;
            let plain = session_cipher
                .decrypt(&frame[16..payload_end], false, None, None, None)
                .expect("解心跳载荷");
            eprintln!(
                "[mock-reconnect] 第 2 次连接收到帧 cmd={cmd:#04x} plain={}",
                String::from_utf8_lossy(&plain)
            );
            if cmd != 0x09 {
                continue;
            }
            // pong（blob 密钥），随后保持连接直到对端关闭。
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
async fn lan_reconnects_after_device_close_end_to_end() {
    let (port, done_rx) = spawn_mock_v34_reconnect_device().await;
    let host = host();
    host.load(PLUGIN, &ensure_fixture("tuya"))
        .expect("load tuya.official");

    host.handle_event(
        PLUGIN,
        "connector.tuya.lan-connect",
        &json!({
            "req_id": "r1",
            "device_id": DEVICE_ID,
            "ip": "127.0.0.1",
            "port": port,
            "local_key_hex": hex_encode(LOCAL_KEY),
            "version": "3.4",
            "heartbeat_ms": 200,
            "reconnect_attempts": 5,
            "reconnect_base_ms": 50,
        })
        .to_string(),
    )
    .expect("lan-connect 派发");

    // 首连 Established（reconnects 仍为 0）。
    let first = wait_state_where(&host, "首连 Established", 5_000, |v| {
        v["state"] == json!("Established") && v["reconnects"].as_u64().unwrap_or(0) == 0
    })
    .await;
    let first_conn = first["conn_id"].as_u64().expect("首连 conn_id");

    // 设备主动断链 → guest 自动退避重连 → 第二次握手成功。
    let second = wait_state_where(&host, "重连后 Established", 8_000, |v| {
        v["state"] == json!("Established") && v["reconnects"].as_u64().unwrap_or(0) >= 1
    })
    .await;
    assert_ne!(
        second["conn_id"].as_u64(),
        Some(first_conn),
        "重连应分配新 connection-id：{second}"
    );
    assert_eq!(second["attempt"], json!(0), "握手成功后 attempt 应归零");
    assert_eq!(second["last_writer"], json!("handshake"));

    // 新会话心跳可用（设备在第二次连接上回 pong）。
    wait_state_where(&host, "重连后心跳应答", 5_000, |v| {
        v["last_pong_ms"].as_u64().unwrap_or(0) > 0
    })
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(1), done_rx)
        .await
        .expect("等待模拟设备完成超时")
        .expect("完成信号通道关闭");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(feature = "tuya-plugin")]
async fn lan_connect_retry_path_exhausts_attempts() {
    // 占位后释放监听端口 → 连接预期被拒绝（ECONNREFUSED）。
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);

    let host = host();
    host.load(PLUGIN, &ensure_fixture("tuya"))
        .expect("load tuya.official");
    host.handle_event(
        PLUGIN,
        "connector.tuya.lan-connect",
        &json!({
            "req_id": "r2",
            "device_id": DEVICE_ID,
            "ip": "127.0.0.1",
            "port": port,
            "local_key_hex": hex_encode(LOCAL_KEY),
            "version": "3.4",
            "reconnect_attempts": 2,
            "reconnect_base_ms": 50,
        })
        .to_string(),
    )
    .expect("lan-connect 派发");

    // 首连失败即落结果通道（ok:false）。
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(2_000);
    let result = loop {
        if let Some(bytes) = host.kv().get(PLUGIN, "tuya-lan:r2") {
            break serde_json::from_slice::<Value>(&bytes).expect("结果 JSON");
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "首连失败应落结果通道"
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    };
    assert_eq!(result["ok"], json!(false), "{result}");
    assert!(
        result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("tcp-connect 失败"),
        "应显式报连接失败：{result}"
    );

    // 重试 2 次后耗尽策略上限 → Failed；退避 50 → 100ms 指数递增。
    let failed = wait_state_where(&host, "重试耗尽 Failed", 8_000, |v| {
        v["state"] == json!("Failed")
    })
    .await;
    assert_eq!(failed["attempt"], json!(2), "应刚好耗尽 2 次重试：{failed}");
    assert_eq!(
        failed["next_retry_ms"],
        json!(100),
        "第二次退避应为 50ms·2：{failed}"
    );
    assert!(
        failed.get("reconnect_timer_id").is_none(),
        "耗尽后不应残留退避 timer：{failed}"
    );
}
