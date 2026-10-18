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
//! - `firmware`：**按设备上报的「功能 + 型号」匹配**托管固件（见 `app/firmware.rs`）。设备 body
//!   里 `application.name`（= 固件 `esp_app_desc_t.project_name`）是功能，`chip_model_name`
//!   （= `CONFIG_IDF_TARGET`，如 `esp32s3`）是型号，`board.type` 是板型（可选更细粒度）。
//!   命中**且托管版本比设备当前版本新** → 下发 `{version, url}`（url 带匹配标识）；
//!   否则**回显设备当前版本 + url 空**（官方实测行为：固件据此打 "Current is the latest
//!   version" 且永不升级）。
//!   ⚠️ **不同型号 / 不同功能的包绝不互发**：c3 与 s3 的镜像不可互换，发错包等于变砖。
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

use crate::app::firmware::FirmwareMeta;
use crate::engine::Engines;

/// 设备 OTA 请求里与固件匹配相关的字段（固件 `Board::GetSystemInfoJson` 的 POST body）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DeviceOta {
    /// `application.name`（= 固件 `esp_app_desc_t.project_name`）：功能标识。
    function: String,
    /// `application.version`：设备当前固件版本。
    version: String,
    /// `chip_model_name`（= `CONFIG_IDF_TARGET`，如 `esp32s3`）：型号。
    model: String,
    /// `board.type`（如 `bread-compact-wifi`）：板型（更细粒度，可为空）。
    board: String,
}

/// OTA/版本检查端点（xiaozhi-esp32 固件开机约定，解析见固件 `main/ota.cc CheckVersion`）：
/// 设备携系统信息 POST 固件配置的 OTA_URL，响应引导其接入本服务的 WebSocket 并校时。
pub(super) async fn ota(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    body: String,
) -> Response {
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
    // 设备上报的系统信息：功能 / 型号 / 板型 / 当前版本（解析失败只影响升级判定，不阻塞握手）
    let dev = parse_device(&body);
    tracing::info!(
        "OTA 检查：device={device_id} function={} model={} board={} firmware={}",
        if dev.function.is_empty() {
            "?"
        } else {
            &dev.function
        },
        if dev.model.is_empty() {
            "?"
        } else {
            &dev.model
        },
        if dev.board.is_empty() {
            "-"
        } else {
            &dev.board
        },
        if dev.version.is_empty() {
            "?"
        } else {
            &dev.version
        },
    );

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

    // 同功能 + 同型号（可选板型精确匹配）才谈升级；版本必须真的更新，否则回显设备版本
    let matched = crate::app::firmware::find(&dev.function, &dev.model, &dev.board);
    let upgrade = matched
        .as_ref()
        .filter(|m| crate::app::firmware::is_newer(&m.version, &dev.version));
    match upgrade {
        Some(m) => tracing::info!(
            "OTA 命中托管固件：project={} model={} board={} {} → {}（device={device_id}）",
            m.project,
            m.model,
            if m.board.is_empty() {
                "（通用）"
            } else {
                &m.board
            },
            dev.version,
            m.version,
        ),
        None => tracing::info!(
            "OTA 无可用升级（function={} model={} board={}）：{}",
            dev.function,
            dev.model,
            dev.board,
            match matched {
                Some(_) => "已托管同型号固件但版本不更新",
                None => "未托管该功能+型号的固件",
            }
        ),
    }

    let payload = ota_payload(
        &host,
        &engines.config.server.expected_token,
        engines.config.audio.binary_protocol_version,
        firmware_section(&host, &dev, upgrade),
    );
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        payload.to_string(),
    )
        .into_response()
}

/// 解析设备上报的系统信息（缺字段 → 空串；body 非 JSON 也照常返回默认值）。
fn parse_device(body: &str) -> DeviceOta {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return DeviceOta::default();
    };
    let field = |ptr: &str| {
        v.pointer(ptr)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    };
    DeviceOta {
        function: field("/application/name"),
        version: field("/application/version"),
        model: field("/chip_model_name"),
        board: field("/board/type"),
    }
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

/// `firmware` 段：命中且更新 → 版本 + 下载地址（带匹配标识，下载端据此再校验一次）；
/// 否则回显设备当前版本 + 空 url。
///
/// 未命中时**必须回显设备版本**（不能回服务端自身版本）：固件会拿"新版本号 + 空 url"去
/// 尝试升级并弹错误；回显设备版本才让它安静地判定"已是最新"。
fn firmware_section(
    host: &str,
    dev: &DeviceOta,
    upgrade: Option<&FirmwareMeta>,
) -> serde_json::Value {
    match upgrade {
        Some(m) => serde_json::json!({
            "version": m.version,
            "url": format!(
                "http://{host}/api/ota/firmware/latest?project={}&model={}&board={}",
                m.project, m.model, m.board
            ),
        }),
        None => serde_json::json!({
            "version": if dev.version.is_empty() { "?" } else { dev.version.as_str() },
            "url": "",
        }),
    }
}

/// OTA 响应体（独立纯函数：契约对齐固件解析，单测锁定字段形状）。
fn ota_payload(
    host: &str,
    expected_token: &str,
    bin_ver: u8,
    firmware: serde_json::Value,
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
        "firmware": firmware,
    })
}

#[cfg(test)]
mod tests;
