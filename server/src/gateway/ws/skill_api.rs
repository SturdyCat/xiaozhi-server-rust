//! Authenticated saved skill catalog and persistent task management; never dispatches device audio.
use super::nlu_build::{access, error};
use crate::{config::Config, runtime::Engines, skills::SkillExecutionContext};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc, OnceLock},
};

type ApiError = (StatusCode, &'static str, String);

pub(super) fn routes() -> Router<Arc<Engines>> {
    Router::new()
        .route("/api/skills/catalog", get(catalog))
        .route("/api/skills/capabilities", get(capabilities))
        .route("/api/skill-tasks", get(list_tasks).post(create_task))
        .route("/api/skill-tasks/{id}", get(get_task))
        .route("/api/skill-tasks/{id}/cancel", post(cancel_task))
}

fn invalid(message: impl Into<String>) -> ApiError {
    (StatusCode::BAD_REQUEST, "invalid_input", message.into())
}
fn internal(operation: &str, err: anyhow::Error) -> ApiError {
    tracing::warn!(operation, error = %err, "Skill management operation failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "skill_store_failed",
        "Skill storage operation failed; inspect server logs".into(),
    )
}
fn missing() -> ApiError {
    (
        StatusCode::NOT_FOUND,
        "task_not_found",
        "Task not found for the requested device".into(),
    )
}
fn valid_string(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
fn valid_device(value: &str) -> Result<(), ApiError> {
    if valid_string(value, 256) {
        Ok(())
    } else {
        Err(invalid("device_id must contain 1..256 non-control bytes"))
    }
}
fn valid_task(value: &str) -> Result<(), ApiError> {
    if super::nlu_build::valid_id(value) {
        Ok(())
    } else {
        Err(invalid("task id must be a canonical UUID"))
    }
}
fn parse_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    if body.len() > 16384 {
        return Err(invalid("Request exceeds 16384 bytes"));
    }
    serde_json::from_slice(body).map_err(|_| invalid("Invalid JSON request or unknown fields"))
}

// A timed-out request keeps a permit until its SQLite worker actually exits.
async fn blocking<F>(f: F) -> Response
where
    F: FnOnce() -> Result<Value, ApiError> + Send + 'static,
{
    static LIMIT: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let permit = match LIMIT
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(4)))
        .clone()
        .try_acquire_owned()
    {
        Ok(p) => p,
        Err(_) => {
            return error(
                StatusCode::TOO_MANY_REQUESTS,
                "skill_api_busy",
                "Four skill management operations are already running",
            )
        }
    };
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        f()
    });
    match tokio::time::timeout(std::time::Duration::from_secs(10), task).await {
        Ok(Ok(Ok(value))) => Json(value).into_response(),
        Ok(Ok(Err((status, "clock_clarification", speech)))) => (status, Json(json!({
            "state":"error", "error_code":"clock_clarification", "error":speech, "speech":speech
        }))).into_response(),
        Ok(Ok(Err((status, code, message)))) => error(status, code, message),
        Ok(Err(_)) => error(StatusCode::INTERNAL_SERVER_ERROR, "skill_worker_failed", "Skill management worker failed"),
        Err(_) => error(StatusCode::GATEWAY_TIMEOUT, "skill_api_timeout", "Operation timed out and may still complete; retry task creation only with the same request_id"),
    }
}

fn catalog_json(cfg: &Config) -> anyhow::Result<Value> {
    let effective = crate::plugins::skill::effective_catalog(&cfg.command, &cfg.nlu)?;
    let defaults = crate::plugins::nlu::NluConfig::default();
    let intents: Vec<Value> = effective
        .intents
        .iter()
        .map(|intent| {
            let source = if intent.name == "end_session" {
                "command"
            } else if defaults
                .intents
                .iter()
                .any(|i| serde_json::to_value(i).ok() == serde_json::to_value(intent).ok())
            {
                "default"
            } else {
                "custom"
            };
            let mut value = serde_json::to_value(intent).expect("serializable intent");
            value["source"] = json!(source);
            value
        })
        .collect();
    // Only public declarative skill data is serialized: never model paths or the whole config.
    Ok(
        json!({"state":"ok","source":"saved","revision":super::config_revision(cfg),
        "recognition":"model_only","source_semantics":"default means definition equals shipped seed; custom means saved override/addition; command means command vocabulary",
        "intents":intents,
        "effective":{"enabled":effective.enabled,"intents":intents,"slots":effective.slots},
        "capabilities":crate::skills::capabilities(),"executed":false}),
    )
}
async fn catalog(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if let Some(r) = access(&engines, &headers, &query, false) {
        return r;
    }
    let store = engines.config_store.clone();
    blocking(move || {
        let cfg = store.load().map_err(|e| internal("catalog_snapshot", e))?;
        catalog_json(&cfg).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "invalid_catalog",
                "Saved skill catalog is invalid; inspect configuration".into(),
            )
        })
    })
    .await
}
async fn capabilities(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if let Some(r) = access(&engines, &headers, &query, false) {
        return r;
    }
    Json(json!({"state":"ok","capabilities":crate::skills::capabilities(),"executed":false}))
        .into_response()
}

fn page(query: &HashMap<String, String>) -> Result<(Option<String>, usize, usize), ApiError> {
    let device = query.get("device_id").cloned();
    if let Some(value) = &device {
        valid_device(value)?;
    }
    let number = |key: &str, default: usize, max: usize| -> Result<usize, ApiError> {
        let value = match query.get(key) {
            Some(value) => value
                .parse::<usize>()
                .map_err(|_| invalid(format!("Invalid {key}")))?,
            None => default,
        };
        if value > max || (key == "limit" && value == 0) {
            return Err(invalid(format!("{key} exceeds allowed range")));
        }
        Ok(value)
    };
    Ok((
        device,
        number("limit", 20, 100)?,
        number("offset", 0, 10000)?,
    ))
}
async fn list_tasks(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if let Some(r) = access(&engines, &headers, &query, false) {
        return r;
    }
    let (device, limit, offset) = match page(&query) {
        Ok(p) => p,
        Err((s, c, e)) => return error(s, c, e),
    };
    blocking(move || {
        let mut result = engines
            .skill_system
            .list_tasks(device.as_deref(), limit, offset)
            .map_err(|e| internal("tasks.list", e))?;
        result["state"] = json!("ok");
        result["limit"] = json!(limit);
        result["offset"] = json!(offset);
        result["delivery_note"] =
            json!("Stored task state is not proof of device delivery or playback");
        Ok(result)
    })
    .await
}
async fn get_task(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(r) = access(&engines, &headers, &query, false) {
        return r;
    }
    if let Err((s, c, e)) = valid_task(&id) {
        return error(s, c, e);
    }
    let device = query.get("device_id").cloned();
    if let Some(value) = &device {
        if let Err((s, c, e)) = valid_device(value) {
            return error(s, c, e);
        }
    }
    blocking(move || {
        let task = engines
            .skill_system
            .get_task(&id, device.as_deref())
            .map_err(|e| internal("tasks.get", e))?
            .ok_or_else(missing)?;
        Ok(json!({"state":"ok","task":task}))
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateTask {
    device_id: String,
    request_id: Option<String>,
    args: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CancelTask {
    device_id: String,
}
fn context(device_id: String, request_id: String) -> SkillExecutionContext {
    SkillExecutionContext {
        device_id,
        session_id: format!("admin:{request_id}"),
        request_id,
        device_kind: Some("admin".into()),
        now_ms: chrono::Utc::now().timestamp_millis(),
        cancelled: Arc::new(AtomicBool::new(false)),
    }
}
fn execution_error(err: anyhow::Error) -> ApiError {
    if let Some(action_error) = err.downcast_ref::<crate::skills::SkillActionError>() {
        return (
            StatusCode::BAD_REQUEST,
            "clock_clarification",
            action_error.speech.clone(),
        );
    }
    if err
        .chain()
        .any(|cause| cause.is::<rusqlite::Error>() || cause.is::<std::io::Error>())
    {
        return internal("task_execution", err);
    }
    if err.to_string().contains("idempotency conflict") {
        return (
            StatusCode::CONFLICT,
            "request_id_conflict",
            "request_id was already used with different arguments".into(),
        );
    }
    if err.to_string().contains("capacity") {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "task_capacity",
            "Task capacity exceeded".into(),
        );
    }
    tracing::warn!(error = %err, "Skill task request rejected");
    invalid("Task request rejected; check capability argument schema, schedule, device binding and request_id")
}
async fn create_task(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    if let Some(r) = access(&engines, &headers, &query, false) {
        return r;
    }
    let req: CreateTask = match parse_body(&body) {
        Ok(r) => r,
        Err((s, c, e)) => return error(s, c, e),
    };
    if let Err((s, c, e)) = valid_device(&req.device_id) {
        return error(s, c, e);
    }
    if !req.args.is_object() || req.args.get("device_id").is_some() {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "args must be an object and cannot override device_id",
        );
    }
    let request_id = req
        .request_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if !valid_string(&request_id, 128) {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "request_id must contain 1..128 non-control bytes",
        );
    }
    blocking(move || {
        let ctx = context(req.device_id, request_id.clone());
        let mut result = engines
            .skill_system
            .execute_api(&ctx, "tasks.create", &req.args)
            .map_err(execution_error)?;
        result["state"] = json!("ok");
        result["request_id"] = json!(request_id);
        result["played"] = json!(false);
        result["socket_written"] = json!(false);
        Ok(result)
    })
    .await
}
async fn cancel_task(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(r) = access(&engines, &headers, &query, false) {
        return r;
    }
    if let Err((s, c, e)) = valid_task(&id) {
        return error(s, c, e);
    }
    let req: CancelTask = match parse_body(&body) {
        Ok(r) => r,
        Err((s, c, e)) => return error(s, c, e),
    };
    if let Err((s, c, e)) = valid_device(&req.device_id) {
        return error(s, c, e);
    }
    blocking(move || {
        engines
            .skill_system
            .get_task(&id, Some(&req.device_id))
            .map_err(|e| internal("tasks.get", e))?
            .ok_or_else(missing)?;
        let ctx = context(req.device_id, uuid::Uuid::new_v4().to_string());
        let mut result = engines
            .skill_system
            .execute_api(&ctx, "tasks.cancel", &json!({"task_id":id}))
            .map_err(execution_error)?;
        result["task_state"] = result["state"].clone();
        result["state"] = json!("ok");
        Ok(result)
    })
    .await
}

#[cfg(test)]
mod tests;
