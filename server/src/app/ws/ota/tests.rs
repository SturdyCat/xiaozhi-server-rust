//! `server/src/app/ws/ota.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::*;

    /// OTA 响应契约（锁定固件 main/ota.cc 解析所需的字段形状）：
    /// OTA 响应契约（锁定固件 main/ota.cc 解析所需的字段形状）：
/// websocket 为扁平对象 url/token/version；firmware 未托管时**回显设备版本** + url 空。
#[test]
fn ota_payload_contract() {
    let v = ota_payload("192.168.99.250:8000", "tk123", 1, Some("2.0.4"), "2.5.1");
    assert_eq!(v["websocket"]["url"], "ws://192.168.99.250:8000/api/ws");
    assert_eq!(v["websocket"]["token"], "tk123");
    assert_eq!(v["websocket"]["version"], 1);
    // 已托管固件：firmware 指向下载地址（设备版本比较通过即自动升级）
    assert_eq!(v["firmware"]["version"], "2.0.4");
    assert_eq!(v["firmware"]["url"], "http://192.168.99.250:8000/api/ota/firmware/latest");
    assert!(v["server_time"]["timestamp"].as_u64().unwrap() > 0);
    // 时间字段对齐官方实测约定：UTC epoch ms + offset=+480（固件 ts += offset*60s → 本地墙钟）
    assert_eq!(v["server_time"]["timezone_offset"], 480);
    // 未托管：**回显设备当前版本** + url 空 → 设备永不触发升级（官方实测行为）
    let none = ota_payload("h:1", "", 1, None, "2.5.1");
    assert_eq!(none["firmware"]["version"], "2.5.1");
    assert_eq!(none["firmware"]["url"], "");
    // 未配置鉴权（expected_token 空）：字段仍在，空 token 固件自动跳过 Authorization 头
    assert_eq!(none["websocket"]["token"], "");
    // 有意省略官方字段：mqtt（否则固件优先选 MQTT）、activation（免激活流程）
    assert!(v.get("mqtt").is_none());
    assert!(v.get("activation").is_none());
}

/// Device-Id 必需校验（官方：缺 → 400 {"success":false,"error":"Device ID is required"}）。
#[test]
fn device_id_required() {
    let empty = HeaderMap::new();
    assert_eq!(device_id_of(&empty), None, "缺头应为 None");
    let mut h = HeaderMap::new();
    h.insert("Device-Id", "  ".parse().unwrap());
    assert_eq!(device_id_of(&h), None, "空白串应视为缺失");
    h.insert("Device-Id", "84:fc:e6:7e:54:40".parse().unwrap());
    assert_eq!(device_id_of(&h).as_deref(), Some("84:fc:e6:7e:54:40"));
    // 错误体形状（官方同款字段）
    let err: serde_json::Value = serde_json::from_str(&ota_error_body("Device ID is required")).unwrap();
    assert_eq!(err["success"], false);
    assert_eq!(err["error"], "Device ID is required");
    assert_eq!(err["message"], "Device ID is required");
}
