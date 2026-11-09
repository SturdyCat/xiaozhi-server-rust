//! S3f：tuya.official LAN 0x55AA 帧编解码 **rustuya 对拍**。
//!
//! 宿主侧参考实现 = 外部 crate rustuya（当前 native LAN 栈所有者，
//! `device/tuya/lan.rs` 的 async facade 之下）——用它的 `pack_message` /
//! `unpack_message` 在宿主生成黄金帧，与 guest 单源
//! `plugins-src/tuya/src/lan_codec.rs` 的输出逐字节对拍。这是 P8c LAN 域
//! 第一块**无需真机**的协议迁移证据；native 删除仍以真机固件回归为门槛。

use serde_json::{json, Value};
use std::sync::Arc;

use super::{ensure_fixture, fixture, host_with, ScriptedFetcher};
use crate::plugins::wasm::WasmHost;

const PLUGIN: &str = "tuya.official";
const LAN_PREFIX_KEY: &str = "tuya-lan";

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

fn host() -> (WasmHost, Arc<ScriptedFetcher>) {
    let fetcher = Arc::new(ScriptedFetcher::new());
    let host = host_with(&fetcher);
    host.load(PLUGIN, &ensure_fixture("tuya"))
        .expect("load tuya.official");
    (host, fetcher)
}

fn call_lan(host: &WasmHost, topic: &str, payload: Value, req_id: &str) -> Value {
    host.handle_event(PLUGIN, topic, &payload.to_string())
        .expect("lan dispatch");
    let bytes = host
        .kv()
        .get(PLUGIN, &format!("{LAN_PREFIX_KEY}:{req_id}"))
        .unwrap_or_else(|| panic!("LAN 结果未落 kv（{req_id}）"));
    serde_json::from_slice(&bytes).expect("结果应为合法 JSON")
}

/// rustuya 宿主参考：打包一个 0x55AA 帧。
#[cfg(feature = "tuya-plugin")]
fn rustuya_pack(seqno: u32, cmd: u32, payload: &[u8], hmac_key: Option<&[u8]>) -> Vec<u8> {
    use rustuya::protocol::{pack_message, TuyaMessage};

    let msg = TuyaMessage {
        seqno,
        cmd,
        payload: payload.to_vec(),
        retcode: None,
        prefix: rustuya::protocol::PREFIX_55AA,
        iv: None,
    };
    pack_message(&msg, hmac_key).expect("rustuya pack")
}

#[cfg(feature = "tuya-plugin")]
#[test]
fn lan_pack_plaintext_frame_matches_rustuya_byte_for_byte() {
    let (host, _fetcher) = host();
    let payload = br#"{"dps":{"1":true}}"#;
    let reference = rustuya_pack(42, 7, payload, None);

    let v = call_lan(
        &host,
        "connector.tuya.lan-pack",
        json!({"req_id": "lf1", "seqno": 42, "cmd": 7,
               "payload_hex": hex_encode(payload)}),
        "lf1",
    );
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(
        hex_decode(v["frame_hex"].as_str().expect("frame hex")),
        reference,
        "guest 帧必须与 rustuya 参考逐字节一致"
    );
}

#[cfg(feature = "tuya-plugin")]
#[test]
fn lan_pack_hmac_variant_matches_rustuya_byte_for_byte() {
    let (host, _fetcher) = host();
    let payload = br#"{"dps":{"1":true}}"#;
    let key = b"0123456789abcdef";
    let reference = rustuya_pack(43, 8, payload, Some(key));

    let v = call_lan(
        &host,
        "connector.tuya.lan-pack",
        json!({"req_id": "lf2", "seqno": 43, "cmd": 8,
               "payload_hex": hex_encode(payload),
               "hmac_key_hex": hex_encode(key)}),
        "lf2",
    );
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(
        hex_decode(v["frame_hex"].as_str().expect("frame hex")),
        reference,
        "HMAC 变体帧必须与 rustuya 参考逐字节一致"
    );
}

#[cfg(feature = "tuya-plugin")]
#[test]
fn lan_parse_matches_rustuya_unpack_semantics() {
    let (host, _fetcher) = host();

    // ① 命令帧（payload 以 '{' 开头 → retcode=None）
    let command = rustuya_pack(44, 9, br#"{"dps":{"1":false}}"#, None);
    let v = call_lan(
        &host,
        "connector.tuya.lan-parse",
        json!({"req_id": "lf3", "data_hex": hex_encode(&command)}),
        "lf3",
    );
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["seqno"], json!(44), "{v}");
    assert_eq!(v["cmd"], json!(9), "{v}");
    assert_eq!(v["retcode"], Value::Null, "命令帧不应启发式出 retcode：{v}");
    assert_eq!(
        hex_decode(v["payload_hex"].as_str().expect("payload")),
        br#"{"dps":{"1":false}}"#.to_vec(),
        "{v}"
    );

    // ② 应答帧（payload 以 retcode u32(0) 开头 → retcode=Some(0)，余量为 JSON）
    let mut response_payload = 0u32.to_be_bytes().to_vec();
    response_payload.extend_from_slice(br#"{}"#);
    let response = rustuya_pack(45, 10, &response_payload, None);
    let v = call_lan(
        &host,
        "connector.tuya.lan-parse",
        json!({"req_id": "lf4", "data_hex": hex_encode(&response)}),
        "lf4",
    );
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["retcode"], json!(0), "应答帧应启发式出 retcode=0：{v}");
    assert_eq!(
        hex_decode(v["payload_hex"].as_str().expect("payload")),
        br#"{}"#.to_vec(),
        "{v}"
    );

    // ③ HMAC 帧解析（key 传入才可校验）
    let key = b"0123456789abcdef";
    let hmac_frame = rustuya_pack(46, 11, br#"{"x":1}"#, Some(key));
    let v = call_lan(
        &host,
        "connector.tuya.lan-parse",
        json!({"req_id": "lf5", "data_hex": hex_encode(&hmac_frame),
               "hmac_key_hex": hex_encode(key)}),
        "lf5",
    );
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["seqno"], json!(46), "{v}");
}

#[cfg(feature = "tuya-plugin")]
#[test]
fn lan_parse_rejects_crc_corruption() {
    let (host, _fetcher) = host();
    // 用 rustuya 打合法帧，再翻转 CRC 尾一个字节 → ok:false（fail-closed）
    let mut frame = rustuya_pack(47, 12, br#"{}"#, None);
    let crc_last = frame.len() - 5; // crc(4) + suffix(4) 的最后一个 CRC 字节
    frame[crc_last] ^= 0xff;
    let v = call_lan(
        &host,
        "connector.tuya.lan-parse",
        json!({"req_id": "lf6", "data_hex": hex_encode(&frame)}),
        "lf6",
    );
    assert_eq!(v["ok"], json!(false), "CRC 翻转必须被拒：{v}");
    assert!(v["error"].as_str().unwrap().contains("CRC"), "{v}");
}

#[test]
fn lan_parse_rejects_foreign_prefix() {
    let (host, _fetcher) = host();
    // ≥16 字节、合法形状但前缀是 0x6699 → 前缀拒绝（16 字节内判不出长度谎报）
    let mut foreign = vec![0u8; 24];
    foreign[2] = 0x66;
    foreign[3] = 0x99;
    let v = call_lan(
        &host,
        "connector.tuya.lan-parse",
        json!({"req_id": "lf7", "data_hex": hex_encode(&foreign)}),
        "lf7",
    );
    assert_eq!(v["ok"], json!(false), "外来前缀必须被拒：{v}");
    assert!(v["error"].as_str().unwrap().contains("0x55AA"), "{v}");
}

// ---- S3g：0x6699（3.5/AES-128-GCM）帧编解码对拍（固定 IV，guest 无 rand）----

#[cfg(feature = "tuya-plugin")]
const GCM_KEY: &[u8] = b"0123456789abcdef";
#[cfg(feature = "tuya-plugin")]
const GCM_IV: &[u8] = b"fixed12bytes";

/// rustuya 宿主参考：打包一个 0x6699 帧（IV 显式传入，确定性）。
#[cfg(feature = "tuya-plugin")]
fn rustuya_pack_6699(
    seqno: u32,
    cmd: u32,
    retcode: Option<u32>,
    payload: &[u8],
    key: &'static [u8],
    iv: &'static [u8],
) -> Vec<u8> {
    use rustuya::protocol::{pack_message, TuyaMessage};

    let msg = TuyaMessage {
        seqno,
        cmd,
        payload: payload.to_vec(),
        retcode,
        prefix: rustuya::protocol::PREFIX_6699,
        iv: Some(iv.to_vec()),
    };
    pack_message(&msg, Some(key)).expect("rustuya 6699 pack")
}

#[cfg(feature = "tuya-plugin")]
#[test]
fn lan_pack_6699_matches_rustuya_byte_for_byte() {
    let (host, _fetcher) = host();
    let reference = rustuya_pack_6699(52, 9, Some(0), br#"{"dps":{"1":true}}"#, GCM_KEY, GCM_IV);

    let v = call_lan(
        &host,
        "connector.tuya.lan-pack",
        json!({"req_id": "g1", "format": "6699", "seqno": 52, "cmd": 9,
               "retcode": 0, "payload_hex": hex_encode(br#"{"dps":{"1":true}}"#),
               "key_hex": hex_encode(GCM_KEY), "iv_hex": hex_encode(GCM_IV)}),
        "g1",
    );
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(
        hex_decode(v["frame_hex"].as_str().expect("frame hex")),
        reference,
        "guest 6699 帧必须与 rustuya 参考逐字节一致"
    );
}

#[cfg(feature = "tuya-plugin")]
#[test]
fn lan_parse_6699_matches_rustuya_unpack() {
    let (host, _fetcher) = host();
    // rustuya 打应答帧（retcode=0）→ guest 解析字段全对齐
    let reference = rustuya_pack_6699(53, 10, Some(0), br#"{}"#, GCM_KEY, GCM_IV);
    let v = call_lan(
        &host,
        "connector.tuya.lan-parse",
        json!({"req_id": "g2", "format": "6699", "data_hex": hex_encode(&reference),
               "key_hex": hex_encode(GCM_KEY)}),
        "g2",
    );
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["seqno"], json!(53), "{v}");
    assert_eq!(v["cmd"], json!(10), "{v}");
    assert_eq!(v["retcode"], json!(0), "{v}");
    assert_eq!(
        hex_decode(v["payload_hex"].as_str().expect("payload")),
        br#"{}"#.to_vec(),
        "{v}"
    );
    assert_eq!(
        hex_decode(v["iv_hex"].as_str().expect("iv")),
        GCM_IV.to_vec(),
        "{v}"
    );

    // 反向：guest 打的帧由 rustuya unpack 解出（双向等价公民证据）
    let pack_v = call_lan(
        &host,
        "connector.tuya.lan-pack",
        json!({"req_id": "g3", "format": "6699", "seqno": 54, "cmd": 11,
               "retcode": 0, "payload_hex": hex_encode(br#"{"dps":{"2":30}}"#),
               "key_hex": hex_encode(GCM_KEY), "iv_hex": hex_encode(GCM_IV)}),
        "g3",
    );
    let guest_frame = hex_decode(pack_v["frame_hex"].as_str().expect("frame"));
    let msg = rustuya::protocol::unpack_message(&guest_frame, Some(GCM_KEY), None, None)
        .expect("rustuya 应能解开 guest 打的 6699 帧");
    assert_eq!(msg.seqno, 54);
    assert_eq!(msg.cmd, 11);
    assert_eq!(msg.retcode, Some(0));
    assert_eq!(msg.payload, br#"{"dps":{"2":30}}"#.to_vec());
}

#[cfg(feature = "tuya-plugin")]
#[test]
fn lan_parse_6699_rejects_tampered_ciphertext() {
    let (host, _fetcher) = host();
    let mut frame = rustuya_pack_6699(55, 12, Some(0), br#"{}"#, GCM_KEY, GCM_IV);
    let mid = frame.len() - 10; // 翻转 ct 尾部一个字节（suffix 之前）
    frame[mid] ^= 0xff;
    let v = call_lan(
        &host,
        "connector.tuya.lan-parse",
        json!({"req_id": "g4", "format": "6699", "data_hex": hex_encode(&frame),
               "key_hex": hex_encode(GCM_KEY)}),
        "g4",
    );
    assert_eq!(v["ok"], json!(false), "GCM tag 篡改必须被拒：{v}");
    assert!(v["error"].as_str().unwrap().contains("GCM"), "{v}");
}

// ---- S3h：v3.3 控制命令全链路（载荷形状 → ECB → 帧）rustuya 对拍 ----

#[cfg(feature = "tuya-plugin")]
#[test]
fn lan_command_v33_matches_rustuya_byte_for_byte() {
    use rustuya::protocol::{
        get_protocol, pack_message, CommandType, DeviceType, TuyaMessage, Version,
    };

    let (host, _fetcher) = host();
    let local_key = b"0123456789abcdef";
    let cipher = rustuya::crypto::TuyaCipher::new(local_key).expect("cipher");
    let protocol = get_protocol(Version::V3_3, DeviceType::Default);

    // rustuya 参考：generate_payload → pack_payload → pack_message
    let dps = json!({"1": true});
    let (cmd, payload) = protocol
        .generate_payload(
            "device-1",
            CommandType::Control,
            Some(dps),
            None,
            1_700_000_000,
        )
        .expect("generate payload");
    let packed = protocol
        .pack_payload(payload.to_string().as_bytes(), cmd, &cipher)
        .expect("pack payload");
    let msg = TuyaMessage {
        seqno: 61,
        cmd,
        retcode: None,
        payload: packed,
        prefix: rustuya::protocol::PREFIX_55AA,
        iv: None,
    };
    let reference = pack_message(&msg, protocol.get_hmac_key(local_key)).expect("rustuya pack");

    let v = call_lan(
        &host,
        "connector.tuya.lan-command",
        json!({"req_id": "c1", "seqno": 61, "device_id": "device-1",
               "t": 1_700_000_000u64, "command": "control",
               "dps": {"1": true}, "key_hex": hex_encode(local_key)}),
        "c1",
    );
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["cmd"], json!(cmd), "effective cmd 应一致：{v}");
    assert_eq!(
        hex_decode(v["frame_hex"].as_str().expect("frame hex")),
        reference,
        "v3.3 控制命令帧必须与 rustuya 参考逐字节一致"
    );
}

#[cfg(feature = "tuya-plugin")]
#[test]
fn lan_command_v33_update_dps_matches_rustuya() {
    use rustuya::protocol::{
        get_protocol, pack_message, CommandType, DeviceType, TuyaMessage, Version,
    };

    let (host, _fetcher) = host();
    let local_key = b"0123456789abcdef";
    let cipher = rustuya::crypto::TuyaCipher::new(local_key).expect("cipher");
    let protocol = get_protocol(Version::V3_3, DeviceType::Default);

    let (cmd, payload) = protocol
        .generate_payload(
            "device-2",
            CommandType::UpdateDps,
            Some(json!({"2": 30})),
            None,
            1_700_000_100,
        )
        .expect("generate payload");
    let packed = protocol
        .pack_payload(payload.to_string().as_bytes(), cmd, &cipher)
        .expect("pack payload");
    let msg = TuyaMessage {
        seqno: 62,
        cmd,
        retcode: None,
        payload: packed,
        prefix: rustuya::protocol::PREFIX_55AA,
        iv: None,
    };
    let reference = pack_message(&msg, protocol.get_hmac_key(local_key)).expect("rustuya pack");

    let v = call_lan(
        &host,
        "connector.tuya.lan-command",
        json!({"req_id": "c2", "seqno": 62, "device_id": "device-2",
               "t": 1_700_000_100u64, "command": "update_dps",
               "dps": {"2": 30}, "key_hex": hex_encode(local_key)}),
        "c2",
    );
    assert_eq!(v["ok"], json!(true), "{v}");
    assert_eq!(v["cmd"], json!(cmd), "{v}");
    assert_eq!(
        hex_decode(v["frame_hex"].as_str().expect("frame hex")),
        reference,
        "update_dps 帧必须与 rustuya 参考逐字节一致"
    );
}
