//! Side-effect-free local pipeline diagnostic; not a production device conversation.
use super::{
    contracts::{BenchResult, StageResult, TestRequest},
    providers::{self, RunContext},
    samples,
};
use crate::plugins::prompt::{MemoryPosition, TurnPrompt};
use serde_json::json;

pub(super) async fn run(ctx: &RunContext, req: &TestRequest, revision: String) -> BenchResult {
    let t = std::time::Instant::now();
    let mut stages = vec![];
    let mut text = req.text.clone().unwrap_or_default();
    if ctx.cfg.aiui.enabled {
        return BenchResult::new(&req.action_id,Some(revision),StageResult::error("pipeline","managed_pipeline","AIUI full-chain mode has no isolated local pipeline adapter; local diagnostics are not proof of managed full-chain readiness"));
    }
    if req.action_id == "pipeline.text" {
        stages.push(StageResult::skipped("vad", "Text input bypasses VAD"));
        stages.push(StageResult::skipped("asr", "Text input bypasses ASR"));
        if text.trim().is_empty() {
            return finish(
                req,
                revision,
                stages,
                StageResult::error("pipeline", "invalid_input", "text is required"),
                t,
            );
        }
    } else {
        let (audio, provenance) = match samples::audio(req) {
            Ok(v) => v,
            Err(e) => {
                return finish(
                    req,
                    revision,
                    stages,
                    StageResult::error("pipeline", "invalid_input", e),
                    t,
                )
            }
        };
        let (mut vad, speech) = providers::vad(ctx, audio).await;
        vad.details["input"] = provenance;
        stages.push(vad.clone());
        if vad.state != "ok" {
            return fail_remaining(req, revision, stages, vad, t);
        }
        if speech.is_empty() {
            return fail_remaining(
                req,
                revision,
                stages,
                StageResult::error(
                    "vad",
                    "no_speech",
                    "No speech segment detected; ASR was not invoked",
                ),
                t,
            );
        }
        let asr = providers::asr(ctx, speech).await;
        text = asr.text.clone().unwrap_or_default();
        stages.push(asr.clone());
        if asr.state != "ok" {
            return fail_remaining(req, revision, stages, asr, t);
        }
        if text.trim().is_empty() {
            return fail_remaining(
                req,
                revision,
                stages,
                StageResult::error("asr", "no_transcript", "ASR returned no transcript"),
                t,
            );
        }
    }
    let nlu = providers::nlu(ctx, text.clone()).await;
    stages.push(nlu.clone());
    if nlu.state != "ok" {
        return fail_remaining(req, revision, stages, nlu, t);
    }
    let reply = if nlu.details["matched"] == true {
        stages.push(StageResult::skipped(
            "llm",
            "Skill matched; device action suppressed (dry run)",
        ));
        nlu.text.unwrap_or_default()
    } else if !ctx.cfg.llm.enabled {
        stages.push(StageResult::skipped(
            "llm",
            "Saved LLM disabled: echo-input diagnostic mode",
        ));
        text
    } else {
        let p = super::features::preview(&ctx.cfg, "soul.preview", "");
        if p.state != "ok" {
            stages.push(p.clone());
            return fail_remaining(req, revision, stages, p, t);
        }
        stages.push(p.clone());
        let prompt = TurnPrompt {
            instructions: p.text.unwrap_or_default(),
            memory: None,
            position: MemoryPosition::default(),
        };
        let llm = providers::llm(ctx, text, prompt).await;
        stages.push(llm.clone());
        if llm.state != "ok" {
            return fail_remaining(req, revision, stages, llm, t);
        }
        llm.text.unwrap_or_default()
    };
    if reply.trim().is_empty() {
        stages.push(StageResult::skipped(
            "tts",
            "Matched action has no reply; action not executed",
        ));
    } else {
        let tts = providers::tts(ctx, reply.clone()).await;
        stages.push(tts.clone());
        if tts.state != "ok" {
            return finish(req, revision, stages, tts, t);
        }
    }
    let mut result = StageResult::ok("pipeline", 0, "local_dry_run");
    result.text = Some(reply);
    result.readiness = if ctx.factory.native() {
        "provider_verified"
    } else {
        "configured"
    };
    result.measurement = "pipeline_preview";
    finish(req, revision, stages, result, t)
}
fn finish(
    req: &TestRequest,
    revision: String,
    stages: Vec<StageResult>,
    mut result: StageResult,
    t: std::time::Instant,
) -> BenchResult {
    result.elapsed_ms = t.elapsed().as_millis() as u64;
    result.details["context"] = json!({"history_turns":0,"soul":"saved snapshot when LLM is used","memory_recall":"not_run_no_snapshot_adapter","memory_writeback":"suppressed","device_actions":false,"socket_written":false,"played":false,"production_fullflow":false});
    let mut out = BenchResult::new(&req.action_id, Some(revision), result);
    out.context_mode = "pipeline_preview";
    out.stages = stages;
    out
}
fn fail_remaining(
    req: &TestRequest,
    revision: String,
    mut stages: Vec<StageResult>,
    error: StageResult,
    t: std::time::Instant,
) -> BenchResult {
    for id in ["vad", "asr", "nlu", "llm", "tts"] {
        if !stages.iter().any(|s| s.stage == id) {
            stages.push(StageResult::skipped(id, "Prior stage failed; not invoked"));
        }
    }
    finish(req, revision, stages, error, t)
}
