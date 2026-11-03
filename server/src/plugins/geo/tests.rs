//! 出口 IP 定位解析的离线测试（不联网）。

use super::{parse_ipip, parse_ipip_parts};

#[test]
fn test_parse_ipip_domestic() {
    let text = "当前 IP：1.2.3.4 来自于：中国 广东 深圳 电信";
    let loc = parse_ipip(text).expect("应为国内定位");
    assert_eq!(loc.ip, "1.2.3.4");
    assert_eq!(loc.country, "中国");
    assert_eq!(loc.province, "广东");
    assert_eq!(loc.city, "深圳");
    assert_eq!(loc.source, "myip.ipip.net");
}

#[test]
fn test_parse_ipip_foreign_returns_none() {
    // 国外出口：parse_ipip 返回 None（不写定位）。
    let text = "IP：1.2.3.4 来自于：United States California Menlo Park";
    assert!(parse_ipip(text).is_none());
    // 结构层可区分「国外」与「格式不符」，供调用方决定是否兜底。
    let parts = parse_ipip_parts(text).expect("结构应可解析");
    assert!(!parts.1.contains("中国"));
}

#[test]
fn test_parse_ipip_no_prefix() {
    // 兼容无「当前 」前缀。
    let text = "IP：10.0.0.1 来自于：中国 北京 北京 联通";
    let loc = parse_ipip(text).expect("应为国内定位");
    assert_eq!(loc.ip, "10.0.0.1");
    assert_eq!(loc.city, "北京");
}

#[test]
fn test_parse_ipip_bad_format_returns_none() {
    // 格式不符（如被劫持页/接口变更）：结构解析失败。
    assert!(parse_ipip_parts("<html>blocked</html>").is_none());
    assert!(parse_ipip("").is_none());
}
