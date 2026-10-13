//! `/api/config/presets` 端点组（P1-5）：把当前配置快照成 `presets/<name>.toml`，
//! 可应用/删除；「应用」复用 `PUT /api/config` 的同一管线（部分更新 + 原子写 +
//! 密钥护栏 + 生效档位）。
//!
//! 从父模块 `config.rs` 拆出（AGENTS.md §5.10 文件规模约定：行为不变的搬移）。

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::config::Config;
use crate::config::overlay::{preset_path, presets_dir};
use crate::engine::Engines;

use super::{config_revision, current_config, json_msg, json_msg_full, merge_client_patch};
use super::super::json_ok;
use super::impact::save_impact;

// ============================================================
// 配置预设（P1-5）：把当前配置快照成 `presets/<name>.toml`，可应用/删除；
// 「应用」复用 `PUT /api/config` 的同一管线（部分更新 + 原子写 + 密钥护栏 + 生效档位）。
// ============================================================

/// `GET /api/config/presets`：列出全部预设名。
pub(in crate::app::ws) async fn presets_list(State(engines): State<Arc<Engines>>) -> Response {
    let Some(config_path) = engines.config_path() else {
        return (StatusCode::BAD_REQUEST, json_msg(false, "server 以内置默认配置启动，无法使用预设（请用 --config 指定配置文件后重启）。", None)).into_response();
    };
    let Some(dir) = presets_dir(config_path) else {
        return (StatusCode::BAD_REQUEST, json_msg(false, "无法确定预设目录。", None)).into_response();
    };
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("toml") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
    }
    names.sort();
    json_ok(serde_json::json!({ "presets": names }))
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
    let Some(config_path) = engines.config_path() else {
        return (StatusCode::BAD_REQUEST, json_msg(false, "server 以内置默认配置启动，无法使用预设（请用 --config 指定配置文件后重启）。", None)).into_response();
    };
    let Some(preset_file) = preset_path(config_path, &name) else {
        return (StatusCode::BAD_REQUEST, json_msg(false, "预设名非法（仅允许字母数字与 - _ . 字符）。", None)).into_response();
    };
    let current = current_config(&engines);
    let toml_str = match toml::to_string_pretty(&current) {
        Ok(s) => s,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, json_msg(false, &format!("序列化预设失败: {e}"), None)).into_response(),
    };
    if let Some(parent) = preset_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&preset_file, &toml_str) {
        Ok(()) => (StatusCode::OK, json_msg(true, &format!("已把当前配置保存为预设「{name}」。"), Some(preset_file.to_str().unwrap_or("")))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, json_msg(false, &format!("保存预设失败: {e}"), Some(&preset_file.to_string_lossy()))).into_response(),
    }
}

/// `POST /api/config/presets/{name}/apply`：把预设内容写回全局配置（复用部分更新管线）。
pub(in crate::app::ws) async fn preset_apply(
    State(engines): State<Arc<Engines>>,
    Path(name): Path<String>,
) -> Response {
    let Some(config_path) = engines.config_path() else {
        return (StatusCode::BAD_REQUEST, json_msg(false, "server 以内置默认配置启动，无法应用预设（请用 --config 指定配置文件后重启）。", None)).into_response();
    };
    let Some(preset_file) = preset_path(config_path, &name) else {
        return (StatusCode::BAD_REQUEST, json_msg(false, "预设名非法（仅允许字母数字与 - _ . 字符）。", None)).into_response();
    };
    if !preset_file.exists() {
        return (StatusCode::NOT_FOUND, json_msg(false, &format!("预设「{name}」不存在。"), None)).into_response();
    }
    let preset_cfg = match Config::load(preset_file.to_str().unwrap_or("")) {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_REQUEST, json_msg(false, &format!("预设「{name}」解析失败: {e}"), None)).into_response(),
    };
    let current = current_config(&engines);
    let preset_json = match serde_json::to_value(&preset_cfg) {
        Ok(v) => v,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, json_msg(false, &format!("预设序列化失败: {e}"), None)).into_response(),
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
                    Some(config_path),
                    Some(&new_rev),
                    Some(&impact),
                ),
            ).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, json_msg(false, &format!("{e:#}"), Some(config_path))).into_response(),
    }
}

/// `DELETE /api/config/presets/{name}`：删除预设。
pub(in crate::app::ws) async fn preset_delete(
    State(engines): State<Arc<Engines>>,
    Path(name): Path<String>,
) -> Response {
    let Some(config_path) = engines.config_path() else {
        return (StatusCode::BAD_REQUEST, json_msg(false, "server 以内置默认配置启动，无法管理预设（请用 --config 指定配置文件后重启）。", None)).into_response();
    };
    let Some(preset_file) = preset_path(config_path, &name) else {
        return (StatusCode::BAD_REQUEST, json_msg(false, "预设名非法（仅允许字母数字与 - _ . 字符）。", None)).into_response();
    };
    match std::fs::remove_file(&preset_file) {
        Ok(()) => (StatusCode::OK, json_msg(true, &format!("已删除预设「{name}」。"), None)).into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (StatusCode::OK, json_msg(true, "预设不存在，无需删除。", None)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, json_msg(false, &format!("删除预设失败: {e}"), None)).into_response(),
    }
}
