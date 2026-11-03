//! `lan.rs` 纯函数单测：失败三归类、DP 载荷解析。**不连真机、不 sleep**。

use std::io::ErrorKind;

use rustuya::error::TuyaError;

use super::lan::*;

#[test]
fn timeout_error_maps_to_timeout_kind() {
    let f = classify(&TuyaError::Timeout);
    assert_eq!(f.kind, "timeout");
    assert!(!f.advice.is_empty());
}

#[test]
fn io_timed_out_maps_to_timeout_kind() {
    let f = classify(&TuyaError::io(ErrorKind::TimedOut, "5s"));
    assert_eq!(f.kind, "timeout");
}

#[test]
fn connection_refused_maps_to_offline() {
    for err in [
        TuyaError::io(ErrorKind::ConnectionRefused, "refused"),
        TuyaError::io(ErrorKind::HostUnreachable, "unreachable"),
        TuyaError::io(ErrorKind::NetworkUnreachable, "net down"),
        TuyaError::Offline,
        TuyaError::ConnectionFailed,
        TuyaError::DeviceNotFound("d1".into()),
    ] {
        let f = classify(&err);
        assert_eq!(f.kind, "offline", "{err:?} 应归 offline");
        assert!(!f.advice.is_empty());
    }
}

#[test]
fn protocol_failures_map_to_unsupported() {
    for err in [
        TuyaError::HandshakeFailed,
        TuyaError::KeyOrVersionError,
        TuyaError::HmacMismatch,
        TuyaError::DecryptionFailed,
        TuyaError::InvalidHeader,
        TuyaError::CrcMismatch,
    ] {
        let f = classify(&err);
        assert_eq!(f.kind, "unsupported", "{err:?} 应归 unsupported");
        assert!(f.advice.contains("local_key") || f.advice.contains("协议"), "{err:?} 建议动作应指向同步/协议");
    }
}

#[test]
fn failure_status_codes_are_distinct() {
    assert_eq!(ControlFailure::offline("x".into()).status_code(), 409);
    assert_eq!(classify(&TuyaError::Timeout).status_code(), 504);
    assert_eq!(classify(&TuyaError::HandshakeFailed).status_code(), 400);
}

#[test]
fn parse_dps_bare_object() {
    let dps = parse_dps(r#"{"1":true,"2":30,"3":"on"}"#);
    assert_eq!(dps.len(), 3);
    assert_eq!(dps["1"], serde_json::json!(true));
    assert_eq!(dps["2"], serde_json::json!(30));
    assert_eq!(dps["3"], serde_json::json!("on"));
}

#[test]
fn parse_dps_wrapped_object() {
    let dps = parse_dps(r#"{"dps":{"20":false},"t":123}"#);
    assert_eq!(dps.len(), 1);
    assert_eq!(dps["20"], serde_json::json!(false));
}

#[test]
fn parse_dps_garbage_yields_empty() {
    assert!(parse_dps("not json").is_empty());
    assert!(parse_dps("").is_empty());
    assert!(parse_dps("[1,2]").is_empty(), "非对象载荷按空处理");
}
