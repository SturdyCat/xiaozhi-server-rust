//! `server/src/app/protocol.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

/// tts_test 的 vcn 字段（测试台音色覆盖）：序列化可用、解析缺省为 None。
#[test]
fn tts_test_vcn_roundtrip() {
    let with: ClientMessage = serde_json::from_str(
        r#"{"type":"tts_test","text":"你好","speaker":3,"lang":"zh","speed":1.0,"vcn":"x6_lingxiaoyue_pro"}"#,
    )
    .unwrap();
    match with {
        ClientMessage::TtsTest { vcn: Some(v), .. } => assert_eq!(v, "x6_lingxiaoyue_pro"),
        other => panic!("解析失败: {other:?}"),
    }
    // 老客户端不带 vcn：缺省 None（服务端回退已保存配置）
    let without: ClientMessage =
        serde_json::from_str(r#"{"type":"tts_test","text":"你好"}"#).unwrap();
    match without {
        ClientMessage::TtsTest { vcn: None, .. } => {}
        other => panic!("缺省应 None: {other:?}"),
    }
}
use super::*;

#[test]
fn bin_v1_roundtrip() {
    let opus = [1u8, 2, 3, 4, 5];
    let wrapped = wrap_downlink(BinVersion::V1, &opus, 0);
    assert_eq!(wrapped, opus);
    assert_eq!(unwrap_uplink(BinVersion::V1, &wrapped), &opus);
}

/// ⚠️ 字节序回归（对齐固件 `htons/htonl`，勿改回小端）：逐字节核对 v2/v3 布局。
#[test]
fn bin_v2_v3_big_endian_layout() {
    let opus = [0xAAu8, 0xBB, 0xCC];
    // v2: version=2(大端 00 02，等于协商版本号) | type=0 | reserved=0 | ts(大端) | size(大端)
    let v2 = wrap_downlink(BinVersion::V2, &opus, 0x0102_0304);
    assert_eq!(&v2[0..2], &[0x00, 0x02], "version 必须大端且等于协商版本");
    assert_eq!(&v2[2..4], &[0x00, 0x00], "type=OPUS（大端）");
    assert_eq!(&v2[4..8], &[0x00, 0x00, 0x00, 0x00], "reserved（大端）");
    assert_eq!(&v2[8..12], &[0x01, 0x02, 0x03, 0x04], "timestamp 必须大端");
    assert_eq!(&v2[12..16], &[0x00, 0x00, 0x00, 0x03], "payload_size 必须大端");
    assert_eq!(&v2[16..], &opus);
    // 上行剥离同字节序
    assert_eq!(unwrap_uplink(BinVersion::V2, &v2), &opus);
    // v3: type | reserved | size(大端)
    let v3 = wrap_downlink(BinVersion::V3, &opus, 0);
    assert_eq!(&v3[0..2], &[0x00, 0x00], "v3 type/reserved");
    assert_eq!(&v3[2..4], &[0x00, 0x03], "v3 payload_size 必须大端");
    assert_eq!(&v3[4..], &opus);
    assert_eq!(unwrap_uplink(BinVersion::V3, &v3), &opus);
}

#[test]
fn bin_v2_roundtrip() {
    let opus = [9u8, 8, 7, 6];
    let wrapped = wrap_downlink(BinVersion::V2, &opus, 1234);
    assert_eq!(wrapped.len(), 16 + opus.len());
    // v2 帧首 2 字节是 version 字段 = 2，**网络序**（对齐固件 htons(2)；旧断言误用小端）。
    assert_eq!(u16::from_be_bytes([wrapped[0], wrapped[1]]), 2);
    assert_eq!(unwrap_uplink(BinVersion::V2, &wrapped), &opus);
}

#[test]
fn bin_v3_roundtrip() {
    let opus = [4u8, 5, 6];
    let wrapped = wrap_downlink(BinVersion::V3, &opus, 0);
    assert_eq!(wrapped.len(), 4 + opus.len());
    assert_eq!(unwrap_uplink(BinVersion::V3, &wrapped), &opus);
}

#[test]
fn llm_test_client_message_deserializes_snake_case() {
    // ClientMessage 用 rename_all = "snake_case"：多单词变体必须是 llm_test（小写下划线），
    // 写错（如 llmTest）会在运行时报 unknown variant 且 cargo check 无法发现。
    let cm: ClientMessage =
        serde_json::from_str(r#"{"type":"llm_test","text":"你好"}"#).expect("解析失败");
    assert!(
        matches!(cm, ClientMessage::LlmTest { text, .. } if text == "你好"),
        "应反序列化为 LlmTest 变体"
    );
}

#[test]
fn llm_test_result_serializes_with_snake_case_tag() {
    // ServerMessage 用 rename_all = "lowercase"（LlmTestResult 默认会变成 "llmtest"），
    // 显式 rename 后 tag 必须是 "llm_test"，与上行命名对称。
    let sm = ServerMessage::LlmTestResult {
        session_id: "s1".to_string(),
        state: "ok".to_string(),
        text: Some("回复".to_string()),
        elapsed_ms: Some(123),
    };
    let v: serde_json::Value = serde_json::from_str(&sm.to_json()).expect("序列化失败");
    assert_eq!(v["type"], "llm_test");
    assert_eq!(v["session_id"], "s1");
    assert_eq!(v["state"], "ok");
    assert_eq!(v["text"], "回复");
    assert_eq!(v["elapsed_ms"], 123);
}
