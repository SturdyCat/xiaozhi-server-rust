//! `/api/devices/{id}/overlay` 端点组（P1-5）：每设备「会话内参数」增量，
//! 落盘于 config 同目录 `overlays/devices/<device_id>.toml`；仅白名单字段可覆盖
//! （见 `crate::config::overlay`）。不改变 `PUT /api/config` 的部分更新语义。
//!
//! 从父模块 `config.rs` 拆出（AGENTS.md §5.10 文件规模约定：行为不变的搬移）。

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::config::overlay::{device_overlay_path, json_to_toml, read_device_overlay_value, validate_overlay};
use crate::engine::Engines;

use super::{json_msg, json_ok};

// ============================================================
// 设备 overlay（P1-5）：每设备「会话内参数」增量，落盘于 config 同目录
// `overlays/devices/<device_id>.toml`；仅白名单字段可覆盖（见 `crate::config::overlay`）。
// 不改变 `PUT /api/config` 的部分更新语义。
// ============================================================

/// `GET /api/devices/{device_id}/overlay`：返回该设备当前 overlay（白名单字段子集）。
pub(in crate::app::ws) async fn device_overlay_get(
    State(engines): State<Arc<Engines>>,
    Path(device_id): Path<String>,
) -> Response {
    let Some(config_path) = &engines.config_path else {
        return (StatusCode::BAD_REQUEST, json_msg(false, "server 以内置默认配置启动，无法读写设备 overlay（请用 --config 指定配置文件后重启）。", None)).into_response();
    };
    let Some(path) = device_overlay_path(config_path, &device_id) else {
        return (StatusCode::BAD_REQUEST, json_msg(false, "设备 ID 非法（仅允许字母数字与 - _ : 字符）。", None)).into_response();
    };
    match read_device_overlay_value(&path) {
        Some(overlay) => json_ok(serde_json::json!({
            "device_id": device_id,
            "overlay": overlay,
            "has_overlay": true,
        })),
        None => json_ok(serde_json::json!({
            "device_id": device_id,
            "overlay": serde_json::json!({}),
            "has_overlay": false,
        })),
    }
}

/// `PUT /api/devices/{device_id}/overlay`：写入/替换该设备 overlay（白名单校验 + 原子写）。
pub(in crate::app::ws) async fn device_overlay_put(
    State(engines): State<Arc<Engines>>,
    Path(device_id): Path<String>,
    Json(raw): Json<serde_json::Value>,
) -> Response {
    if !raw.is_object() {
        return (StatusCode::BAD_REQUEST, "overlay JSON 必须是对象（如 {\"tts\":{\"speaker\":2}}）".to_string()).into_response();
    }
    if let Err(msg) = validate_overlay(&raw) {
        return (StatusCode::BAD_REQUEST, json_msg(false, &msg, None)).into_response();
    }
    let Some(config_path) = &engines.config_path else {
        return (StatusCode::BAD_REQUEST, json_msg(false, "server 以内置默认配置启动，无法读写设备 overlay（请用 --config 指定配置文件后重启）。", None)).into_response();
    };
    let Some(path) = device_overlay_path(config_path, &device_id) else {
        return (StatusCode::BAD_REQUEST, json_msg(false, "设备 ID 非法（仅允许字母数字与 - _ : 字符）。", None)).into_response();
    };
    let toml_str = match json_to_toml(&raw) {
        Ok(s) => s,
        Err(e) => return (StatusCode::BAD_REQUEST, json_msg(false, &format!("overlay 序列化失败: {e}"), None)).into_response(),
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&path, &toml_str) {
        Ok(()) => (
            StatusCode::OK,
            json_msg(true, &format!("设备 {device_id} 的 overlay 已保存（新会话生效）。"), Some(path.to_str().unwrap_or(""))),
        ).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            json_msg(false, &format!("写入设备 overlay 失败: {e}"), Some(path.to_str().unwrap_or(""))),
        ).into_response(),
    }
}

/// `DELETE /api/devices/{device_id}/overlay`：删除该设备 overlay（恢复用全局配置）。
pub(in crate::app::ws) async fn device_overlay_delete(
    State(engines): State<Arc<Engines>>,
    Path(device_id): Path<String>,
) -> Response {
    let Some(config_path) = &engines.config_path else {
        return (StatusCode::BAD_REQUEST, json_msg(false, "server 以内置默认配置启动，无法读写设备 overlay（请用 --config 指定配置文件后重启）。", None)).into_response();
    };
    let Some(path) = device_overlay_path(config_path, &device_id) else {
        return (StatusCode::BAD_REQUEST, json_msg(false, "设备 ID 非法（仅允许字母数字与 - _ : 字符）。", None)).into_response();
    };
    match std::fs::remove_file(&path) {
        Ok(()) => (StatusCode::OK, json_msg(true, &format!("已删除设备 {device_id} 的 overlay（恢复使用全局配置）。"), None)).into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (StatusCode::OK, json_msg(true, "设备 overlay 不存在，无需删除。", None)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, json_msg(false, &format!("删除设备 overlay 失败: {e}"), None)).into_response(),
    }
}
