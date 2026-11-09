//! S3i/S3j：tuya.official **v3.4 / v3.5 TCP 会话层状态机 rustuya 对拍**。
//!
//! 宿主侧参考 = 外部 crate rustuya 的 `TuyaProtocol`（V3_4 / V3_5）+ `TuyaCipher`
//! + `pack_message`——即 `device/actor.rs::negotiate_session_key` 在宿主运行时
//! 所走的同一批原语。黄金向量固定（guest 无随机源，nonce/IV 由调用方传入），
//! 逐字节对拍：start 帧、finish 帧、会话密钥库 blob、会话模式下的控制/心跳帧。
//! TCP 传输本身仍由 native 持有（WIT 契约暂无 TCP 出站），本片证据面 =
//! 「协商状态机可完整迁插件」。
//!
//! 两代差异（对拍实证）：v3.4 = 0x55AA + HMAC 尾 + 载荷 ECB + blob=ECB(无填充)；
//! v3.5 = 0x6699 + 帧级 GCM（载荷仅拼 "3.5" 头不加密）+ blob=GCM 密文前 16B；
//! 验证与密钥派生（HMAC 校验 / XOR / finish HMAC）两代共享。

use serde_json::{json, Value};
use std::sync::Arc;

use super::{ensure_fixture, host_with};
use crate::plugins::wasm::WasmHost;

const PLUGIN: &str = "tuya.official";
const LAN_PREFIX_KEY: &str = "tuya-lan";

/// 固定黄金向量（16B 对齐；与 tuya_lan.rs 的 GCM 向量同风格）。
#[cfg(feature = "tuya-plugin")]
pub(crate) const LOCAL_KEY: &[u8] = b"0123456789abcdef";
#[cfg(feature = "tuya-plugin")]
pub(crate) const LOCAL_NONCE: &[u8] = b"local-nonce-16by";
#[cfg(feature = "tuya-plugin")]
const REMOTE_NONCE: &[u8] = b"remote-nonce-16b";

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(feature = "tuya-plugin")]
fn hex_decode(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex"))
        .collect()
}

pub(crate) fn host() -> WasmHost {
    let fetcher = Arc::new(super::ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load(PLUGIN, &ensure_fixture("tuya"))
        .expect("load tuya.official");
    host
}

pub(crate) fn call_lan(host: &WasmHost, topic: &str, payload: Value, req_id: &str) -> Value {
    host.handle_event(PLUGIN, topic, &payload.to_string())
        .expect("lan dispatch");
    let bytes = host
        .kv()
        .get(PLUGIN, &format!("{LAN_PREFIX_KEY}:{req_id}"))
        .unwrap_or_else(|| panic!("LAN 结果未落 kv（{req_id}）"));
    serde_json::from_slice(&bytes).expect("结果应为合法 JSON")
}

/// rustuya 宿主参考：按 v3.4 `pack_msg` 全链打包（pack_payload 的
/// ECB+padding + "3.4" 头判定 → 0x55AA + HMAC 尾）。明文 payload 进、
/// 完整帧字节出——会话/控制/心跳统一用这一个参考入口。
#[cfg(feature = "tuya-plugin")]
fn v34_pack_ref(seqno: u32, cmd: u32, plaintext_payload: &[u8], key: &[u8]) -> Vec<u8> {
    use rustuya::crypto::TuyaCipher;
    use rustuya::protocol::{pack_message, TuyaMessage, TuyaProtocol, Version};

    let proto = rustuya::protocol::get_protocol(Version::V3_4, Default::default());
    let cipher = TuyaCipher::new(key).expect("cipher");
    let packed = proto
        .pack_payload(plaintext_payload, cmd, &cipher)
        .expect("pack_payload");
    let msg = TuyaMessage {
        seqno,
        cmd,
        payload: packed,
        retcode: None,
        prefix: proto.get_prefix(),
        ..Default::default()
    };
    pack_message(&msg, proto.get_hmac_key(cipher.key())).expect("pack_message")
}

/// rustuya 宿主参考：从 resp payload（remote_nonce || HMAC）派生会话密钥库
/// blob——与 `negotiate_session_key` 第 2+3 步同批原语。
#[cfg(feature = "tuya-plugin")]
fn v34_session_derive_ref(resp_payload: &[u8]) -> (Vec<u8>, Vec<u8>) {
    use rustuya::crypto::TuyaCipher;
    use rustuya::protocol::{TuyaProtocol, Version};

    let proto = rustuya::protocol::get_protocol(Version::V3_4, Default::default());
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

/// 设备回包 payload（解密后形态）：remote_nonce(16) || HMAC-SHA256(local_key, local_nonce)(32)。
#[cfg(feature = "tuya-plugin")]
pub(crate) fn craft_resp_payload(key: &[u8], nonce: &[u8]) -> Vec<u8> {
    use hmac::digest::{KeyInit as _, Mac as _};
    type HmacSha256 = hmac::Hmac<sha2::Sha256>;
    let mut mac = <HmacSha256 as hmac::digest::KeyInit>::new_from_slice(key).expect("hmac key");
    mac.update(nonce);
    let mut resp = REMOTE_NONCE.to_vec();
    resp.extend_from_slice(&mac.finalize().into_bytes());
    resp
}

#[cfg(feature = "tuya-plugin")]
fn session_start_payload() -> Value {
    json!({
        "req_id": "s1",
        "seqno": 1,
        "local_key_hex": hex_encode(LOCAL_KEY),
        "nonce_hex": hex_encode(LOCAL_NONCE),
    })
}

#[test]
#[cfg(feature = "tuya-plugin")]
fn lan_session_start_frame_matches_rustuya_byte_for_byte() {
    let host = host();
    let out = call_lan(
        &host,
        "connector.tuya.lan-session-start",
        session_start_payload(),
        "s1",
    );
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["cmd"], json!(0x03));
    let expect = v34_pack_ref(1, 0x03, LOCAL_NONCE, LOCAL_KEY);
    assert_eq!(
        hex_decode(out["frame_hex"].as_str().unwrap()),
        expect,
        "SessKeyNegStart 帧逐字节不一致"
    );
}

#[test]
#[cfg(feature = "tuya-plugin")]
fn lan_session_finish_flow_matches_rustuya() {
    let resp_payload = craft_resp_payload(LOCAL_KEY, LOCAL_NONCE);
    let (remote_nonce, blob) = v34_session_derive_ref(&resp_payload);

    let host = host();
    let out = call_lan(
        &host,
        "connector.tuya.lan-session-finish",
        json!({
            "req_id": "s2",
            "seqno": 2,
            "local_key_hex": hex_encode(LOCAL_KEY),
            "nonce_hex": hex_encode(LOCAL_NONCE),
            "resp_payload_hex": hex_encode(&resp_payload),
        }),
        "s2",
    );
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["cmd"], json!(0x05));
    assert_eq!(
        hex_decode(out["remote_nonce_hex"].as_str().unwrap()),
        remote_nonce,
        "remote nonce 派生不一致"
    );
    assert_eq!(
        hex_decode(out["session_key_hex"].as_str().unwrap()),
        blob,
        "会话密钥库 blob 不一致"
    );
    // finish 帧：rustuya 参考 = HMAC(local_key, remote_nonce) 走 pack_msg。
    let finish_hmac = {
        use hmac::digest::{KeyInit as _, Mac as _};
        type HmacSha256 = hmac::Hmac<sha2::Sha256>;
        let mut mac =
            <HmacSha256 as hmac::digest::KeyInit>::new_from_slice(LOCAL_KEY).expect("hmac key");
        mac.update(&remote_nonce);
        mac.finalize().into_bytes().to_vec()
    };
    let expect = v34_pack_ref(2, 0x05, &finish_hmac, LOCAL_KEY);
    assert_eq!(
        hex_decode(out["frame_hex"].as_str().unwrap()),
        expect,
        "SessKeyNegFinish 帧逐字节不一致"
    );
}

#[test]
#[cfg(feature = "tuya-plugin")]
fn lan_session_finish_rejects_tampered_hmac() {
    let mut resp_payload = craft_resp_payload(LOCAL_KEY, LOCAL_NONCE);
    resp_payload[20] ^= 0x01; // 篡改 HMAC 尾部一个字节
    let host = host();
    let out = call_lan(
        &host,
        "connector.tuya.lan-session-finish",
        json!({
            "req_id": "s3",
            "seqno": 3,
            "local_key_hex": hex_encode(LOCAL_KEY),
            "nonce_hex": hex_encode(LOCAL_NONCE),
            "resp_payload_hex": hex_encode(&resp_payload),
        }),
        "s3",
    );
    assert_eq!(out["ok"], json!(false), "{out}");
    let err = out["error"].as_str().unwrap_or_default();
    assert!(err.contains("HMAC"), "应显式报 HMAC 验证失败：{err}");
}

#[test]
#[cfg(feature = "tuya-plugin")]
fn lan_session_finish_rejects_short_payload() {
    let host = host();
    let out = call_lan(
        &host,
        "connector.tuya.lan-session-finish",
        json!({
            "req_id": "s4",
            "seqno": 4,
            "local_key_hex": hex_encode(LOCAL_KEY),
            "nonce_hex": hex_encode(LOCAL_NONCE),
            "resp_payload_hex": "aabbccdd",
        }),
        "s4",
    );
    assert_eq!(out["ok"], json!(false), "{out}");
    let err = out["error"].as_str().unwrap_or_default();
    assert!(err.contains("过短"), "应显式报回包过短：{err}");
}

/// 会话建立后的 v3.4 控制帧（modern envelope + ControlNew + "3.4" 头 +
/// HMAC 尾，密钥 = 会话密钥库 blob）。
#[test]
#[cfg(feature = "tuya-plugin")]
fn lan_command_v34_control_under_session_key_matches_rustuya() {
    let resp_payload = craft_resp_payload(LOCAL_KEY, LOCAL_NONCE);
    let (_remote_nonce, blob) = v34_session_derive_ref(&resp_payload);
    let dps = json!({"1": true});
    let t: u64 = 1_700_000_000;
    let dev_id = "ivf9id";

    // rustuya 参考：generate_payload(Control) → (ControlNew, envelope)。
    use rustuya::protocol::{CommandType, TuyaProtocol, Version};
    let proto = rustuya::protocol::get_protocol(Version::V3_4, Default::default());
    let (cmd, payload_value) = proto
        .generate_payload(dev_id, CommandType::Control, Some(dps.clone()), None, t)
        .expect("generate_payload");
    let payload_bytes = serde_json::to_vec(&payload_value).expect("serialize");
    let expect = v34_pack_ref(9, cmd, &payload_bytes, &blob);

    let host = host();
    let out = call_lan(
        &host,
        "connector.tuya.lan-command",
        json!({
            "req_id": "c1",
            "version": "3.4",
            "seqno": 9,
            "device_id": dev_id,
            "t": t,
            "command": "control",
            "dps": dps,
            "key_hex": hex_encode(&blob),
        }),
        "c1",
    );
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["cmd"], json!(cmd), "effective cmd 应为 ControlNew");
    assert_eq!(out["version"], json!("3.4"));
    assert_eq!(
        hex_decode(out["frame_hex"].as_str().unwrap()),
        expect,
        "v3.4 控制帧逐字节不一致"
    );
}

/// 子设备场景：cid 进 envelope 的 data（ctype=0）。
#[test]
#[cfg(feature = "tuya-plugin")]
fn lan_command_v34_control_with_cid_matches_rustuya() {
    let resp_payload = craft_resp_payload(LOCAL_KEY, LOCAL_NONCE);
    let (_remote_nonce, blob) = v34_session_derive_ref(&resp_payload);
    let dps = json!({"20": 25});
    let t: u64 = 1_700_000_001;
    let dev_id = "ivf9id";
    let cid = "subdevice01";

    use rustuya::protocol::{CommandType, TuyaProtocol, Version};
    let proto = rustuya::protocol::get_protocol(Version::V3_4, Default::default());
    let (cmd, payload_value) = proto
        .generate_payload(
            dev_id,
            CommandType::Control,
            Some(dps.clone()),
            Some(cid),
            t,
        )
        .expect("generate_payload");
    let payload_bytes = serde_json::to_vec(&payload_value).expect("serialize");
    let expect = v34_pack_ref(10, cmd, &payload_bytes, &blob);

    let host = host();
    let out = call_lan(
        &host,
        "connector.tuya.lan-command",
        json!({
            "req_id": "c2",
            "version": "3.4",
            "seqno": 10,
            "device_id": dev_id,
            "cid": cid,
            "t": t,
            "command": "control",
            "dps": dps,
            "key_hex": hex_encode(&blob),
        }),
        "c2",
    );
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(hex_decode(out["frame_hex"].as_str().unwrap()), expect);
}

/// 心跳帧：strip uid/t → `{gwId, devId}`，裸头命令，会话密钥 HMAC 尾。
#[test]
#[cfg(feature = "tuya-plugin")]
fn lan_heartbeat_v34_matches_rustuya() {
    let resp_payload = craft_resp_payload(LOCAL_KEY, LOCAL_NONCE);
    let (_remote_nonce, blob) = v34_session_derive_ref(&resp_payload);
    let dev_id = "ivf9id";

    use rustuya::protocol::{CommandType, TuyaProtocol, Version};
    let proto = rustuya::protocol::get_protocol(Version::V3_4, Default::default());
    let (cmd, payload_value) = proto
        .generate_payload(dev_id, CommandType::HeartBeat, None, None, 0)
        .expect("generate_payload");
    let payload_bytes = serde_json::to_vec(&payload_value).expect("serialize");
    let expect = v34_pack_ref(11, cmd, &payload_bytes, &blob);

    let host = host();
    let out = call_lan(
        &host,
        "connector.tuya.lan-command",
        json!({
            "req_id": "c3",
            "version": "3.4",
            "seqno": 11,
            "device_id": dev_id,
            "command": "heartbeat",
            "key_hex": hex_encode(&blob),
        }),
        "c3",
    );
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["cmd"], json!(0x09));
    assert_eq!(
        hex_decode(out["frame_hex"].as_str().unwrap()),
        expect,
        "v3.4 心跳帧逐字节不一致"
    );
}

/// UpdateDps：只留 cid + dpId（与 v3.3 同形，但 HMAC 尾 + "3.4" 头判定）。
#[test]
#[cfg(feature = "tuya-plugin")]
fn lan_command_v34_update_dps_with_cid_matches_rustuya() {
    let resp_payload = craft_resp_payload(LOCAL_KEY, LOCAL_NONCE);
    let (_remote_nonce, blob) = v34_session_derive_ref(&resp_payload);
    let dev_id = "ivf9id";
    let cid = "subdevice01";

    use rustuya::protocol::{CommandType, TuyaProtocol, Version};
    let proto = rustuya::protocol::get_protocol(Version::V3_4, Default::default());
    let (cmd, payload_value) = proto
        .generate_payload(dev_id, CommandType::UpdateDps, None, Some(cid), 0)
        .expect("generate_payload");
    let payload_bytes = serde_json::to_vec(&payload_value).expect("serialize");
    let expect = v34_pack_ref(12, cmd, &payload_bytes, &blob);

    let host = host();
    let out = call_lan(
        &host,
        "connector.tuya.lan-command",
        json!({
            "req_id": "c4",
            "version": "3.4",
            "seqno": 12,
            "device_id": dev_id,
            "cid": cid,
            "command": "update_dps",
            "key_hex": hex_encode(&blob),
        }),
        "c4",
    );
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["cmd"], json!(0x12));
    assert_eq!(hex_decode(out["frame_hex"].as_str().unwrap()), expect);
}
