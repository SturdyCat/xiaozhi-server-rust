//! Unified diagnostics HTTP boundary. Legacy paths retained, all delegate to one test service.
use crate::{
    app::bench_api::{BenchResult, StageResult, TestRequest, TestService, MAX_BODY_BYTES},
    config::ServerConfig,
    engine::Engines,
};
use axum::{
    extract::{rejection::JsonRejection, DefaultBodyLimit, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};

struct HttpState {
    service: Arc<TestService>,
    server: ServerConfig,
}
pub(super) fn routes(engines: &Arc<Engines>) -> Router<Arc<Engines>> {
    service_router(
        TestService::from_engines(engines),
        engines.config.server.clone(),
    )
    .with_state(())
}
/// Used by HTTP contract tests with a real SQLite store and injected provider factories.
pub(crate) fn service_router(service: Arc<TestService>, server: ServerConfig) -> Router {
    let state = Arc::new(HttpState { service, server });
    Router::new()
        .route("/api/test/capabilities", get(capabilities))
        .route("/api/test/run", post(run))
        .route("/api/test/llm", post(llm))
        .route("/api/test/llm/models", post(models))
        .route("/api/test/tts", post(tts))
        .route("/api/test/vad", post(vad))
        .route("/api/test/asr", post(asr))
        .route("/api/test/nlu", post(nlu))
        .route("/api/test/pipeline", post(pipeline))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth))
        .with_state(state)
}
async fn auth(State(state): State<Arc<HttpState>>, req: Request, next: Next) -> Response {
    let query = Query::<HashMap<String, String>>::try_from_uri(req.uri())
        .map(|q| q.0)
        .unwrap_or_default();
    if !super::auth_ok(req.headers(), &query, &state.server) {
        return error(
            StatusCode::UNAUTHORIZED,
            "",
            "unauthorized",
            "Admin authentication required",
        );
    }
    next.run(req).await
}
async fn capabilities(State(state): State<Arc<HttpState>>) -> Response {
    Json(state.service.capabilities().await).into_response()
}
async fn handle(
    state: Arc<HttpState>,
    input: Result<Json<Value>, JsonRejection>,
    alias: Option<&str>,
) -> Response {
    let raw = match input {
        Ok(Json(v)) => v,
        Err(_) => {
            return error(
                StatusCode::BAD_REQUEST,
                "",
                "invalid_input",
                "Invalid JSON or request body exceeds diagnostic limit",
            )
        }
    };
    let req = match request(raw, alias) {
        Ok(v) => v,
        Err(e) => return error(StatusCode::BAD_REQUEST, "", "invalid_input", &e),
    };
    response(state.service.run(req).await)
}
fn request(mut raw: Value, alias: Option<&str>) -> Result<TestRequest, String> {
    if let Some(alias) = alias {
        let asked = raw.get("action_id").and_then(|v| v.as_str());
        let allowed = match alias {
            "asr.sample" => matches!(asked, None | Some("asr.sample" | "asr.recording")),
            "pipeline.text" => matches!(asked, None | Some("pipeline.text" | "pipeline.audio")),
            _ => asked.is_none() || asked == Some(alias),
        };
        if !allowed {
            return Err("action_id does not match endpoint".into());
        }
        if asked.is_none() {
            let action = if alias == "pipeline.text"
                && (raw.get("audio").is_some() || raw.get("sample_id").is_some())
            {
                "pipeline.audio"
            } else {
                alias
            };
            let Some(map) = raw.as_object_mut() else {
                return Err("JSON object required".into());
            };
            map.insert("action_id".into(), json!(action));
        }
    }
    serde_json::from_value(raw).map_err(|_|"Invalid test request; supported fields are action_id/text/sample_id/audio/expected_revision and legacy provider overrides. Draft patches, device overlays, paths and URLs are unsupported.".into())
}
pub(crate) fn response(out: BenchResult) -> Response {
    let code = match out.result.error_code {
        Some("invalid_input") => StatusCode::BAD_REQUEST,
        Some("revision_conflict") => StatusCode::CONFLICT,
        Some("busy") => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::OK,
    };
    (code, Json(out)).into_response()
}
fn error(code: StatusCode, action: &str, kind: &'static str, message: &str) -> Response {
    (
        code,
        Json(BenchResult::new(
            action,
            None,
            StageResult::error(action, kind, message),
        )),
    )
        .into_response()
}
async fn run(
    State(state): State<Arc<HttpState>>,
    v: Result<Json<Value>, JsonRejection>,
) -> Response {
    handle(state, v, None).await
}
async fn llm(
    State(state): State<Arc<HttpState>>,
    v: Result<Json<Value>, JsonRejection>,
) -> Response {
    handle(state, v, Some("llm.chat")).await
}
async fn tts(
    State(state): State<Arc<HttpState>>,
    v: Result<Json<Value>, JsonRejection>,
) -> Response {
    handle(state, v, Some("tts.synthesis")).await
}
async fn vad(
    State(state): State<Arc<HttpState>>,
    v: Result<Json<Value>, JsonRejection>,
) -> Response {
    handle(state, v, Some("vad.sample")).await
}
async fn asr(
    State(state): State<Arc<HttpState>>,
    v: Result<Json<Value>, JsonRejection>,
) -> Response {
    handle(state, v, Some("asr.sample")).await
}
async fn nlu(
    State(state): State<Arc<HttpState>>,
    v: Result<Json<Value>, JsonRejection>,
) -> Response {
    handle(state, v, Some("nlu.classify")).await
}
async fn pipeline(
    State(state): State<Arc<HttpState>>,
    v: Result<Json<Value>, JsonRejection>,
) -> Response {
    handle(state, v, Some("pipeline.text")).await
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelsRequest {
    api_base: Option<String>,
    api_key: Option<String>,
}
async fn models(
    State(state): State<Arc<HttpState>>,
    v: Result<Json<Value>, JsonRejection>,
) -> Response {
    let req = match v
        .ok()
        .and_then(|Json(raw)| serde_json::from_value::<ModelsRequest>(raw).ok())
    {
        Some(v) => v,
        None => {
            return error(
                StatusCode::BAD_REQUEST,
                "llm.models",
                "invalid_input",
                "Expected api_base/api_key only",
            )
        }
    };
    Json(
        state
            .service
            .models(req.api_base.as_deref(), req.api_key.as_deref())
            .await,
    )
    .into_response()
}
