//! `/api/devices/{id}/overlay` 端点组（P1-5）：每设备「会话内参数」增量，落盘于 config 同目录
//! `overlays/devices/<device_id>.toml`（文件后端）或 `cfg_device_overlays` 表（SQLite 后端）；
//! 仅白名单字段可覆盖（见 `crate::config::overlay`）。不改变 `PUT /api/config` 的部分更新语义。
//!
//! 从父模块 `config.rs` 拆出（AGENTS.md §5.10 文件规模约定：行为不变的搬移），本次改为经
//! [`crate::config::store::ConfigStore`] 读写（不再直接 `std::fs` 操作）。

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::config::overlay::validate_overlay;
use crate::engine::Engines;

use super::{json_msg, json_ok};

// ============================================================
// 设备 overlay（P1-5）：每设备「会话内参数」增量；仅白名单字段可覆盖。
// 不再改变 `PUT /api/config` 的部分更新语义。
// ============================================================

/// `GET /api/devices/{device_id}/overlay`：返回该设备当前 overlay（白名单字段子集）。
pub(in crate::app::ws) async fn device_overlay_get(
    State(engines): State<Arc<Engines>>,
    Path(device_id): Path<String>,
) -> Response {
    match engines.config_store.load_overlay(&device_id) {
        Ok(Some(overlay)) => json_ok(serde_json::json!({
            "device_id": device_id,
            "overlay": overlay,
            "has_overlay": true,
        })),
        Ok(None) => json_ok(serde_json::json!({
            "device_id": device_id,
            "overlay": serde_json::json!({}),
            "has_overlay": false,
        })),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            json_msg(false, &format!("{e:#}"), None),
        )
            .into_response(),
    }
}

/// `PUT /api/devices/{device_id}/overlay`：写入/替换该设备 overlay（白名单校验 + 落库）。
pub(in crate::app::ws) async fn device_overlay_put(
    State(engines): State<Arc<Engines>>,
    Path(device_id): Path<String>,
    Json(raw): Json<serde_json::Value>,
) -> Response {
    if !raw.is_object() {
        return (
            StatusCode::BAD_REQUEST,
            "overlay JSON 必须是对象（如 {\"tts\":{\"speaker\":2}}）".to_string(),
        )
            .into_response();
    }
    if let Err(msg) = validate_overlay(&raw) {
        return (StatusCode::BAD_REQUEST, json_msg(false, &msg, None)).into_response();
    }
    match engines.config_store.save_overlay(&device_id, &raw) {
        Ok(()) => (
            StatusCode::OK,
            json_msg(
                true,
                &format!("设备 {device_id} 的 overlay 已保存（新会话生效）。"),
                engines.config_store.location(),
            ),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            json_msg(false, &format!("写入设备 overlay 失败: {e:#}"), None),
        )
            .into_response(),
    }
}

/// `DELETE /api/devices/{device_id}/overlay`：删除该设备 overlay（恢复用全局配置）。
pub(in crate::app::ws) async fn device_overlay_delete(
    State(engines): State<Arc<Engines>>,
    Path(device_id): Path<String>,
) -> Response {
    match engines.config_store.delete_overlay(&device_id) {
        Ok(()) => (
            StatusCode::OK,
            json_msg(
                true,
                &format!("已删除设备 {device_id} 的 overlay（恢复使用全局配置）。"),
                None,
            ),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            json_msg(false, &format!("删除设备 overlay 失败: {e:#}"), None),
        )
            .into_response(),
    }
}
