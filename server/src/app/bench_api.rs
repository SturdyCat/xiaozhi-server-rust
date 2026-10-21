//! Unified, isolated node tests. Saved SQLite snapshot once per request; no engine publication,
//! production session state, config writes, device actions, memory writes or audio caches.
//! The default feature build never passes audio compilation stubs off as real providers.
use crate::app::inference::GateHandle;
use crate::plugins::prompt::{MemoryPosition, TurnPrompt};
use crate::{
    config::{store::ConfigStore, Config},
    engine::Engines,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};
use std::time::{Duration, Instant};

mod capabilities;
mod contracts;
mod features;
mod models;
mod pipeline;
mod providers;
mod samples;
#[cfg(test)]
mod tests;
use contracts::{validate_request, TEST_TIMEOUT_SECS};
pub use contracts::{BenchResult, StageResult, TestRequest, MAX_BODY_BYTES};
pub use models::ModelsResult;
use providers::{NativeProviders, ProviderFactory, RunContext};

pub(super) struct Snapshot {
    pub cfg: Config,
    pub revision: String,
}
impl Snapshot {
    fn new(cfg: Config) -> Self {
        let revision = crate::app::ws::config_revision(&cfg);
        Self { cfg, revision }
    }
}
pub(crate) struct TestService {
    store: Arc<dyn ConfigStore>,
    asr_gate: GateHandle,
    tts_gate: GateHandle,
    factory: Arc<dyn ProviderFactory>,
    limit: Arc<tokio::sync::Semaphore>,
    read_limit: Arc<tokio::sync::Semaphore>,
    timeout: Duration,
}
impl TestService {
    pub(crate) fn from_engines(engines: &Arc<Engines>) -> Arc<Self> {
        Arc::new(Self {
            store: engines.config_store.clone(),
            asr_gate: engines.inference.asr(),
            tts_gate: engines.inference.tts(),
            factory: Arc::new(NativeProviders),
            limit: admission(),
            read_limit: snapshot_admission(),
            timeout: Duration::from_secs(TEST_TIMEOUT_SECS),
        })
    }
    pub(crate) async fn capabilities(&self) -> Value {
        match snapshot(self.store.clone(), self.read_limit.clone(), None).await {
            Ok(s) => capabilities::body(&s.cfg, &s.revision),
            Err(e) => {
                json!({"source":"saved","revision":null,"actions":[],"samples":[],"state":"error","error_code":"snapshot_unavailable","error":e})
            }
        }
    }
    pub(crate) async fn models(
        &self,
        api_base: Option<&str>,
        api_key: Option<&str>,
    ) -> ModelsResult {
        models::from_service(self, api_base, api_key).await
    }
    pub(crate) async fn run(&self, req: TestRequest) -> BenchResult {
        let started = Instant::now();
        let action = req.action_id.clone();
        if let Err(e) = validate_request(&req) {
            return failure(&action, None, "invalid_input", e);
        }
        let slot = match self.limit.clone().try_acquire_owned() {
            Ok(v) => Arc::new(v),
            Err(_) => {
                return failure(
                    &action,
                    None,
                    "busy",
                    "Two diagnostics are already running; retry after completion",
                )
            }
        };
        let saved = match snapshot(
            self.store.clone(),
            self.read_limit.clone(),
            Some(slot.clone()),
        )
        .await
        {
            Ok(s) => s,
            Err(e) => return failure(&action, None, "snapshot_unavailable", e),
        };
        let mut out = run_snapshot(
            saved,
            req,
            self.asr_gate.clone(),
            self.tts_gate.clone(),
            self.factory.clone(),
            slot,
            self.timeout.saturating_sub(started.elapsed()),
        )
        .await;
        out.result.elapsed_ms = started.elapsed().as_millis() as u64;
        out
    }
}
fn snapshot_admission() -> Arc<tokio::sync::Semaphore> {
    static READS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    limiter(&READS, 4)
}
async fn snapshot(
    store: Arc<dyn ConfigStore>,
    reads: Arc<tokio::sync::Semaphore>,
    keepalive: Option<Arc<tokio::sync::OwnedSemaphorePermit>>,
) -> Result<Snapshot, String> {
    let read = reads
        .try_acquire_owned()
        .map_err(|_| "Saved configuration snapshot reads are busy".to_string())?;
    let worker = tokio::task::spawn_blocking(move || {
        let _read = read;
        let _keepalive = keepalive;
        store.load().map(Snapshot::new)
    });
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .map_err(|_| {
            "Saved configuration snapshot timed out; read capacity remains held until completion"
                .to_string()
        })?
        .map_err(|_| "Configuration snapshot worker failed".to_string())?
        .map_err(|_| {
            "Cannot read saved SQLite configuration; no startup/live fallback was used".to_string()
        })
}
fn limiter(
    cell: &'static OnceLock<Arc<tokio::sync::Semaphore>>,
    capacity: usize,
) -> Arc<tokio::sync::Semaphore> {
    cell.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(capacity)))
        .clone()
}
fn admission() -> Arc<tokio::sync::Semaphore> {
    static LIMIT: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    limiter(&LIMIT, 2)
}
fn failure(
    action: &str,
    revision: Option<String>,
    code: &'static str,
    error: impl Into<String>,
) -> BenchResult {
    BenchResult::new(action, revision, StageResult::error(action, code, error))
}
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
async fn run_snapshot(
    saved: Snapshot,
    req: TestRequest,
    asr_gate: GateHandle,
    tts_gate: GateHandle,
    factory: Arc<dyn ProviderFactory>,
    slot: Arc<tokio::sync::OwnedSemaphorePermit>,
    timeout: Duration,
) -> BenchResult {
    let t = Instant::now();
    let action = req.action_id.clone();
    let revision = saved.revision;
    if req
        .expected_revision
        .as_ref()
        .is_some_and(|v| v != &revision)
    {
        return failure(
            &action,
            Some(revision),
            "revision_conflict",
            "Saved config changed; reload before testing",
        );
    }
    let mut cfg = saved.cfg;
    let overridden = req.api_base.is_some()
        || req.api_key.is_some()
        || req.lang.is_some()
        || req.vcn.is_some()
        || req.speed.is_some();
    if let Some(v) = &req.api_base {
        cfg.llm.api_base = v.clone();
    }
    if let Some(v) = &req.api_key {
        cfg.llm.api_key = v.clone();
    }
    if let Some(v) = &req.lang {
        cfg.tts.kokoro.lang = v.clone();
    }
    if let Some(v) = &req.vcn {
        cfg.tts.xfyun.voice = v.clone();
    }
    if let Some(v) = req.speed {
        cfg.tts.speed = v;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let _cancel_on_drop = CancelOnDrop(cancel.clone());
    let ctx = RunContext {
        cfg: Arc::new(cfg),
        factory,
        asr_gate,
        tts_gate,
        cancel,
        slot,
    };
    let mut out = match tokio::time::timeout(timeout, dispatch(&ctx, &req, revision.clone())).await
    {
        Ok(r) => r,
        Err(_) => failure(
            &action,
            Some(revision),
            "timeout",
            "Diagnostic deadline exceeded; native worker capacity remains held until it returns",
        ),
    };
    out.result.elapsed_ms = t.elapsed().as_millis() as u64;
    if overridden {
        out.source = "saved+request";
    }
    out
}
async fn dispatch(ctx: &RunContext, req: &TestRequest, revision: String) -> BenchResult {
    let action = req.action_id.as_str();
    if action == "pipeline.text" || action == "pipeline.audio" {
        return pipeline::run(ctx, req, revision).await;
    }
    let text = req.text.clone().unwrap_or_default();
    let result = match action {
        "vad.sample" | "asr.sample" | "asr.recording" => {
            if action == "asr.recording" && req.audio.is_none() {
                StageResult::error(
                    "asr",
                    "invalid_input",
                    "asr.recording requires typed PCM audio",
                )
            } else {
                match samples::audio(req) {
                    Ok((samples, provenance)) => {
                        let mut result = if action == "vad.sample" {
                            providers::vad(ctx, samples).await.0
                        } else {
                            providers::asr(ctx, samples).await
                        };
                        result.details["input"] = provenance;
                        result
                    }
                    Err(e) => StageResult::error(action, "invalid_input", e),
                }
            }
        }
        "nlu.classify" | "llm.chat" | "tts.synthesis" if text.trim().is_empty() => {
            StageResult::error(action, "invalid_input", "text is required")
        }
        "nlu.classify" => providers::nlu(ctx, text).await,
        "llm.chat" => {
            providers::llm(
                ctx,
                text,
                TurnPrompt {
                    instructions: ctx.cfg.llm.system_prompt.clone(),
                    memory: None,
                    position: MemoryPosition::default(),
                },
            )
            .await
        }
        "tts.synthesis" => providers::tts(ctx, text).await,
        _ => features::preview(&ctx.cfg, action, &text),
    };
    let mut out = BenchResult::new(action, Some(revision), result.clone());
    out.stages.push(result);
    out
}
