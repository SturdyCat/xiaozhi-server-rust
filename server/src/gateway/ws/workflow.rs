//! Workflow designer read/validate endpoints. Persistence uses the existing
//! `/api/config` merge pipeline; this endpoint never bypasses ConfigStore.
//!
//! 观察者目录（已装载 wasm 插件）注入编译/投影：编辑器可绑定的节点目录与
//! 编译器同一来源（保存门 = 同一 `compile_with_observers`）。
use super::{
    config::{config_revision, current_config},
    json_ok, json_response,
};
use crate::{runtime::Engines, workflow::designer};
use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::Response,
};
use serde_json::Value;
use std::sync::Arc;

pub(super) async fn get_workflow(State(engines): State<Arc<Engines>>) -> Response {
    let cfg = current_config(&engines);
    let revision = config_revision(&cfg);
    let observers = engines.wasm_observers();
    if let Err(error) = designer::compile_with_observers(&cfg.workflow, &observers) {
        return json_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            serde_json::json!({"valid":false,"error":error,"revision":revision}),
        );
    }
    let mode = if cfg.aiui.enabled { "aiui" } else { "local" };
    json_ok(designer::projection_with_observers(
        &cfg.workflow,
        revision,
        mode,
        &observers,
    ))
}

/// Dry-run compiler shared by the editor and test clients. It does not save or
/// activate a plan; callers persist through PUT /api/config with revision.
pub(super) async fn validate_workflow(
    State(engines): State<Arc<Engines>>,
    Json(value): Json<Value>,
) -> Response {
    match designer::compile_with_observers(&value, &engines.wasm_observers()) {
        Ok(compiled) => json_ok(
            serde_json::json!({"valid":true,"version":compiled.config.version,"plan":compiled.plan.iter().map(|b| serde_json::json!({"plugin_id":b.plugin_id,"position_id":b.position_id,"order":b.order,"on_error":b.on_error})).collect::<Vec<_>>() }),
        ),
        Err(error) => json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"valid":false,"error":error}),
        ),
    }
}
