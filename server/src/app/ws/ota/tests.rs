//! `server/src/app/ws/ota.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::*;

/// 设备 OTA 请求体解析：功能/型号/板型/当前版本分别取自
/// `application.name`、`chip_model_name`、`board.type`、`application.version`
///（固件 `Board::GetSystemInfoJson` 的真实形状，2026-10 对齐 main 分支源码）。
#[test]
fn parse_device_reads_system_info_fields() {
    let body = r#"{
        "version": 2,
        "chip_model_name": "esp32s3",
        "application": {"name": "xiaozhi", "version": "1.6.4"},
        "board": {"type": "bread-compact-wifi", "name": "面包板紧凑型WIFI"}
    }"#;
    assert_eq!(
        parse_device(body),
        DeviceOta {
            function: "xiaozhi".into(),
            version: "1.6.4".into(),
            model: "esp32s3".into(),
            board: "bread-compact-wifi".into(),
        }
    );
    // 字段缺失 / body 非 JSON：全空 → 调用方据此不下发任何固件（不猜）
    assert_eq!(parse_device("{}"), DeviceOta::default());
    assert_eq!(parse_device("not json"), DeviceOta::default());
}

/// firmware 段：**只有同功能+同型号命中且版本更新**才给下载地址；
/// 否则回显设备当前版本 + 空 url（固件据此判定"已是最新"，不会拿空 url 去升级）。
#[test]
fn firmware_section_only_offers_newer_matched_firmware() {
    let dev = DeviceOta {
        function: "xiaozhi".into(),
        version: "1.6.4".into(),
        model: "esp32s3".into(),
        board: "bread-compact-wifi".into(),
    };
    let meta = FirmwareMeta {
        project: "xiaozhi".into(),
        model: "esp32s3".into(),
        board: String::new(),
        version: "2.0.4".into(),
        size: 1,
        sha256: "aa".into(),
        idf_ver: String::new(),
        built_at: String::new(),
        uploaded_at: 1,
    };
    // 命中 + 更新：版本 + 带匹配标识的下载地址（下载端会再校验一次）
    let v = firmware_section("192.168.99.250:8000", &dev, Some(&meta));
    assert_eq!(v["version"], "2.0.4");
    assert_eq!(
        v["url"],
        "http://192.168.99.250:8000/api/ota/firmware/latest?project=xiaozhi&model=esp32s3&board="
    );
    // 未命中（不同型号/不同功能/未托管）：回显设备版本 + 空 url
    let v = firmware_section("h:1", &dev, None);
    assert_eq!(v["version"], "1.6.4");
    assert_eq!(v["url"], "");
    // 设备版本未知：回显 "?"，同样不下发
    let unknown = DeviceOta {
        version: String::new(),
        ..dev.clone()
    };
    assert_eq!(firmware_section("h:1", &unknown, None)["version"], "?");
}

/// OTA 响应契约（锁定固件 main/ota.cc 解析所需的字段形状）：
/// websocket 为扁平对象 url/token/version；firmware 由调用方按设备信息算好传入。
#[test]
fn ota_payload_contract() {
    let firmware =
        serde_json::json!({"version": "2.0.4", "url": "http://h/api/ota/firmware/latest"});
    let v = ota_payload("192.168.99.250:8000", "tk123", 1, firmware);
    assert_eq!(v["websocket"]["url"], "ws://192.168.99.250:8000/api/ws");
    assert_eq!(v["websocket"]["token"], "tk123");
    assert_eq!(v["websocket"]["version"], 1);
    assert_eq!(v["firmware"]["version"], "2.0.4");
    assert_eq!(v["firmware"]["url"], "http://h/api/ota/firmware/latest");
    assert!(v["server_time"]["timestamp"].as_u64().unwrap() > 0);
    // 时间字段对齐官方实测约定：UTC epoch ms + offset=+480（固件 ts += offset*60s → 本地墙钟）
    assert_eq!(v["server_time"]["timezone_offset"], 480);
    // 未配置鉴权（expected_token 空）：字段仍在，空 token 固件自动跳过 Authorization 头
    let none = ota_payload(
        "h:1",
        "",
        1,
        serde_json::json!({"version": "1.0.0", "url": ""}),
    );
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
    let err: serde_json::Value =
        serde_json::from_str(&ota_error_body("Device ID is required")).unwrap();
    assert_eq!(err["success"], false);
    assert_eq!(err["error"], "Device ID is required");
    assert_eq!(err["message"], "Device ID is required");
}
