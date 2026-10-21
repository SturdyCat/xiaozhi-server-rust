//! Isolated real provider factories and common measured execution; never publish/cache engines.
use super::contracts::StageResult;
use crate::app::inference::GateHandle;
use crate::config::Config;
use crate::plugins::{
    registry::{active_impl, Built, Capability},
    skill::SkillRegistry,
    vad::VadEngine,
};
use anyhow::{anyhow, Result};
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Instant;

pub(super) trait ProviderFactory: Send + Sync {
    fn build(&self, cfg: &Config, cap: Capability) -> Result<Built>;
    fn vad(&self, cfg: &Config) -> Result<Box<dyn VadEngine>>;
    fn skills(&self, cfg: &Config) -> Result<SkillRegistry>;
    fn native(&self) -> bool {
        false
    }
}
pub(super) struct NativeProviders;
impl ProviderFactory for NativeProviders {
    fn native(&self) -> bool {
        true
    }
    fn build(&self, cfg: &Config, cap: Capability) -> Result<Built> {
        let d = active_impl(cfg, cap)
            .and_then(crate::plugins::registry::find)
            .ok_or_else(|| anyhow!("No selected provider for {}", cap.id()))?;
        (d.validate)(cfg)?;
        d.build
            .ok_or_else(|| anyhow!("Provider has no standalone constructor"))?(cfg)
    }
    fn vad(&self, cfg: &Config) -> Result<Box<dyn VadEngine>> {
        #[cfg(feature = "sherpa")]
        {
            Ok(Box::new(crate::plugins::vad::SherpaVad::new(&cfg.vad)?))
        }
        #[cfg(not(feature = "sherpa"))]
        {
            let _ = cfg;
            anyhow::bail!(
                "Silero VAD requires --features sherpa; compilation stub is not a test provider"
            )
        }
    }
    fn skills(&self, cfg: &Config) -> Result<SkillRegistry> {
        crate::plugins::skill::validate(&cfg.command, &cfg.nlu)?;
        let bert = if cfg.nlu.bert_enabled {
            Some(crate::plugins::nlu::build_bert_checked(&cfg.nlu)?)
        } else {
            None
        };
        Ok(SkillRegistry::new(&cfg.command, &cfg.nlu, bert))
    }
}

#[derive(Clone)]
pub(super) struct RunContext {
    pub cfg: Arc<Config>,
    pub factory: Arc<dyn ProviderFactory>,
    pub asr_gate: GateHandle,
    pub tts_gate: GateHandle,
    pub cancel: Arc<AtomicBool>,
    // A timed-out/dropped HTTP future cannot release capacity held by a native worker.
    pub slot: Arc<tokio::sync::OwnedSemaphorePermit>,
}
impl RunContext {
    pub async fn blocking<F, T>(
        &self,
        stage: &'static str,
        gate: Option<GateHandle>,
        f: F,
    ) -> Result<T, StageResult>
    where
        F: FnOnce(Self) -> Result<T, StageResult> + Send + 'static,
        T: Send + 'static,
    {
        let started = Instant::now();
        let permit = if let Some(gate) = gate {
            Some(
                gate.acquire()
                    .await
                    .ok_or_else(|| StageResult::error(stage, "busy", "Inference gate closed"))?,
            )
        } else {
            None
        };
        let ctx = self.clone();
        tokio::task::spawn_blocking(move || {
            let _gate = permit;
            let _slot = ctx.slot.clone();
            if ctx.cancel.load(Ordering::Relaxed) {
                return Err(StageResult::error(stage, "timeout", "Test cancelled"));
            }
            f(ctx)
        })
        .await
        .unwrap_or_else(|_| {
            Err(StageResult::error(
                stage,
                "worker_failed",
                "Provider worker failed",
            ))
        })
        .map_err(|mut error| {
            error.elapsed_ms = ms(started);
            error
        })
    }
    fn verified(&self, r: &mut StageResult) {
        r.readiness = if self.factory.native() {
            "provider_verified"
        } else {
            "configured"
        };
        if !self.factory.native() {
            r.details["test_double"] = json!(true);
        }
    }
}
fn ms(t: Instant) -> u64 {
    t.elapsed().as_millis() as u64
}
fn unavailable(stage: &str, _e: anyhow::Error) -> StageResult {
    // Factory details may contain server-local paths/credentials. Return a stable actionable error.
    StageResult::error(stage,"provider_unavailable",format!("{stage} provider could not be built; check saved configuration, model files and build features"))
}

pub(super) async fn vad(ctx: &RunContext, samples: Vec<f32>) -> (StageResult, Vec<f32>) {
    ctx.blocking("vad", None, move |ctx| {
        let t = Instant::now();
        let mut vad = ctx.factory.vad(&ctx.cfg).map_err(|e| unavailable("vad",e))?;
        let build_ms = ms(t);
        let mut speech = Vec::new();
        let mut count = 0;
        let mut first = None;
        let mut overflow = false;
        let mut cb = |seg: Vec<f32>| {
            if first.is_none() { first = Some(ms(t)); }
            count += 1;
            if speech.len() + seg.len() <= super::contracts::MAX_AUDIO_SAMPLES { speech.extend(seg); } else { overflow = true; }
        };
        for chunk in samples.chunks(512) {
            if ctx.cancel.load(Ordering::Relaxed) { return Err(StageResult::error("vad","timeout","VAD cancelled")); }
            vad.accept(chunk, &mut cb);
        }
        vad.flush(&mut cb);
        vad.reset();
        if overflow { return Err(StageResult::error("vad","output_limit","VAD output exceeds input budget")); }
        let mut r = StageResult::ok("vad",ms(t),"silero");
        r.first_packet_ms = first;
        r.measurement = "provider_segment";
        r.text = Some(format!("{count} speech segments; {} speech samples",speech.len()));
        r.details = json!({"build_ms":build_ms,"segments":count,"speech_samples":speech.len(),"first_segment_ms":first,"sample_rate":16000});
        ctx.verified(&mut r);
        Ok((r,speech))
    })
    .await
    .unwrap_or_else(|e| (e, vec![]))
}

pub(super) async fn asr(ctx: &RunContext, samples: Vec<f32>) -> StageResult {
    ctx.blocking("asr", Some(ctx.asr_gate.clone()), move |ctx| {
        let t = Instant::now();
        let Built::Asr(asr) = ctx.factory.build(&ctx.cfg,Capability::Asr).map_err(|e| unavailable("asr",e))? else { return Err(StageResult::error("asr","provider_unavailable","Wrong ASR provider type")); };
        let build_ms = ms(t);
        let text = asr.recognize(&samples,16000).map_err(|_| StageResult::error("asr","provider_error","ASR inference failed"))?;
        if text.len() > 16384 { return Err(StageResult::error("asr","output_limit","ASR text exceeds 16 KiB")); }
        if text.trim().is_empty() { return Err(StageResult::error("asr","no_transcript","ASR returned no transcript; this is not speech-recognition success")); }
        let mut r = StageResult::ok("asr",ms(t),&ctx.cfg.asr.engine_id());
        r.text = Some(text);
        r.measurement = "provider_transcript";
        r.details = json!({"build_ms":build_ms,"input_samples":samples.len(),"sample_rate":16000,"speech_attested":false});
        ctx.verified(&mut r);
        Ok(r)
    }).await.unwrap_or_else(|r|r)
}

pub(super) async fn nlu(ctx: &RunContext, text: String) -> StageResult {
    ctx.blocking("nlu",None,move |ctx| {
        let t = Instant::now();
        let registry = ctx.factory.skills(&ctx.cfg).map_err(|e| unavailable("nlu",e))?;
        let hit = registry.parse_staged(&text);
        let mut r = StageResult::ok("nlu",ms(t),"skill_registry");
        r.measurement = "dry_run_classification";
        r.details = match hit {
            Some((stage,inv)) => { r.text = Some(inv.reply.clone()); json!({"matched":true,"recognition_stage":stage.as_str(),"skill":inv.name,"slots":inv.slots,"action":format!("{:?}",inv.action),"reply":inv.reply,"dry_run":true,"executed":false}) },
            None => json!({"matched":false,"next":"llm","dry_run":true,"executed":false}),
        };
        ctx.verified(&mut r);
        Ok(r)
    }).await.unwrap_or_else(|r|r)
}

pub(super) async fn llm(
    ctx: &RunContext,
    text: String,
    prompt: crate::plugins::prompt::TurnPrompt,
) -> StageResult {
    let t = Instant::now();
    let llm = match ctx.factory.build(&ctx.cfg, Capability::Llm) {
        Ok(Built::Llm(v)) => v,
        _ => return unavailable("llm", anyhow!("LLM construction failed")),
    };
    let stats = Arc::new(Mutex::new((None, 0usize, false)));
    let cb_stats = stats.clone();
    let cancel = ctx.cancel.clone();
    let out = llm
        .chat_stream(
            &[],
            &text,
            &prompt,
            None,
            Box::new(move |ev| {
                if cancel.load(Ordering::Relaxed) {
                    return false;
                }
                if let crate::plugins::llm::LlmEvent::Text(s) = ev {
                    let mut g = cb_stats.lock().unwrap_or_else(|e| e.into_inner());
                    if !s.is_empty() && g.0.is_none() {
                        g.0 = Some(ms(t));
                    }
                    g.1 += s.len();
                    if g.1 > 16384 {
                        g.2 = true;
                        return false;
                    }
                }
                true
            }),
        )
        .await;
    let (first, _, limited) = *stats.lock().unwrap_or_else(|e| e.into_inner());
    if limited {
        return StageResult::error("llm", "output_limit", "LLM output exceeds 16 KiB");
    }
    match out {
        Ok(v) if v.cancelled => StageResult::error(
            "llm",
            "cancelled",
            "LLM returned a cancelled partial result",
        ),
        Ok(v) if v.text.len() <= 16384 && !v.text.trim().is_empty() => {
            let mut r = StageResult::ok("llm", ms(t), &ctx.cfg.llm.model);
            r.first_packet_ms = first;
            r.first_token_ms = first;
            r.text = Some(v.text);
            r.measurement = "provider_token";
            r.details = json!({"history_turns":0,"device_actions":false,"socket_written":false,"played":false});
            ctx.verified(&mut r);
            r
        }
        Ok(_) => StageResult::error("llm", "empty_output", "LLM returned no bounded text output"),
        Err(_) => StageResult::error(
            "llm",
            "provider_error",
            "LLM request failed; check upstream and saved credentials",
        ),
    }
}

pub(super) async fn tts(ctx: &RunContext, text: String) -> StageResult {
    ctx.blocking("tts",Some(ctx.tts_gate.clone()),move |ctx| {
        let t = Instant::now();
        let Built::Tts(tts) = ctx.factory.build(&ctx.cfg,Capability::Tts).map_err(|e| unavailable("tts",e))? else { return Err(unavailable("tts",anyhow!("wrong type"))); };
        let build_ms = ms(t);
        let stats = Arc::new(Mutex::new((None,0usize,0u32,false)));
        let cb_stats = stats.clone();
        let cancel = ctx.cancel.clone();
        let result = tts.synthesize_stream(&text,ctx.cfg.tts.speed,ctx.cfg.tts.speaker,Box::new(move |sr,chunk| {
            if cancel.load(Ordering::Relaxed) { return false; }
            let mut g = cb_stats.lock().unwrap_or_else(|e|e.into_inner());
            if !chunk.is_empty() && g.0.is_none() { g.0 = Some(ms(t)); }
            g.1 += chunk.len();
            if sr == 0 || sr > 192000 || (g.2 != 0 && g.2 != sr) || g.1 > (sr as usize).saturating_mul(60) || chunk.iter().any(|v|!v.is_finite()) { g.3 = true; return false; }
            g.2 = sr; true
        }));
        let (first,count,sr,limited) = *stats.lock().unwrap_or_else(|e|e.into_inner());
        if ctx.cancel.load(Ordering::Relaxed) { return Err(StageResult::error("tts","timeout","TTS cancelled")); }
        if limited { return Err(StageResult::error("tts","output_limit","TTS PCM violates output budget/format")); }
        result.map_err(|_|StageResult::error("tts","provider_error","TTS synthesis failed; check provider configuration"))?;
        if count == 0 { return Err(StageResult::error("tts","empty_output","TTS produced no PCM")); }
        let mut r = StageResult::ok("tts",ms(t),tts.name());
        r.first_packet_ms = first; r.first_pcm_ms = first;
        r.measurement = "provider_pcm";
        r.details = json!({"build_ms":build_ms,"pcm_samples":count,"sample_rate":sr,"duration_ms":count as u64 * 1000 / sr as u64,"socket_written":false,"played":false});
        ctx.verified(&mut r); Ok(r)
    }).await.unwrap_or_else(|r|r)
}
