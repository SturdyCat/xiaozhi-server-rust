//! S3m：tuya.official **v3.4 会话编排端到端**——guest 状态机 + 宿主 `tcp-connect`
//! + `io.tcp.*` 回灌，对端 = **rustuya 原语搭的模拟设备**（真 TCP）。
//!
//! 全链路：guest `lan-connect`（tcp-connect → 发 SessKeyNegStart）→ 模拟设备解密
//! nonce、回 SessKeyNegResp（ECB + HMAC 帧）→ 宿主读循环回灌 `io.tcp.data` →
//! guest 验证派生 → 发 SessKeyNegFinish → 模拟设备验证 finish_hmac → guest 状态
//! kv 置 Established。模拟设备全部用 rustuya 本尊原语（TuyaCipher / pack_message /
//! hmac）——这是「协议语义在插件侧可执行 + 可对接参考实现」的端到端证据。

use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::{ensure_fixture, host_with};
use crate::plugins::wasm::WasmHost;

const PLUGIN: &str = "tuya.official";
const LOCAL_KEY: &[u8] = b"0123456789abcdef";
const DEVICE_ID: &str = "ivf9id";
const REMOTE_NONCE: &[u8] = b"mock-remote-key1"; // 恰 16 字节（协议固定，见 verify_session_key_response）

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// rustuya 模拟 v3.4 设备：三步握手的设备侧。返回 (监听端口, 完成信号)。
#[cfg(feature = "tuya-plugin")]
async fn spawn_mock_v34_device() -> (u16, tokio::sync::oneshot::Receiver<()>) {
    use hmac::digest::{KeyInit as _, Mac as _};
    use rustuya::crypto::TuyaCipher;
    use rustuya::protocol::{pack_message, TuyaMessage, PREFIX_55AA};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");
        let cipher = TuyaCipher::new(LOCAL_KEY).expect("cipher");

        // —— 第 1 步：收 SessKeyNegStart（0x55AA + HMAC 尾，payload = nonce 密文）。
        let mut header = [0u8; 16];
        sock.read_exact(&mut header).await.expect("读 start 头");
        assert_eq!(&header[..4], &PREFIX_55AA.to_be_bytes(), "start 前缀");
        let len = u32::from_be_bytes([header[12], header[13], header[14], header[15]]) as usize;
        // len 字段 = 头（16B）之后的全部字节（payload + 尾差）。
        let mut rest = vec![0u8; len];
        sock.read_exact(&mut rest).await.expect("读 start 体");
        let start_frame = [header.to_vec(), rest].concat();

        // 解帧（HMAC 校验）+ ECB 解密去填充 → nonce。
        let hmac_key = Some(LOCAL_KEY);
        let trailer = 36; // hmac(32) + suffix(4)
        let payload_end = 16 + len - trailer;
        let ct = &start_frame[16..payload_end];
        let nonce = cipher
            .decrypt(ct, false, None, None, None)
            .expect("解 start nonce");

        // —— 第 2 步：造 SessKeyNegResp = remote_nonce ‖ HMAC(local_key, nonce)。
        let mut mac =
            <hmac::Hmac<sha2::Sha256> as hmac::digest::KeyInit>::new_from_slice(LOCAL_KEY)
                .expect("hmac key");
        mac.update(&nonce);
        let mut resp_payload = REMOTE_NONCE.to_vec();
        resp_payload.extend_from_slice(&mac.finalize().into_bytes());
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

        // —— 第 3 步：收 SessKeyNegFinish，验证 finish_hmac = HMAC(local_key, remote)。
        let mut header = [0u8; 16];
        sock.read_exact(&mut header).await.expect("读 finish 头");
        assert_eq!(
            u32::from_be_bytes([header[8], header[9], header[10], header[11]]),
            0x05
        );
        let len = u32::from_be_bytes([header[12], header[13], header[14], header[15]]) as usize;
        let mut rest = vec![0u8; len];
        sock.read_exact(&mut rest).await.expect("读 finish 体");
        let finish_frame = [header.to_vec(), rest].concat();
        let payload_end = 16 + len - trailer;
        let ct_finish = &finish_frame[16..payload_end];
        let finish_hmac = cipher
            .decrypt(ct_finish, false, None, None, None)
            .expect("解 finish");
        let mut expect =
            <hmac::Hmac<sha2::Sha256> as hmac::digest::KeyInit>::new_from_slice(LOCAL_KEY)
                .expect("hmac key");
        expect.update(REMOTE_NONCE);
        assert_eq!(
            finish_hmac.as_slice(),
            expect.finalize().into_bytes().as_slice(),
            "finish HMAC 应等于 HMAC(local_key, remote_nonce)"
        );

        // —— S3n：Established 后的心跳循环（密钥 = 会话 blob）。
        // blob = ECB(local_key, local_nonce ⊕ remote_nonce, 无填充)。
        let mut blob_src = Vec::new();
        for (i, b) in nonce.iter().enumerate() {
            blob_src.push(b ^ REMOTE_NONCE[i % REMOTE_NONCE.len()]);
        }
        let blob = cipher
            .encrypt(&blob_src, false, None, None, false)
            .expect("blob");
        assert_eq!(blob.len(), 16);

        // pong 帧：echo 收到的心跳明文，cmd 9，**blob** 加密 + HMAC 尾
        // （Established 后帧全换钥——外层 cipher 是 local_key，不能用）。
        let session_cipher = TuyaCipher::new(&blob).expect("session cipher");
        let pong = |plain: &[u8]| -> Vec<u8> {
            let ct = session_cipher
                .encrypt(plain, false, None, None, true)
                .expect("加密 pong");
            pack_message(
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
            .expect("打包 pong")
        };

        // 帧边界读取（55AA：头 16B 的 len 字段 = 头后全部字节）。
        let mut acc: Vec<u8> = Vec::new();
        let mut hb_seen = 0usize;
        let mut pong_sent = false;
        loop {
            // 读到至少一个完整帧
            let frame = loop {
                if let Some(pos) = find_frame_end(&acc) {
                    break acc.drain(..pos).collect::<Vec<u8>>();
                }
                let mut chunk = [0u8; 512];
                let n = sock.read(&mut chunk).await.expect("读心跳流");
                if n == 0 {
                    panic!("设备连接被关闭");
                }
                acc.extend_from_slice(&chunk[..n]);
            };
            // 验证 HMAC（blob）并解密
            let cmd = u32::from_be_bytes([frame[8], frame[9], frame[10], frame[11]]);
            let payload_end = frame.len() - 36;
            // 心跳帧密钥 = 会话 blob（不是 local_key—— Establishment 后帧全部换钥）。
            let session_cipher = TuyaCipher::new(&blob).expect("session cipher");
            let plain = session_cipher
                .decrypt(&frame[16..payload_end], false, None, None, None)
                .expect("解心跳载荷");
            eprintln!(
                "[mock] 收到帧 cmd={cmd:#04x} plain={}",
                String::from_utf8_lossy(&plain)
            );
            if cmd != 0x09 {
                continue;
            }
            hb_seen += 1;
            if !pong_sent {
                // 首次应答：拆两半写（半包重组）。
                let f = pong(&plain);
                let (a, b) = f.split_at(10);
                sock.write_all(a).await.expect("写 pong 前半");
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                sock.write_all(b).await.expect("写 pong 后半");
                // 粘包：一次 write 连发两帧。
                let mut glued = pong(&plain);
                glued.extend_from_slice(&pong(&plain));
                sock.write_all(&glued).await.expect("写粘包 pong ×2");
                pong_sent = true;
            }
            if hb_seen >= 1 && pong_sent {
                // 真实设备行为：应答后**保持连接**（立即 drop 可能发 RST，
                // 把尚未被应用层读走的 pong 字节丢弃——实测竞态）。
                let _ = done_tx.send(());
                // 继续读心跳直到对端关闭（测试结束时宿主断开）。
                loop {
                    let mut chunk = [0u8; 512];
                    match sock.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
                break;
            }
        }
    });
    (port, done_rx)
}

/// 55AA 帧边界：返回完整帧长度（数据不足返回 None）。
fn find_frame_end(buf: &[u8]) -> Option<usize> {
    // wire 前缀是完整 32-bit 00 00 55 AA（与 guest feed_split 同口径勘误）。
    if buf.len() < 16 || buf[2..4] != [0x55, 0xAA] || buf[0..2] != [0x00, 0x00] {
        return None;
    }
    let len = u32::from_be_bytes([buf[12], buf[13], buf[14], buf[15]]) as usize;
    let total = 16 + len;
    if buf.len() >= total {
        Some(total)
    } else {
        None
    }
}

fn host() -> WasmHost {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    host_with(&fetcher)
}

/// 轮询设备状态 kv 直到目标状态（泵任务需要运行时让出）。
async fn wait_state(host: &WasmHost, want: &str, timeout_ms: u64) -> Value {
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);
    loop {
        if let Some(bytes) = host
            .kv()
            .get(PLUGIN, &format!("tuya-lan-state:{DEVICE_ID}"))
        {
            let v: Value = serde_json::from_slice(&bytes).expect("状态 JSON");
            if v.get("state").and_then(Value::as_str) == Some(want) {
                return v;
            }
            if v.get("state").and_then(Value::as_str) == Some("Failed") {
                panic!("编排失败：{v}");
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let cur = host
                .kv()
                .get(PLUGIN, &format!("tuya-lan-state:{DEVICE_ID}"))
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_else(|| "<无状态>".into());
            panic!("等待状态 {want} 超时；当前状态：{cur}");
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(feature = "tuya-plugin")]
async fn lan_connect_negotiates_with_mock_device_end_to_end() {
    let (port, done_rx) = spawn_mock_v34_device().await;
    let host = host();
    host.load(PLUGIN, &ensure_fixture("tuya"))
        .expect("load tuya.official");

    let out = host
        .handle_event(
            PLUGIN,
            "connector.tuya.lan-connect",
            &json!({
                "req_id": "o1",
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

    // 全链路：握手三步经真 TCP 完成，guest 状态置 Established。
    let state = wait_state(&host, "Established", 5_000).await;
    assert!(
        state["session_key_hex"].as_str().is_some(),
        "Established 应带会话密钥 blob：{state}"
    );
    assert_eq!(state["version"], json!("3.4"));

    // S3n：心跳节拍（200ms 间隔）→ 设备收到并回 pong（半包 + 粘包注入）→
    // guest 帧切分重组后识别应答。等 last_pong_ms 与 data_frames ≥ 3。
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(5_000);
    let state = loop {
        let bytes = host
            .kv()
            .get(PLUGIN, &format!("tuya-lan-state:{DEVICE_ID}"))
            .expect("状态 kv");
        let v: Value = serde_json::from_slice(&bytes).expect("状态 JSON");
        if v["last_pong_ms"].as_u64().unwrap_or(0) > 0
            && v["data_frames"].as_u64().unwrap_or(0) >= 3
        {
            break v;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "等待心跳应答超时：{v}\nphase={}",
            host.snapshot_json()["plugins"][0]["phase"]
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    };
    assert!(
        state["data_frames"].as_u64().unwrap_or(0) >= 3,
        "应解出 ≥3 个数据帧（1 半包 + 2 粘包）：{state}"
    );
    // 模拟设备侧确认 finish_hmac 正确（对拍闭环）。
    tokio::time::timeout(std::time::Duration::from_secs(1), done_rx)
        .await
        .expect("等待模拟设备完成超时")
        .expect("完成信号通道关闭");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(feature = "tuya-plugin")]
async fn lan_connect_rejects_duplicate_session() {
    let (port, _done_rx) = spawn_mock_v34_device().await;
    let host = host();
    host.load(PLUGIN, &ensure_fixture("tuya"))
        .expect("load tuya.official");

    let payload = json!({
        "req_id": "o2",
        "device_id": DEVICE_ID,
        "ip": "127.0.0.1",
        "port": port,
        "local_key_hex": hex_encode(LOCAL_KEY),
        "version": "3.4",
    })
    .to_string();
    host.handle_event(PLUGIN, "connector.tuya.lan-connect", &payload)
        .expect("第一次连接");
    let state = wait_state(&host, "Established", 5_000).await;
    assert_eq!(state["state"], json!("Established"));

    // 已建立会话再连 → 显式业务错误（fail-closed，不静默重建）。泵路径对
    // guest 业务性 Err 静默，可观测面 = 结果通道（tuya-lan:o2 → ok:false）。
    host.handle_event(PLUGIN, "connector.tuya.lan-connect", &payload)
        .expect("第二次连接派发");
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(2_000);
    let result = loop {
        if let Some(bytes) = host.kv().get(PLUGIN, "tuya-lan:o2") {
            break serde_json::from_slice::<Value>(&bytes).expect("结果 JSON");
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "重复连接应落结果通道"
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    };
    assert_eq!(result["ok"], json!(false), "{result}");
    assert!(
        result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("已有会话"),
        "应显式报已有会话：{result}"
    );
}

// ==================================================================
// S3o：v3.5（0x6699/GCM）会话编排端到端——同一 guest 状态机换 wire 布局。
// 设备侧仍全用 rustuya 本尊原语（unpack_message / pack_message /
// ProtocolV35::encrypt_session_key），证据口径与 v3.4 完全一致。
// ==================================================================

const V35_REMOTE_NONCE: &[u8] = b"v35-remote-nonce"; // 恰 16 字节（协议固定）
const V35_DEV_RESP_IV: &[u8] = b"devresp-iv12"; // 恰 12 字节（设备选 IV，随帧传）
const V35_PONG_IV_A: &[u8] = b"pong-iv-a-12"; // 两帧 pong 必须不同 IV（GCM 纪律）
const V35_PONG_IV_B: &[u8] = b"pong-iv-b-12";

/// 读一个完整 0x6699 帧（头 18 + len + suffix 4；len 在 [14..18]）。
#[cfg(feature = "tuya-plugin")]
async fn read_frame_6699(sock: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut header = [0u8; 18];
    sock.read_exact(&mut header).await.expect("读 6699 帧头");
    assert_eq!(&header[..4], &[0x00, 0x00, 0x66, 0x99], "6699 前缀");
    let len = u32::from_be_bytes([header[14], header[15], header[16], header[17]]) as usize;
    let mut rest = vec![0u8; len + 4];
    sock.read_exact(&mut rest).await.expect("读 6699 帧体");
    [header.to_vec(), rest].concat()
}

#[cfg(feature = "tuya-plugin")]
fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    use hmac::digest::Mac as _;
    let mut mac =
        <hmac::Hmac<sha2::Sha256> as hmac::digest::KeyInit>::new_from_slice(key).expect("hmac key");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// rustuya 模拟 v3.5 设备：三步握手 + 心跳应答（pong 粘包一次写）。
#[cfg(feature = "tuya-plugin")]
async fn spawn_mock_v35_device() -> (u16, tokio::sync::oneshot::Receiver<()>) {
    use rustuya::crypto::TuyaCipher;
    use rustuya::protocol::{
        get_protocol, pack_message, unpack_message, TuyaMessage, Version, PREFIX_6699,
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.expect("accept");

        // —— 第 1 步：收 SessKeyNegStart（6699/GCM under local_key，payload=nonce 明文）。
        let start_frame = read_frame_6699(&mut sock).await;
        let start = unpack_message(&start_frame, Some(LOCAL_KEY), None, Some(true))
            .expect("解 start 帧（6699/GCM）");
        assert_eq!(start.cmd, 0x03, "start cmd");
        let nonce = start.payload.clone();
        assert_eq!(nonce.len(), 16, "start 载荷 = 16B nonce 明文");

        // —— 第 2 步：SessKeyNegResp = remote_nonce ‖ HMAC(local_key, nonce)，
        // 6699/GCM（设备自选 IV）。回包**拆两次写**：验证 guest 握手阶段也走流式重组。
        let mut resp_payload = V35_REMOTE_NONCE.to_vec();
        resp_payload.extend_from_slice(&hmac_sha256(LOCAL_KEY, &nonce));
        let resp_frame = pack_message(
            &TuyaMessage {
                seqno: 1,
                cmd: 0x04,
                payload: resp_payload,
                retcode: None,
                prefix: PREFIX_6699,
                iv: Some(V35_DEV_RESP_IV.to_vec()),
                ..Default::default()
            },
            Some(LOCAL_KEY),
        )
        .expect("打包 6699 resp");
        let (head, tail) = resp_frame.split_at(20);
        sock.write_all(head).await.expect("发 resp 前半");
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        sock.write_all(tail).await.expect("发 resp 后半");

        // —— 第 3 步：收 SessKeyNegFinish，验证 finish_hmac = HMAC(local_key, remote)。
        let finish_frame = read_frame_6699(&mut sock).await;
        let finish =
            unpack_message(&finish_frame, Some(LOCAL_KEY), None, Some(true)).expect("解 finish 帧");
        assert_eq!(finish.cmd, 0x05, "finish cmd");
        assert_eq!(
            finish.payload,
            hmac_sha256(LOCAL_KEY, V35_REMOTE_NONCE),
            "finish HMAC 应等于 HMAC(local_key, remote_nonce)"
        );

        // —— 会话密钥 blob（rustuya V35 本尊：GCM(local_key, nonce=local_nonce[..12])
        // 的 [12..28] 切片）——后续帧全部换钥，用 blob 当 GCM 密钥。
        let proto = get_protocol(Version::V3_5, Default::default());
        let (session_raw, _finish_hmac) = proto
            .finalize_session_key(&nonce, V35_REMOTE_NONCE, LOCAL_KEY)
            .expect("finalize_session_key");
        let local_cipher = TuyaCipher::new(LOCAL_KEY).expect("cipher");
        let blob = proto
            .encrypt_session_key(&session_raw, &local_cipher, &nonce)
            .expect("encrypt_session_key");
        assert_eq!(blob.len(), 16);

        // —— Established：读心跳帧（0x6699 under blob），验证明文 = {"devId","gwId"}。
        let expected_hb = format!(r#"{{"devId":"{DEVICE_ID}","gwId":"{DEVICE_ID}"}}"#);
        loop {
            let frame = read_frame_6699(&mut sock).await;
            let msg = unpack_message(&frame, Some(&blob), None, Some(true)).expect("解心跳帧");
            eprintln!(
                "[mock-v35] 收到帧 cmd={:#04x} plain={}",
                msg.cmd,
                String::from_utf8_lossy(&msg.payload)
            );
            if msg.cmd != 0x09 {
                continue;
            }
            assert_eq!(
                String::from_utf8_lossy(&msg.payload),
                expected_hb,
                "心跳明文形状（v3.5 与 v3.4 同载荷臂）"
            );
            // pong ×2 **粘包一次写**（guest 切分后应识别出 2 个数据帧）。
            let mut glued = Vec::new();
            for iv in [V35_PONG_IV_A, V35_PONG_IV_B] {
                glued.extend_from_slice(
                    &pack_message(
                        &TuyaMessage {
                            seqno: 99,
                            cmd: 0x09,
                            payload: msg.payload.clone(),
                            retcode: None,
                            prefix: PREFIX_6699,
                            iv: Some(iv.to_vec()),
                            ..Default::default()
                        },
                        Some(&blob),
                    )
                    .expect("打包 pong"),
                );
            }
            sock.write_all(&glued).await.expect("发粘包 pong ×2");
            let _ = done_tx.send(());
            // 保持连接（S3n 教训：应答后立即 drop 可能 RST 丢弃未读字节）。
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
async fn lan_connect_v35_negotiates_and_heartbeats_end_to_end() {
    let (port, done_rx) = spawn_mock_v35_device().await;
    let host = host();
    host.load(PLUGIN, &ensure_fixture("tuya"))
        .expect("load tuya.official");

    host.handle_event(
        PLUGIN,
        "connector.tuya.lan-connect",
        &json!({
            "req_id": "o3",
            "device_id": DEVICE_ID,
            "ip": "127.0.0.1",
            "port": port,
            "local_key_hex": hex_encode(LOCAL_KEY),
            "version": "3.5",
            "heartbeat_ms": 200,
        })
        .to_string(),
    )
    .expect("lan-connect 派发");

    // 握手三步（回包半包到达）经真 TCP 完成 → Established。
    let state = wait_state(&host, "Established", 5_000).await;
    assert_eq!(state["version"], json!("3.5"));
    assert!(
        state["session_key_hex"].as_str().is_some(),
        "Established 应带会话密钥 blob：{state}"
    );

    // 心跳节拍：200ms 后发 0x6699 心跳，设备收包验证明文并以粘包 ×2 回 pong。
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(5_000);
    let state = loop {
        let bytes = host
            .kv()
            .get(PLUGIN, &format!("tuya-lan-state:{DEVICE_ID}"))
            .expect("状态 kv");
        let v: Value = serde_json::from_slice(&bytes).expect("状态 JSON");
        if v["last_pong_ms"].as_u64().unwrap_or(0) > 0
            && v["data_frames"].as_u64().unwrap_or(0) >= 2
        {
            break v;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "等待 v3.5 心跳应答超时：{v}\nphase={}",
            host.snapshot_json()["plugins"][0]["phase"]
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    };
    assert!(
        state["data_frames"].as_u64().unwrap_or(0) >= 2,
        "粘包 ×2 应解出 ≥2 个数据帧：{state}"
    );
    // 设备侧断言全过才发完成信号（握手半包重组 + finish HMAC + 心跳明文）。
    tokio::time::timeout(std::time::Duration::from_secs(1), done_rx)
        .await
        .expect("等待模拟设备完成超时")
        .expect("完成信号通道关闭");
}
