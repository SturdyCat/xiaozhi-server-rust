//! 插件生命周期端点：register / activate / deactivate / remove。
//!
//! 三态语义（契约锁定，2026-11-07）：
//!
//! - **register**：Staged → Registered（完整校验 + self-test；失败保留 Staged 带原因），
//!   成功写 `<id>.registered.json` 标记（重启恢复 Registered）；
//! - **activate**：Registered → Active（依赖前置：所有 requires 命中 Active 且版本
//!   ≥ min-version，否则 409 列出缺口）；成功装载 + 持久化 `enabled=true`；
//! - **deactivate**：Active → Registered（存在 Active 依赖者 → 409 列出）；卸载实例，
//!   产物/标记/kv 保留 + 持久化 `enabled=false`；
//! - **remove**：仅 Staged/Registered（Active 一律 409）；删除条目+产物+标记+kv。
//!
//! 管理鉴权与现有 wasm 端点同型（main.rs 全局 layer）；wasm 调用全部 spawn_blocking。

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::Value;

use crate::plugins::wasm::{ActivateError, DeactivateError, RemoveError, WasmHost};
use crate::runtime::Engines;

use super::super::json_ok;

fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "WASM 插件平台不可用（初始化失败已降级关闭，见服务日志）",
    )
        .into_response()
}

fn error_json(status: StatusCode, error: String, extra: Value) -> Response {
    let mut body = serde_json::json!({ "ok": false, "error": error });
    if let (Value::Object(map), Value::Object(extra)) = (&mut body, extra) {
        for (k, v) in extra {
            map.insert(k, v);
        }
    }
    (status, axum::Json(body)).into_response()
}

/// 读产物字节（register/activate 需要）。`Ok(None)` = 无产物目录或文件缺失。
async fn artifact_bytes(engines: &Engines, plugin_id: &str) -> Result<Option<Vec<u8>>, Response> {
    let Some(dir) = engines.wasm_artifact_dir() else {
        // 内置配置无数据卷：上传无法落盘，生命周期端点无字节可用
        return Err(error_json(
            StatusCode::SERVICE_UNAVAILABLE,
            "无产物目录（内置配置启动不支持插件持久生命周期）".to_string(),
            serde_json::json!({}),
        ));
    };
    let path = dir.path().join(format!("{plugin_id}.wasm"));
    match tokio::task::spawn_blocking(move || std::fs::read(path)).await {
        Ok(Ok(bytes)) => Ok(Some(bytes)),
        Ok(Err(_)) => Ok(None),
        Err(e) => Err(error_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("产物读取任务失败：{e}"),
            serde_json::json!({}),
        )),
    }
}

fn contract_version_label() -> String {
    format!("{}.x", crate::plugins::wasm::CONTRACT_MAJOR_VERSION)
}

fn lifecycle_of(host: &WasmHost, id: &str) -> String {
    host.lifecycle_of(id)
        .map(|(lc, _)| lc.as_str().to_string())
        .unwrap_or_else(|| "missing".to_string())
}

/// `POST /api/plugins/{id}/register`：Staged → Registered（完整校验 + self-test）。
pub(crate) async fn register_plugin(
    State(engines): State<Arc<Engines>>,
    Path(plugin_id): Path<String>,
) -> Response {
    let Some(host) = engines.wasm.as_ref() else {
        return unavailable();
    };
    let bytes = match artifact_bytes(&engines, &plugin_id).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            return error_json(
                StatusCode::NOT_FOUND,
                format!("插件 {plugin_id} 无产物（先上传）"),
                serde_json::json!({}),
            );
        }
        Err(resp) => return resp,
    };
    let registered = tokio::task::spawn_blocking({
        let host = host.clone();
        let id = plugin_id.clone();
        move || host.register(&id, &bytes)
    })
    .await;
    let registration = match registered {
        Ok(Ok(registration)) => registration,
        Ok(Err(e)) => {
            // 失败保留 Staged 带原因
            return error_json(
                StatusCode::BAD_REQUEST,
                format!("注册失败（保留原状态）：{e:#}"),
                serde_json::json!({ "lifecycle": lifecycle_of(host, &plugin_id) }),
            );
        }
        Err(e) => {
            return error_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("注册任务失败：{e}"),
                serde_json::json!({}),
            );
        }
    };
    // Registered 标记持久化（重启扫描据此恢复 Registered）
    if let Some(dir) = engines.wasm_artifact_dir() {
        if let Err(e) = dir.write_registered_marker(&plugin_id, &contract_version_label()) {
            tracing::warn!(plugin = %plugin_id, "注册标记写入失败（重启后将回到 Staged）：{e}");
        }
    }
    json_ok(serde_json::json!({
        "ok": true,
        "id": plugin_id,
        "lifecycle": "registered",
        "actions": registration.actions.iter().map(|a| a.id.clone()).collect::<Vec<_>>(),
        "config_fields": registration.config_fields.iter().map(|f| f.key.clone()).collect::<Vec<_>>(),
    }))
}

/// `POST /api/plugins/{id}/activate`：Registered → Active（依赖前置 + 装载 + enabled=true）。
pub(crate) async fn activate_plugin(
    State(engines): State<Arc<Engines>>,
    Path(plugin_id): Path<String>,
) -> Response {
    let Some(host) = engines.wasm.as_ref() else {
        return unavailable();
    };
    let bytes = match artifact_bytes(&engines, &plugin_id).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            return error_json(
                StatusCode::NOT_FOUND,
                format!("插件 {plugin_id} 无产物（先上传）"),
                serde_json::json!({}),
            );
        }
        Err(resp) => return resp,
    };
    let activated = match tokio::task::spawn_blocking({
        let host = host.clone();
        let id = plugin_id.clone();
        move || host.activate_checked(&id, &bytes)
    })
    .await
    {
        Ok(result) => result,
        Err(e) => {
            return error_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("激活任务失败：{e}"),
                serde_json::json!({}),
            );
        }
    };
    match activated {
        Ok(()) => {}
        Err(ActivateError::NotFound) => {
            return error_json(
                StatusCode::NOT_FOUND,
                format!("插件 {plugin_id} 未登记（先上传）"),
                serde_json::json!({}),
            );
        }
        Err(ActivateError::WrongLifecycle { current }) => {
            let hint = if current == "staged" {
                "（先注册）"
            } else {
                "（已激活）"
            };
            return error_json(
                StatusCode::CONFLICT,
                format!("插件 {plugin_id} lifecycle={current}{hint}"),
                serde_json::json!({ "lifecycle": current }),
            );
        }
        Err(ActivateError::Dependencies(gaps)) => {
            return error_json(
                StatusCode::CONFLICT,
                format!(
                    "依赖不满足：{}",
                    gaps.iter()
                        .map(|g| match g.reason {
                            "missing" => format!("{} 缺失", g.id),
                            "inactive" => format!("{} 未激活", g.id),
                            _ => format!(
                                "{} 版本 {} < {}",
                                g.active_version.as_deref().unwrap_or("?"),
                                g.id,
                                g.min_version
                            ),
                        })
                        .collect::<Vec<_>>()
                        .join("、"),
                ),
                serde_json::json!({
                    "missing": gaps.iter().map(|g| g.to_json()).collect::<Vec<_>>(),
                }),
            );
        }
        Err(ActivateError::Build(reason)) => {
            return error_json(
                StatusCode::BAD_REQUEST,
                format!("激活失败（保留 Registered）：{reason}"),
                serde_json::json!({ "lifecycle": "registered" }),
            );
        }
    }
    // 激活意图持久化（重启引导按它拓扑激活）
    if let Err(e) = engines.set_wasm_enabled(&plugin_id, true) {
        tracing::warn!(plugin = %plugin_id, "激活意图写入配置库失败（本次运行有效）：{e:#}");
    }
    json_ok(serde_json::json!({
        "ok": true,
        "id": plugin_id,
        "lifecycle": lifecycle_of(host, &plugin_id),
        "extensions": host.snapshot_json()["extensions"].clone(),
    }))
}

/// `POST /api/plugins/{id}/deactivate`：Active → Registered（依赖者检查 + 卸载实例）。
pub(crate) async fn deactivate_plugin(
    State(engines): State<Arc<Engines>>,
    Path(plugin_id): Path<String>,
) -> Response {
    let Some(host) = engines.wasm.as_ref() else {
        return unavailable();
    };
    let deactivated = match tokio::task::spawn_blocking({
        let host = host.clone();
        let id = plugin_id.clone();
        move || host.deactivate(&id)
    })
    .await
    {
        Ok(result) => result,
        Err(e) => {
            return error_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("停用任务失败：{e}"),
                serde_json::json!({}),
            );
        }
    };
    match deactivated {
        Ok(()) => {}
        Err(DeactivateError::NotFound) => {
            return error_json(
                StatusCode::NOT_FOUND,
                format!("插件 {plugin_id} 未登记"),
                serde_json::json!({}),
            );
        }
        Err(DeactivateError::WrongLifecycle { current }) => {
            return error_json(
                StatusCode::CONFLICT,
                format!("插件 {plugin_id} lifecycle={current}（仅 active 可停用）"),
                serde_json::json!({ "lifecycle": current }),
            );
        }
        Err(DeactivateError::Dependents(dependents)) => {
            return error_json(
                StatusCode::CONFLICT,
                format!("存在 Active 依赖者，先停用：{}", dependents.join("、")),
                serde_json::json!({ "dependents": dependents }),
            );
        }
    }
    // 停用意图持久化 + Registered 标记兜底（无标记的旧迁移条目也保持 Registered 语义）
    if let Err(e) = engines.set_wasm_enabled(&plugin_id, false) {
        tracing::warn!(plugin = %plugin_id, "停用意图写入配置库失败（本次运行有效）：{e:#}");
    }
    if let Some(dir) = engines.wasm_artifact_dir() {
        if dir.read_registered_marker(&plugin_id).is_none() {
            if let Err(e) = dir.write_registered_marker(&plugin_id, &contract_version_label()) {
                tracing::warn!(plugin = %plugin_id, "停用后注册标记补写失败：{e}");
            }
        }
    }
    json_ok(serde_json::json!({
        "ok": true,
        "id": plugin_id,
        "lifecycle": "registered",
    }))
}

/// `POST /api/plugins/{id}/remove`：仅 Staged/Registered 可移除（Active 一律 409）。
/// 删除注册表条目 + kv 命名空间 + 产物 + 注册标记 + 摘要钉版。
pub(crate) async fn remove_plugin(
    State(engines): State<Arc<Engines>>,
    Path(plugin_id): Path<String>,
) -> Response {
    let Some(host) = engines.wasm.as_ref() else {
        return unavailable();
    };
    let entry_removed = match tokio::task::spawn_blocking({
        let host = host.clone();
        let id = plugin_id.clone();
        move || host.remove(&id)
    })
    .await
    {
        Ok(Ok(())) => true,
        Ok(Err(RemoveError::Active)) => {
            return error_json(
                StatusCode::CONFLICT,
                "插件仍在运行（先停用再移除）".to_string(),
                serde_json::json!({}),
            );
        }
        Ok(Err(RemoveError::NotFound)) => false,
        Err(e) => {
            return error_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("移除任务失败：{e}"),
                serde_json::json!({}),
            );
        }
    };
    let artifact_removed = engines
        .wasm_artifact_dir()
        .map(|d| d.remove(&plugin_id))
        .unwrap_or(false);
    let marker_removed = engines
        .wasm_artifact_dir()
        .map(|d| d.remove_registered_marker(&plugin_id))
        .unwrap_or(false);
    if let Err(e) = engines.clear_wasm_artifact_pin(&plugin_id) {
        tracing::warn!(plugin = %plugin_id, "插件摘要钉版清理失败：{e:#}");
    }
    if !entry_removed && !artifact_removed {
        return error_json(
            StatusCode::NOT_FOUND,
            format!("插件 {plugin_id} 未登记且无产物文件"),
            serde_json::json!({}),
        );
    }
    json_ok(serde_json::json!({
        "ok": true,
        "id": plugin_id,
        "entry_removed": entry_removed,
        "artifact_removed": artifact_removed,
        "marker_removed": marker_removed,
    }))
}
