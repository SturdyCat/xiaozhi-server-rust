//! Read-only invocability projection. No model constructors, remote requests or health writes.
use super::{contracts::*, samples};
use crate::{
    config::Config,
    plugins::{
        health::{probe, ProbeState},
        registry::{active_impl, Capability},
    },
};
use serde_json::{json, Value};

const ACTIONS: &[(&str, &str)] = &[
    ("vad.sample", "sample_or_pcm"),
    ("asr.sample", "sample_or_pcm"),
    ("asr.recording", "pcm"),
    ("nlu.classify", "text"),
    ("llm.chat", "text"),
    ("tts.synthesis", "text"),
    ("tts.playback", "native"),
    ("pipeline.text", "text"),
    ("pipeline.audio", "sample_or_pcm"),
    ("input.profile", "none"),
    ("wakeup.preview", "none"),
    ("command.match", "text"),
    ("soul.preview", "none"),
    ("memory.recall", "text"),
    ("output.profile", "none"),
    ("memory.writeback.dry_run", "text"),
];

pub(super) fn body(cfg: &Config, revision: &str) -> Value {
    let actions: Vec<TestCapability> = ACTIONS
        .iter()
        .map(|&(action, input)| {
            let reason = unavailable_reason(cfg, action);
            TestCapability {
                action_id: action,
                available: reason.is_none(),
                readiness: if reason.is_none() {
                    "configured"
                } else {
                    "unavailable"
                },
                reason,
                input_kind: input,
            }
        })
        .collect();
    json!({"source":"saved","revision":revision,"actions":actions,"samples":samples::catalogue(),
        "limits":{"text_bytes":MAX_TEXT,"audio_seconds":15,"audio_sample_rate":16000,"audio_channels":1,"body_bytes":MAX_BODY_BYTES,"concurrent_tests":2,"timeout_seconds":TEST_TIMEOUT_SECS},
        "note":"configured means invocable with valid input; no provider request/inference has been performed. Timings measure provider output, never socket delivery or playback.",
        "snapshot_support":{"saved":true,"draft_patch":false,"device_overlay":false}})
}
fn provider(cfg: &Config, cap: Capability) -> Option<String> {
    if matches!(cap, Capability::Vad | Capability::Asr) && !cfg!(feature = "sherpa") {
        return Some(
            "Real audio provider requires --features sherpa; compilation stubs are not tests"
                .into(),
        );
    }
    let Some(d) = active_impl(cfg, cap).and_then(crate::plugins::registry::find) else {
        return Some(format!("No selected {} provider", cap.id()));
    };
    if (d.validate)(cfg).is_err() {
        return Some(format!(
            "Saved {} provider configuration is invalid; check required fields and build features",
            cap.id()
        ));
    }
    if matches!(cap, Capability::Asr | Capability::Vad | Capability::Tts) {
        if let Some(p) = probe(cfg, cap) {
            if p.state == ProbeState::Fail {
                return Some(format!(
                    "Saved {} model assets are missing; check configured local files",
                    cap.id()
                ));
            }
        }
    }
    None
}
fn nlu(cfg: &Config) -> Option<String> {
    if crate::plugins::skill::validate(&cfg.command, &cfg.nlu).is_err() {
        return Some("Saved command/NLU templates, thresholds or actions are invalid".into());
    }
    if cfg.nlu.bert_enabled {
        if !crate::plugins::nlu::bert_feature_enabled() {
            return Some("Configured TinyBERT requires --features nlu".into());
        }
        if !cfg.nlu.bert_configured() {
            return Some("TinyBERT model/vocab/labels configuration incomplete".into());
        }
        if [
            &cfg.nlu.bert_model,
            &cfg.nlu.bert_vocab,
            &cfg.nlu.bert_labels,
        ]
        .iter()
        .any(|p| !std::path::Path::new(p).is_file())
        {
            return Some("TinyBERT model/vocab/labels file missing".into());
        }
    }
    None
}
fn pipeline_reason(cfg: &Config, audio: bool) -> Option<String> {
    if cfg.aiui.enabled {
        return Some("AIUI managed full-chain has no isolated local pipeline test adapter".into());
    }
    let audio_check = if audio {
        provider(cfg, Capability::Vad).or_else(|| provider(cfg, Capability::Asr))
    } else {
        None
    };
    audio_check
        .or_else(|| nlu(cfg))
        .or_else(|| {
            if cfg.llm.enabled {
                provider(cfg, Capability::Llm)
            } else {
                None
            }
        })
        .or_else(|| provider(cfg, Capability::Tts))
}
pub(super) fn unavailable_reason(cfg: &Config, action: &str) -> Option<String> {
    match action {
        "vad.sample"=>provider(cfg,Capability::Vad),
        "asr.sample"|"asr.recording"=>provider(cfg,Capability::Asr),
        "nlu.classify"=>nlu(cfg),
        "llm.chat"=>provider(cfg,Capability::Llm),
        "tts.synthesis"=>provider(cfg,Capability::Tts),
        "pipeline.text" | "pipeline.audio" => pipeline_reason(cfg, action == "pipeline.audio"),
        "input.profile"|"output.profile"=>None,
        "wakeup.preview"=>cfg.wakeup.validate().err().map(|_|"Invalid saved wakeup greeting configuration".into()),
        "command.match"=>cfg.command.validate().err().map(|_|"Invalid saved command matching configuration".into()),
        "soul.preview"=>crate::plugins::soul::preview::build(cfg,"<node-test>").err().map(|_|"Invalid saved soul preset or prompt variables".into()),
        "memory.recall"|"memory.writeback.dry_run"=>Some("No snapshot-isolated read-only memory test adapter; existing scoped memory API remains available".into()),
        "tts.playback"=>Some("Native client playback is not an HTTP server capability".into()),
        _=>Some("Unknown test action_id".into()),
    }
}
