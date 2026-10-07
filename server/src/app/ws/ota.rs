//! 固件 OTA 引导端点（xiaozhi-esp32 开机约定，解析见固件 `main/ota.cc CheckVersion`）：
//! 设备携系统信息 POST 固件配置的 OTA_URL，响应引导其接入本服务的 WebSocket 并校时。
//!
//! 响应契约（固件逐段解析，缺段仅告警不阻塞）：
//! - `websocket`：**扁平对象** `{url, token, version}`；token 非空时固件自动加 `Bearer ` 前缀；
//!   `version` = 二进制协议版本（显式下发对齐固件，错位会导致下行 Opus 解析乱码）。
//! - `server_time`：`{timestamp: ms, timezone_offset: 分钟}`（offset 固定 +480 = UTC+8）。
//! - `firmware`：未托管固件时 url 空 → 永不升级；`app/firmware.rs` 有托管则给下载地址。
//! - `activation`：不返回 → 设备跳过激活流程。
//!
//! 拆分子模块（原 `ws.rs`）；路由注册见 [`super::router`]。

use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, HeaderMap},
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
    // 设备身份记录（Device-Id = MAC）：接入日志可核对是哪台设备
    let device_id = headers
        .get("Device-Id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    let firmware = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            let app = v.get("application")?;
            Some(format!(
                "{}/{}",
                app.get("name").and_then(|n| n.as_str()).unwrap_or("?"),
                app.get("version").and_then(|n| n.as_str()).unwrap_or("?")
            ))
        })
        .unwrap_or_else(|| "(无/解析失败)".to_string());
    tracing::info!("OTA 检查：device={device_id} firmware={firmware}");

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
    );
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        payload.to_string(),
    )
        .into_response()
}

/// OTA 响应体（独立纯函数：契约对齐固件解析，单测锁定字段形状）。
fn ota_payload(host: &str, expected_token: &str, bin_ver: u8, hosted: Option<&str>) -> serde_json::Value {
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
            // 未托管：url 空 → 固件只打 "Current is the latest version"，永不升级
            None => serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "url": "",
            }),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

        /// OTA 响应契约（锁定固件 main/ota.cc 解析所需的字段形状）：
        /// websocket 为扁平对象 url/token/version；firmware.url 恒空（自建端不托管固件升级）。
        #[test]
        fn ota_payload_contract() {
            let v = ota_payload("192.168.99.250:8000", "tk123", 1, Some("2.0.4"));
            assert_eq!(v["websocket"]["url"], "ws://192.168.99.250:8000/api/ws");
            assert_eq!(v["websocket"]["token"], "tk123");
            assert_eq!(v["websocket"]["version"], 1);
            // 已托管固件：firmware 指向下载地址（设备版本比较通过即自动升级）
            assert_eq!(v["firmware"]["version"], "2.0.4");
            assert_eq!(v["firmware"]["url"], "http://192.168.99.250:8000/api/ota/firmware/latest");
            assert!(v["server_time"]["timestamp"].as_u64().unwrap() > 0);
            // 未托管：url 空 → 设备永不触发升级
            let none = ota_payload("h:1", "", 1, None);
            assert_eq!(none["firmware"]["url"], "");
            // 未配置鉴权（expected_token 空）：字段仍在，空 token 固件自动跳过 Authorization 头
            assert_eq!(none["websocket"]["token"], "");
        }
}
