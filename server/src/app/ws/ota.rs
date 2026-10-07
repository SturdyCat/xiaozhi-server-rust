//! 固件 OTA 引导端点（xiaozhi-esp32 开机约定，解析见固件 `main/ota.cc CheckVersion`）：
//! 设备携系统信息 POST 固件配置的 OTA_URL，响应引导其接入本服务的 WebSocket 并校时。
//!
//! ## 响应契约（对齐官方 `api.tenclass.net/xiaozhi/ota/`，2026-10-07 逐项实测）
//! - `websocket`：**扁平对象** `{url, token, version}`；token 非空时固件自动加 `Bearer ` 前缀。
//!   `version` 是**我方扩展**（官方不下发，固件读 NVS 缺省 1）：显式下发
//!   `[audio].binary_protocol_version`，切 v2/v3 时设备才能跟着切（v2/v3 帧为**网络序**，
//!   见 `app/protocol.rs`）。
//! - `server_time`：`{timestamp: ms, timezone_offset: 分钟}`。**官方实测约定**：timestamp = 真 UTC
//!   epoch ms、offset = **+480**（固件 `ts += offset*60s` 后 settimeofday → 本地墙钟）。
//!   （官方文档示例里的 `-480` 是笔误；实测服务器返回 +480，别照文档改。）
//! - `firmware`：未托管固件时**回显设备当前版本** + url 空（官方实测行为；固件据此打
//!   "Current is the latest version" 且永不升级——若此处回服务器自身版本，低于它的老固件
//!   会拿空 url 尝试升级并弹错误）。`app/firmware.rs` 有托管则给版本 + 下载地址（自升级）。
//! - **有意省略**（与官方的差异，均为让 ESP 走本服务 WebSocket）：
//!   `mqtt`（官方返回 → 固件优先选 MQTT）；`activation`（官方激活流程，自建端不需要）。
//! - `Device-Id` 必需（官方缺时 400 `{"success":false,"error":"Device ID is required",...}`，
//!   我方对齐）；`Client-Id` 官方实际不校验（实测缺失仍 200），我方同样不强制。
//!
//! 拆分子模块（原 `ws.rs`）；路由注册见 [`super::router`]。

use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use crate::engine::Engines;

/// OTA/版本检查端点（xiaozhi-esp32 固件开机约定，解析见固件 `main/ota.cc CheckVersion`）：
/// 设备携系统信息 POST 固件配置的 OTA_URL，响应引导其接入本服务的 WebSocket 并校时。
///
/// 响应契约（固件逐段解析，缺段仅告警不阻塞）：
/// - `websocket`：**扁平对象** `{url, token, version}`，固件原样写入 NVS，
///   `OpenAudioChannel` 读取——token 非空时固件自动加 `Bearer ` 前缀（对齐 /api/ws 鉴权）；
///   `version` = 二进制协议版本，**显式下发对齐固件**（不传则固件用内置默认，可能与
///   `[audio].binary_protocol_version` 错位，下行 Opus 会被解析成乱码）。
/// - `server_time`：`{timestamp: ms, timezone_offset: 分钟}`，固件 settimeofday 校时；
///   offset 固定 +480（UTC+8，部署面为中国局域网，不校时仅日志/显示时间漂移）。
/// - `firmware`：`{version, url}`——url 恒空 + 版本取服务端包版本 → 永不触发固件升级，
///   仅让固件日志打出 "Current is the latest version"。
/// - `activation`：不返回 → 设备跳过激活流程（那是 xiaozhi.me 官方服务器的机制）。
pub(super) async fn ota(State(engines): State<Arc<Engines>>, headers: HeaderMap, body: String) -> Response {
    // Device-Id 必需（官方同款校验与错误形状：缺 → 400 JSON）
    let Some(device_id) = device_id_of(&headers) else {
        tracing::warn!("OTA 请求缺 Device-Id 头，按官方约定返回 400");
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            ota_error_body("Device ID is required"),
        )
            .into_response();
    };
    // 设备上报的 `application`（name/version）：version 用于 firmware 段回显
    let (app_name, app_version) = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            let app = v.get("application")?;
            Some((
                app.get("name").and_then(|n| n.as_str()).unwrap_or("?").to_string(),
                app.get("version").and_then(|n| n.as_str()).unwrap_or("?").to_string(),
            ))
        })
        .unwrap_or_else(|| ("?".to_string(), "?".to_string()));
    tracing::info!("OTA 检查：device={device_id} firmware={app_name}/{app_version}");

    // WebSocket 地址跟随请求 Host：设备用哪个地址访问 OTA，就用哪个地址连 WS
    //（部署 IP/端口变化零维护）。HTTP/1.1 必带 Host；缺失兜底 localhost 并告警。
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            tracing::warn!("OTA 请求缺 Host 头，websocket url 兜底 localhost（设备可能连不上）");
            format!("localhost:{}", engines.config.server.port)
        });

    let hosted = crate::app::firmware::latest().map(|m| m.version);
    let payload = ota_payload(
        &host,
        &engines.config.server.expected_token,
        engines.config.audio.binary_protocol_version,
        hosted.as_deref(),
        &app_version,
    );
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        payload.to_string(),
    )
        .into_response()
}

/// Device-Id 取值（缺头/空串 → None；官方视为必需）。
fn device_id_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get("Device-Id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// OTA 错误体（官方形状：`{"success":false,"error":...,"message":...}`，实测 400 同款）。
fn ota_error_body(msg: &str) -> String {
    serde_json::json!({ "success": false, "error": msg, "message": msg }).to_string()
}

/// OTA 响应体（独立纯函数：契约对齐固件解析，单测锁定字段形状）。
fn ota_payload(
    host: &str,
    expected_token: &str,
    bin_ver: u8,
    hosted: Option<&str>,
    device_version: &str,
) -> serde_json::Value {
    serde_json::json!({
        "websocket": {
            "url": format!("ws://{host}/api/ws"),
            "token": expected_token,
            "version": bin_ver,
        },
        "server_time": {
            "timestamp": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            "timezone_offset": 480,
        },
        "firmware": match hosted {
            // 已托管固件：下发版本 + 下载地址，设备 IsNewVersionAvailable 判定后自动升级
            Some(v) => serde_json::json!({
                "version": v,
                "url": format!("http://{host}/api/ota/firmware/latest"),
            }),
            // 未托管：**回显设备当前版本** + url 空（官方实测行为）→ 固件打
            // "Current is the latest version" 且永不升级。勿回服务端自身版本：
            // 比它低的老固件会拿空 url 尝试升级（弹错误）。
            None => serde_json::json!({
                "version": device_version,
                "url": "",
            }),
        },
    })
}

#[cfg(test)]
mod tests {
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
}
