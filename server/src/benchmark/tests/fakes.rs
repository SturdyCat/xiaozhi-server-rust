//! Test doubles only; production service/factories remain real. Fake results never claim native proof.
use super::*;
use crate::plugins::registry::{Built, Capability};
use crate::plugins::{
    asr::AsrEngine,
    llm::{LlmEvent, LlmProvider, LlmTurnResult, ToolSpec},
    skill::SkillRegistry,
    tts::{TtsChunkCallback, TtsEngine},
    vad::VadEngine,
};
use std::{
    future::Future,
    pin::Pin,
    sync::{atomic::AtomicUsize, Condvar, Mutex},
};

pub(super) struct CountingStore {
    pub db: Arc<crate::config::store::sqlite::SqliteConfigStore>,
    pub loads: AtomicUsize,
    pub writes: AtomicUsize,
    pub fail: AtomicBool,
}
impl CountingStore {
    pub fn new(cfg: &Config) -> Arc<Self> {
        let db = crate::config::store::sqlite::SqliteConfigStore::open(":memory:").unwrap();
        db.persist(cfg).unwrap();
        Arc::new(Self {
            db,
            loads: AtomicUsize::new(0),
            writes: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
        })
    }
}
impl ConfigStore for CountingStore {
    fn location(&self) -> Option<&str> {
        self.db.location()
    }
    fn backend(&self) -> &'static str {
        "sqlite"
    }
    fn load(&self) -> anyhow::Result<Config> {
        self.loads.fetch_add(1, Ordering::Relaxed);
        if self.fail.load(Ordering::Relaxed) {
            anyhow::bail!("secret database error")
        };
        self.db.load()
    }
    fn persist(&self, cfg: &Config) -> anyhow::Result<()> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.db.persist(cfg)
    }
    fn load_overlay(&self, id: &str) -> anyhow::Result<Option<Value>> {
        self.db.load_overlay(id)
    }
    fn save_overlay(&self, id: &str, p: &Value) -> anyhow::Result<()> {
        self.db.save_overlay(id, p)
    }
    fn delete_overlay(&self, id: &str) -> anyhow::Result<()> {
        self.db.delete_overlay(id)
    }
    fn list_presets(&self) -> anyhow::Result<Vec<String>> {
        self.db.list_presets()
    }
    fn save_preset(&self, n: &str, c: &Config) -> anyhow::Result<()> {
        self.db.save_preset(n, c)
    }
    fn load_preset(&self, n: &str) -> anyhow::Result<Option<Config>> {
        self.db.load_preset(n)
    }
    fn delete_preset(&self, n: &str) -> anyhow::Result<()> {
        self.db.delete_preset(n)
    }
    fn query(&self, s: &str) -> anyhow::Result<Vec<serde_json::Map<String, Value>>> {
        self.db.query(s)
    }
    fn backup(&self, p: &str) -> anyhow::Result<()> {
        self.db.backup(p)
    }
    fn is_empty(&self) -> bool {
        self.db.is_empty()
    }
}
#[derive(Default)]
pub(super) struct FakeProviders {
    pub seen: Arc<Mutex<Vec<Value>>>,
    pub fail: bool,
    pub no_speech: bool,
    pub empty_asr: bool,
    pub cancelled_llm: bool,
    pub mutate_on_nlu: Option<Arc<CountingStore>>,
    pub blocker: Option<Arc<Blocker>>,
}
impl providers::ProviderFactory for FakeProviders {
    fn build(&self, cfg: &Config, cap: Capability) -> anyhow::Result<Built> {
        self.seen.lock().unwrap().push(json!({"stage":cap.id(),"model":cfg.llm.model,"api_key":cfg.llm.api_key,"lang":cfg.tts.kokoro.lang,"voice":cfg.tts.xfyun.voice,"speed":cfg.tts.speed}));
        if self.fail {
            anyhow::bail!("provider secret sk-private https://user:password@provider.test")
        }
        match cap {
            Capability::Asr => Ok(Built::Asr(Arc::new(FakeAsr {
                empty: self.empty_asr,
                seen: self.seen.clone(),
            }))),
            Capability::Tts => Ok(Built::Tts(Arc::new(FakeTts {
                blocker: self.blocker.clone(),
            }))),
            Capability::Llm => Ok(Built::Llm(Arc::new(FakeLlm {
                seen: self.seen.clone(),
                cancelled: self.cancelled_llm,
            }))),
            _ => anyhow::bail!("unsupported fake"),
        }
    }
    fn vad(&self, _: &Config) -> anyhow::Result<Box<dyn VadEngine>> {
        if self.fail {
            anyhow::bail!("secret vad")
        };
        Ok(Box::new(FakeVad {
            samples: vec![],
            no_speech: self.no_speech,
        }))
    }
    fn skills(&self, cfg: &Config) -> anyhow::Result<SkillRegistry> {
        self.seen
            .lock()
            .unwrap()
            .push(json!({"stage":"nlu","model":cfg.llm.model}));
        if let Some(store) = &self.mutate_on_nlu {
            let mut new = cfg.clone();
            new.llm.model = "new-after-snapshot".into();
            store.db.persist(&new).unwrap();
        }
        providers::NativeProviders.skills(cfg)
    }
}
struct FakeAsr {
    empty: bool,
    seen: Arc<Mutex<Vec<Value>>>,
}
impl AsrEngine for FakeAsr {
    fn recognize(&self, s: &[f32], rate: u32) -> anyhow::Result<String> {
        self.seen.lock().unwrap().push(json!({"stage":"asr.input","samples":s.len(),"rate":rate,"energy":s.iter().map(|v|v.abs() as f64).sum::<f64>()}));
        Ok(if self.empty {
            String::new()
        } else {
            "Tell me an unusual story about galaxies".into()
        })
    }
}
struct FakeVad {
    samples: Vec<f32>,
    no_speech: bool,
}
impl VadEngine for FakeVad {
    fn accept(&mut self, s: &[f32], _: &mut dyn FnMut(Vec<f32>)) {
        self.samples.extend_from_slice(s)
    }
    fn flush(&mut self, cb: &mut dyn FnMut(Vec<f32>)) {
        if !self.no_speech && !self.samples.is_empty() {
            cb(std::mem::take(&mut self.samples))
        }
    }
}
struct FakeLlm {
    seen: Arc<Mutex<Vec<Value>>>,
    cancelled: bool,
}
impl LlmProvider for FakeLlm {
    fn chat_stream<'a>(
        &'a self,
        history: &'a [(String, String)],
        user_text: &'a str,
        prompt: &'a TurnPrompt,
        tools: Option<&'a [ToolSpec]>,
        mut cb: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<LlmTurnResult>> + Send + 'a>> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(json!({"stage":"llm.input","history":history.len(),"text":user_text,"instructions":prompt.instructions,"memory":prompt.memory,"tools":tools.is_some()}));
            cb(LlmEvent::Text("A bounded test reply.".into()));
            Ok(LlmTurnResult {
                text: "A bounded test reply.".into(),
                cancelled: self.cancelled,
                ..Default::default()
            })
        })
    }
}
#[derive(Default)]
pub(super) struct Blocker {
    pub entered: tokio::sync::Notify,
    pub release: (Mutex<bool>, Condvar),
}
struct FakeTts {
    blocker: Option<Arc<Blocker>>,
}
impl TtsEngine for FakeTts {
    fn name(&self) -> &'static str {
        "fake_tts"
    }
    fn synthesize_stream(
        &self,
        _: &str,
        _: f32,
        _: i32,
        mut cb: TtsChunkCallback,
    ) -> anyhow::Result<()> {
        if let Some(blocker) = &self.blocker {
            blocker.entered.notify_one();
            let (lock, cv) = &blocker.release;
            let guard = lock.lock().unwrap();
            let _ = cv
                .wait_timeout_while(guard, Duration::from_secs(2), |released| !*released)
                .unwrap();
        }
        cb(24000, &[0.1; 240]);
        Ok(())
    }
}
pub(super) fn service(
    cfg: &Config,
    factory: Arc<dyn providers::ProviderFactory>,
) -> (Arc<TestService>, Arc<CountingStore>) {
    let store = CountingStore::new(cfg);
    let gates = crate::workflow::pipeline::inference::InferenceGates::new(cfg);
    (
        Arc::new(TestService {
            store: store.clone(),
            asr_gate: gates.asr(),
            tts_gate: gates.tts(),
            factory,
            limit: Arc::new(tokio::sync::Semaphore::new(2)),
            read_limit: Arc::new(tokio::sync::Semaphore::new(4)),
            timeout: Duration::from_secs(45),
        }),
        store,
    )
}
pub(super) fn request(action: &str, text: Option<&str>) -> TestRequest {
    TestRequest {
        action_id: action.into(),
        text: text.map(str::to_owned),
        ..Default::default()
    }
}
/// Real Axum test router over loopback, avoiding any new test dependency.
pub(super) async fn http(
    service: Arc<TestService>,
    token: &str,
) -> (String, tokio::task::JoinHandle<()>) {
    let mut server = Config::default().server;
    server.expected_token = token.into();
    let app = crate::gateway::ws::test_bench_router(service, server);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, handle)
}
struct AbortServer(tokio::task::JoinHandle<()>);
impl Drop for AbortServer {
    fn drop(&mut self) {
        self.0.abort()
    }
}
pub(super) fn server_guard(h: tokio::task::JoinHandle<()>) -> impl Drop {
    AbortServer(h)
}
// Reset after each slow-worker test even if an assertion panics.
pub(super) struct ReleaseOnDrop(pub Arc<Blocker>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        *self.0.release.0.lock().unwrap() = true;
        self.0.release.1.notify_all();
    }
}
