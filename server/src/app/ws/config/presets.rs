//! `/api/config/presets` 端点组（P1-5）：把当前配置快照成预设（文件后端 `presets/<name>.toml`
//! / SQLite 后端 `cfg_presets` 表），可应用/删除；「应用」复用 `PUT /api/config` 的同一管线
//! （部分更新 + 密钥护栏 + 生效档位）。
//!
//! 从父模块 `config.rs` 拆出（AGENTS.md §5.10 文件规模约定：行为不变的搬移），本次改为经
//! [`crate::config::store::ConfigStore`] 读写。

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::engine::Engines;

use super::{config_revision, current_config, json_msg, json_msg_full, merge_client_patch};
use super::super::json_ok;
use super::impact::save_impact;

// ============================================================
// 配置预设（P1-5）：把当前配置快照成预设，可应用/删除；
// 「应用」复用 `PUT /api/config` 的同一管线（部分更新 + 密钥护栏 + 生效档位）。
// ============================================================

/// `GET /api/config/presets`：列出全部预设名。
pub(in crate::app::ws) async fn presets_list(State(engines): State<Arc<Engines>>) -> Response {
    match engines.config_store.list_presets() {
        Ok(names) => json_ok(serde_json::json!({ "presets": names })),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            json_msg(false, &format!("{e:#}"), None),
        )
            .into_response(),
    }
}

/// `POST /api/config/presets`：把当前生效配置（实时读盘）全量保存为预设。
pub(in crate::app::ws) async fn preset_save(
    State(engines): State<Arc<Engines>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, json_msg(false, "预设名不能为空。", None)).into_response();
    }
    let current = current_config(&engines);
    match engines.config_store.save_preset(&name, &current) {
        Ok(()) => (
            StatusCode::OK,
            json_msg(
                true,
                &format!("已把当前配置保存为预设「{name}」。"),
                engines.config_store.location(),
            ),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            json_msg(false, &format!("保存预设失败: {e:#}"), None),
        )
            .into_response(),
    }
}

/// `POST /api/config/presets/{name}/apply`：把预设内容写回全局配置（复用部分更新管线）。
pub(in crate::app::ws) async fn preset_apply(
    State(engines): State<Arc<Engines>>,
    Path(name): Path<String>,
) -> Response {
    let preset_cfg = match engines.config_store.load_preset(&name) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                json_msg(false, &format!("预设「{name}」解析失败: {e:#}"), None),
            )
                .into_response()
        }
    };
    let Some(preset_cfg) = preset_cfg else {
        return (
            StatusCode::NOT_FOUND,
            json_msg(false, &format!("预设「{name}」不存在。"), None),
        )
            .into_response();
    };
    let current = current_config(&engines);
    let preset_json = match serde_json::to_value(&preset_cfg) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                json_msg(false, &format!("预设序列化失败: {e}"), None),
            )
                .into_response()
        }
    };
    let body = match merge_client_patch(&current, &preset_json) {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("应用预设失败: {e}")).into_response(),
    };
    match engines.config_store.persist(&body) {
        Ok(()) => {
            let impact = save_impact(&current, &body);
            let new_rev = config_revision(&body);
            (
                StatusCode::OK,
                json_msg_full(
                    true,
                    &format!("已应用预设「{name}」到全局配置（新会话生效）。"),
                    engines.config_store.location(),
                    Some(&new_rev),
                    Some(&impact),
                ),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            json_msg(false, &format!("{e:#}"), engines.config_store.location()),
        )
            .into_response(),
    }
}

/// `DELETE /api/config/presets/{name}`：删除预设。
pub(in crate::app::ws) async fn preset_delete(
    State(engines): State<Arc<Engines>>,
    Path(name): Path<String>,
) -> Response {
    match engines.config_store.delete_preset(&name) {
        Ok(()) => (
            StatusCode::OK,
            json_msg(true, &format!("已删除预设「{name}」。"), None),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            json_msg(false, &format!("删除预设失败: {e:#}"), None),
        )
            .into_response(),
    }
}
