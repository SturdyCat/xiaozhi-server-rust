//! Saved-config NLU training. Each build owns immutable versioned output; success never activates it.
//! Build state is process-local; successful manifests persist and remain independently testable by id.
use crate::{
    config::{store::ConfigStore, Config},
    runtime::Engines,
};
use axum::{
    body::Bytes,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{broadcast, Mutex};

mod artifact;
mod mounts;
mod training;
pub(super) use artifact::{load_candidate, valid_id, Artifact};
#[cfg(test)]
use mounts::{map_via_mounts, mount_host_path};

const TRAINER_IMAGE: &str = "registry.cn-hangzhou.aliyuncs.com/notip-com-cn/nlu-trainer:latest";
// Trusted source is compiled into this server; never execute scripts from writable data directories.
const TRAINER_SOURCE: &str = include_str!("../../../../tools/nlu-trainer/train.py");
#[derive(Default)]
struct BuildJob {
    running: bool,
    state: &'static str,
    log: String,
    error: String,
    build_id: Option<String>,
    source_revision: Option<String>,
    output_dir: Option<String>,
    artifact: Option<Artifact>,
}
impl BuildJob {
    fn new() -> Self {
        Self {
            state: "idle",
            ..Self::default()
        }
    }
}
static JOB: std::sync::OnceLock<Mutex<BuildJob>> = std::sync::OnceLock::new();
fn job() -> &'static Mutex<BuildJob> {
    JOB.get_or_init(|| Mutex::new(BuildJob::new()))
}
#[derive(Clone)]
enum BuildEvent {
    Log(String),
    Done,
}
static LOG_TX: std::sync::OnceLock<broadcast::Sender<BuildEvent>> = std::sync::OnceLock::new();
fn log_tx() -> &'static broadcast::Sender<BuildEvent> {
    LOG_TX.get_or_init(|| broadcast::channel(256).0)
}

pub(super) fn error(status: StatusCode, code: &str, message: impl std::fmt::Display) -> Response {
    (
        status,
        Json(json!({"state":"error","error_code":code,"error":message.to_string()})),
    )
        .into_response()
}

pub(super) fn access(
    engines: &Engines,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
    inference: bool,
) -> Option<Response> {
    if !super::auth_ok(headers, query, &engines.config.server) {
        return Some(error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Admin authentication required",
        ));
    }
    if inference && engines.dev_mode {
        return Some(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "dev_inference_disabled",
            crate::dev::DISABLED,
        ));
    }
    None
}

pub(super) fn data_dir(cfg: &Config) -> PathBuf {
    Path::new(&cfg.memory.db_path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("/data"))
        .to_path_buf()
}

async fn push_log(line: &str) {
    {
        let mut j = job().lock().await;
        j.log.push_str(line);
        j.log.push('\n');
        let mut cut = j.log.len().saturating_sub(8192);
        while !j.log.is_char_boundary(cut) {
            cut -= 1;
        }
        j.log.drain(..cut);
    }
    tracing::info!(target: "nlu_build", "{line}");
    let _ = log_tx().send(BuildEvent::Log(line.into()));
}

fn validate_buildable(nlu: &crate::plugins::nlu::NluConfig) -> Result<(), String> {
    if !nlu.enabled {
        return Err("[nlu].enabled=false，请先保存启用配置".into());
    }
    if nlu.intents.is_empty() {
        return Err("[nlu].intents 为空，请先保存技能".into());
    }
    let mut names = std::collections::HashSet::new();
    for intent in &nlu.intents {
        if intent.name.trim().is_empty() || intent.name == "other" || !names.insert(&intent.name) {
            return Err("技能名必须非空、唯一，且不能为保留标签 other".into());
        }
        if intent.templates.is_empty() {
            return Err(format!("技能 {} 没有训练模板", intent.name));
        }
    }
    Ok(())
}

fn training_catalog(cfg: &Config) -> Result<crate::plugins::nlu::NluConfig, String> {
    let mut nlu = crate::plugins::skill::effective_catalog(&cfg.command, &cfg.nlu)
        .map_err(|e| e.to_string())?;
    // New runs emit canonical catalog names; imported model label maps do not apply.
    // Older manifests are loaded verbatim and never passed through this function.
    nlu.bert_label_map.clear();
    validate_buildable(&nlu)?;
    Ok(nlu)
}

fn saved_snapshot(
    store: &dyn ConfigStore,
    expected: Option<&str>,
) -> anyhow::Result<(Config, String)> {
    // No live_config fallback: a failed SQLite read must never silently train stale skills.
    let cfg = store.load()?;
    let revision = super::config::config_revision(&cfg);
    if expected.is_some_and(|v| v != revision) {
        anyhow::bail!("revision_conflict");
    }
    Ok((cfg, revision))
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildRequest {
    expected_revision: Option<String>,
}

pub(super) async fn start_build(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    if let Some(r) = access(&engines, &headers, &query, true) {
        return r;
    }
    if body.len() > 4096 {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "Build request exceeds 4096 bytes",
        );
    }
    let req: BuildRequest = if body.is_empty() {
        BuildRequest::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return error(StatusCode::BAD_REQUEST, "invalid_input", e),
        }
    };
    let store = engines.config_store.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        saved_snapshot(store.as_ref(), req.expected_revision.as_deref())
    })
    .await;
    let (cfg, revision) = match loaded {
        Ok(Ok(v)) => v,
        Ok(Err(e)) if e.to_string() == "revision_conflict" => {
            return error(
                StatusCode::CONFLICT,
                "revision_conflict",
                "Saved config changed; reload before building",
            )
        }
        e => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "config_read_failed",
                format!("{e:?}"),
            )
        }
    };
    let nlu = match training_catalog(&cfg) {
        Ok(catalog) => catalog,
        Err(e) => return error(StatusCode::BAD_REQUEST, "invalid_config", e),
    };
    let id = uuid::Uuid::new_v4().to_string();
    let data = data_dir(&cfg);
    let out = artifact::output_dir(&data, &id).expect("generated UUID");
    let work = data.join("nlu-train").join(&id);
    let input = json!({"intents":nlu.intents.iter().map(|i| json!({"name":i.name,"templates":i.templates})).collect::<Vec<_>>(),"slots":nlu.slots});
    let mut j = job().lock().await;
    if j.running {
        return error(
            StatusCode::CONFLICT,
            "build_running",
            "A model build is already running",
        );
    }
    *j = BuildJob {
        running: true,
        state: "running",
        build_id: Some(id.clone()),
        source_revision: Some(revision.clone()),
        output_dir: Some(out.display().to_string()),
        ..BuildJob::default()
    };
    let image = std::env::var("NLU_TRAINER_IMAGE").unwrap_or_else(|_| TRAINER_IMAGE.into());
    let candidate = Artifact {
        build_id: id.clone(),
        source_revision: revision.clone(),
        source_input_sha256: artifact::sha256(input.to_string().as_bytes()),
        trainer_image: image,
        trainer_image_id: String::new(),
        trainer_script_sha256: artifact::sha256(TRAINER_SOURCE.as_bytes()),
        labels: nlu
            .intents
            .iter()
            .map(|i| i.name.clone())
            .chain(std::iter::once("other".into()))
            .collect(),
        catalog_source: Some("effective_command_nlu_v1".into()),
        files: Vec::new(),
        nlu,
    };
    drop(j);
    tokio::spawn(training::run_training(work, out.clone(), input, candidate));
    Json(json!({"started":true,"state":"running","build_id":id,"source_revision":revision,"output_dir":out})).into_response()
}

fn status_json(j: &BuildJob) -> Value {
    let artifact = j
        .artifact
        .as_ref()
        .zip(j.output_dir.as_ref())
        .map(|(a, p)| a.public_json(Path::new(p)));
    let expected = artifact.as_ref().and_then(|a| a.get("expected_config")).cloned().unwrap_or_else(|| json!({
        "bert_enabled":true,"bert_model":"<output_dir>/model.onnx","bert_vocab":"<output_dir>/tokenizer.json","bert_labels":"<output_dir>/labels.txt"}));
    json!({"type":"state","state":j.state,"running":j.running,"error":j.error,"log_tail":j.log,
        "build_id":j.build_id,"source_revision":j.source_revision,"output_dir":j.output_dir,
        "artifact":artifact,"expected_config":expected,"auto_activated":false})
}

pub(super) async fn build_status(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if let Some(r) = access(&engines, &headers, &query, false) {
        return r;
    }
    Json(status_json(&*job().lock().await)).into_response()
}

pub(super) async fn build_log_ws(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Response {
    if let Some(r) = access(&engines, &headers, &query, false) {
        return r;
    }
    ws.on_upgrade(stream_build_log)
}

async fn stream_build_log(mut socket: WebSocket) {
    let mut rx = log_tx().subscribe();
    let (snapshot, running) = {
        let j = job().lock().await;
        (status_json(&j), j.running)
    };
    if socket
        .send(Message::Text(snapshot.to_string().into()))
        .await
        .is_err()
    {
        return;
    }
    if !running {
        let _ = socket.send(Message::Close(None)).await;
        return;
    }
    loop {
        let frame = match rx.recv().await {
            Ok(BuildEvent::Log(line)) => json!({"type":"log","line":line}),
            Ok(BuildEvent::Done) => {
                let frame = status_json(&*job().lock().await);
                let _ = socket.send(Message::Text(frame.to_string().into())).await;
                let _ = socket.send(Message::Close(None)).await;
                return;
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                json!({"type":"log","line":format!("…丢失 {n} 行日志")})
            }
            Err(broadcast::error::RecvError::Closed) => return,
        };
        if socket
            .send(Message::Text(frame.to_string().into()))
            .await
            .is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
mod tests;
