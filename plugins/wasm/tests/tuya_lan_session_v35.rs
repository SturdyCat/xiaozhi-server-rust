//! S3j：tuya.official **v3.5（0x6699/GCM）TCP 会话层 rustuya 对拍**。
//!
//! 与 v3.4（`tuya_lan_session.rs`，共享 hex/host/call_lan/craft_resp_payload）
//! 共享验证与密钥派生（HMAC 校验 / XOR / finish HMAC）；差异面 =
//! 0x6699 帧级 GCM（载荷只拼 "3.5" 头不加密、IV 由调用方传入）+
//! 密钥库 blob = GCM(local_key, nonce=local_nonce[..12]) 的 **[12..28] 切片**
//! （密文前 16 字节、无 tag——rustuya `encrypt_session_key` 怪癖）。
//! 宿主侧参考 = `TuyaProtocol`(V3_5) + `pack_message` 6699 分支同批原语。

use serde_json::{json, Value};

use super::tuya_lan_session::{
    call_lan, craft_resp_payload, hex_encode, host, LOCAL_KEY, LOCAL_NONCE,
};

const V35_IV: &[u8] = b"v35iv-12byte"; // 恰 12 字节

/// v3.5 宿主参考：`pack_payload`（"3.5" 头，**不做帧前加密**）→ 0x6699
/// GCM 帧（IV 传入，AAD=帧头 [4..18]）。
#[cfg(feature = "tuya-plugin")]
fn v35_pack_ref(seqno: u32, cmd: u32, plaintext_payload: &[u8], key: &[u8], iv: &[u8]) -> Vec<u8> {
    use rustuya::crypto::TuyaCipher;
    use rustuya::protocol::{pack_message, TuyaMessage, TuyaProtocol, Version};

    let proto = rustuya::protocol::get_protocol(Version::V3_5, Default::default());
    let packed = proto
        .pack_payload(
            plaintext_payload,
            cmd,
            &TuyaCipher::new(key).expect("cipher"),
        )
        .expect("pack_payload");
    let msg = TuyaMessage {
        seqno,
        cmd,
        payload: packed,
        retcode: None,
        prefix: rustuya::protocol::PREFIX_6699,
        iv: Some(iv.to_vec()),
        ..Default::default()
    };
    pack_message(&msg, Some(key)).expect("pack_message")
}

/// v3.5 宿主参考：回包验证 + 密钥派生 + GCM 密钥库 blob。
#[cfg(feature = "tuya-plugin")]
fn v35_session_derive_ref(resp_payload: &[u8]) -> (Vec<u8>, Vec<u8>) {
    use rustuya::crypto::TuyaCipher;
    use rustuya::protocol::{TuyaProtocol, Version};

    let proto = rustuya::protocol::get_protocol(Version::V3_5, Default::default());
    let remote_nonce = proto
        .verify_session_key_response(LOCAL_NONCE, resp_payload, LOCAL_KEY)
        .expect("verify_session_key_response");
    let (session_key_raw, _finish_hmac) = proto
        .finalize_session_key(LOCAL_NONCE, &remote_nonce, LOCAL_KEY)
        .expect("finalize_session_key");
    let cipher = TuyaCipher::new(LOCAL_KEY).expect("cipher");
    let blob = proto
        .encrypt_session_key(&session_key_raw, &cipher, LOCAL_NONCE)
        .expect("encrypt_session_key");
    (remote_nonce, blob)
}

#[test]
#[cfg(feature = "tuya-plugin")]
fn lan_session_start_frame_v35_matches_rustuya_byte_for_byte() {
    let host = host();
    let out = call_lan(
        &host,
        "connector.tuya.lan-session-start",
        json!({
            "req_id": "s5",
            "version": "3.5",
            "seqno": 21,
            "local_key_hex": hex_encode(LOCAL_KEY),
            "nonce_hex": hex_encode(LOCAL_NONCE),
            "iv_hex": hex_encode(V35_IV),
        }),
        "s5",
    );
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["version"], json!("3.5"));
    assert_eq!(out["cmd"], json!(0x03));
    // v3.5 start：nonce 明文进帧（裸头命令），帧级 GCM。
    let expect = v35_pack_ref(21, 0x03, LOCAL_NONCE, LOCAL_KEY, V35_IV);
    assert_eq!(
        hex_decode_v35(out["frame_hex"].as_str().unwrap()),
        expect,
        "v3.5 SessKeyNegStart 帧逐字节不一致"
    );
}

#[test]
#[cfg(feature = "tuya-plugin")]
fn lan_session_finish_flow_v35_matches_rustuya() {
    let resp_payload = craft_resp_payload(LOCAL_KEY, LOCAL_NONCE);
    let (remote_nonce, blob) = v35_session_derive_ref(&resp_payload);
    let finish_hmac = {
        use hmac::digest::{KeyInit as _, Mac as _};
        type HmacSha256 = hmac::Hmac<sha2::Sha256>;
        let mut mac =
            <HmacSha256 as hmac::digest::KeyInit>::new_from_slice(LOCAL_KEY).expect("hmac key");
        mac.update(&remote_nonce);
        mac.finalize().into_bytes().to_vec()
    };
    let expect = v35_pack_ref(22, 0x05, &finish_hmac, LOCAL_KEY, V35_IV);

    let host = host();
    let out = call_lan(
        &host,
        "connector.tuya.lan-session-finish",
        json!({
            "req_id": "s6",
            "version": "3.5",
            "seqno": 22,
            "local_key_hex": hex_encode(LOCAL_KEY),
            "nonce_hex": hex_encode(LOCAL_NONCE),
            "resp_payload_hex": hex_encode(&resp_payload),
            "iv_hex": hex_encode(V35_IV),
        }),
        "s6",
    );
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["version"], json!("3.5"));
    assert_eq!(out["cmd"], json!(0x05));
    assert_eq!(
        hex_decode_v35(out["remote_nonce_hex"].as_str().unwrap()),
        remote_nonce,
        "remote nonce 派生不一致"
    );
    assert_eq!(
        hex_decode_v35(out["session_key_hex"].as_str().unwrap()),
        blob,
        "v3.5 会话密钥库 blob（GCM [12..28] 切片）不一致"
    );
    assert_eq!(
        hex_decode_v35(out["frame_hex"].as_str().unwrap()),
        expect,
        "v3.5 SessKeyNegFinish 帧逐字节不一致"
    );
}

#[test]
#[cfg(feature = "tuya-plugin")]
fn lan_session_finish_v35_rejects_tampered_hmac() {
    let mut resp_payload = craft_resp_payload(LOCAL_KEY, LOCAL_NONCE);
    resp_payload[30] ^= 0x80; // 篡改 HMAC 尾部
    let host = host();
    let out = call_lan(
        &host,
        "connector.tuya.lan-session-finish",
        json!({
            "req_id": "s7",
            "version": "3.5",
            "seqno": 23,
            "local_key_hex": hex_encode(LOCAL_KEY),
            "nonce_hex": hex_encode(LOCAL_NONCE),
            "resp_payload_hex": hex_encode(&resp_payload),
            "iv_hex": hex_encode(V35_IV),
        }),
        "s7",
    );
    assert_eq!(out["ok"], json!(false), "{out}");
    assert!(
        out["error"].as_str().unwrap_or_default().contains("HMAC"),
        "应显式报 HMAC 验证失败：{out}"
    );
}

#[test]
#[cfg(feature = "tuya-plugin")]
fn lan_command_v35_control_under_session_key_matches_rustuya() {
    let resp_payload = craft_resp_payload(LOCAL_KEY, LOCAL_NONCE);
    let (_rn, blob) = v35_session_derive_ref(&resp_payload);
    let dps = json!({"1": true});
    let t: u64 = 1_700_000_002;
    let dev_id = "ivf9id";

    use rustuya::protocol::{CommandType, TuyaProtocol, Version};
    let proto = rustuya::protocol::get_protocol(Version::V3_5, Default::default());
    let (cmd, pv) = proto
        .generate_payload(dev_id, CommandType::Control, Some(dps.clone()), None, t)
        .expect("generate_payload");
    let pbytes = serde_json::to_vec(&pv).expect("serialize");
    let expect = v35_pack_ref(31, cmd, &pbytes, &blob, V35_IV);

    let host = host();
    let out = call_lan(
        &host,
        "connector.tuya.lan-command",
        json!({
            "req_id": "c5",
            "version": "3.5",
            "seqno": 31,
            "device_id": dev_id,
            "t": t,
            "command": "control",
            "dps": dps,
            "key_hex": hex_encode(&blob),
            "iv_hex": hex_encode(V35_IV),
        }),
        "c5",
    );
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["cmd"], json!(cmd), "effective cmd 应为 ControlNew");
    assert_eq!(
        hex_decode_v35(out["frame_hex"].as_str().unwrap()),
        expect,
        "v3.5 控制帧逐字节不一致"
    );
}

#[test]
#[cfg(feature = "tuya-plugin")]
fn lan_heartbeat_v35_matches_rustuya() {
    let resp_payload = craft_resp_payload(LOCAL_KEY, LOCAL_NONCE);
    let (_rn, blob) = v35_session_derive_ref(&resp_payload);
    let dev_id = "ivf9id";

    use rustuya::protocol::{CommandType, TuyaProtocol, Version};
    let proto = rustuya::protocol::get_protocol(Version::V3_5, Default::default());
    let (cmd, pv) = proto
        .generate_payload(dev_id, CommandType::HeartBeat, None, None, 0)
        .expect("generate_payload");
    let pbytes = serde_json::to_vec(&pv).expect("serialize");
    let expect = v35_pack_ref(32, cmd, &pbytes, &blob, V35_IV);

    let host = host();
    let out = call_lan(
        &host,
        "connector.tuya.lan-command",
        json!({
            "req_id": "c6",
            "version": "3.5",
            "seqno": 32,
            "device_id": dev_id,
            "command": "heartbeat",
            "key_hex": hex_encode(&blob),
            "iv_hex": hex_encode(V35_IV),
        }),
        "c6",
    );
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["cmd"], json!(0x09));
    assert_eq!(
        hex_decode_v35(out["frame_hex"].as_str().unwrap()),
        expect,
        "v3.5 心跳帧逐字节不一致"
    );
}

/// 本文件自己的 hex 解码（兄弟模块的 `hex_decode` 未导出，保持模块自足）。
#[cfg(feature = "tuya-plugin")]
fn hex_decode_v35(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex"))
        .collect()
}
