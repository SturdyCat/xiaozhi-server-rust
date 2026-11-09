//! `dps.rs` 纯函数单测：DP 载荷两种形状与坏输入。**不连真机、不 sleep**。

use super::dps::parse_dps;

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
