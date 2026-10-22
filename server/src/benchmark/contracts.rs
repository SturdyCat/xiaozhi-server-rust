//! Shared additive wire contract. Never serialize configuration or provider credentials.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const MAX_TEXT: usize = 4096;
pub const MAX_AUDIO_SAMPLES: usize = 16000 * 15;
pub const MAX_BODY_BYTES: usize = 768 * 1024;
pub const TEST_TIMEOUT_SECS: u64 = 45;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestRequest {
    #[serde(default)]
    pub action_id: String,
    pub text: Option<String>,
    pub sample_id: Option<String>,
    pub audio: Option<AudioInput>,
    pub expected_revision: Option<String>,
    // Legacy per-call overrides, never persisted. Unsupported draft/device fields are rejected.
    pub api_base: Option<String>,
    pub api_key: Option<String>,
    pub lang: Option<String>,
    pub vcn: Option<String>,
    pub speed: Option<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioInput {
    pub format: String,
    pub sample_rate: u32,
    pub channels: u8,
    pub data_base64: String,
}

#[derive(Clone, Serialize)]
pub struct StageResult {
    pub stage: String,
    pub state: &'static str,
    pub elapsed_ms: u64,
    pub first_packet_ms: Option<u64>,
    pub first_pcm_ms: Option<u64>,
    pub first_token_ms: Option<u64>,
    pub engine: Option<String>,
    pub text: Option<String>,
    pub error: Option<String>,
    pub error_code: Option<&'static str>,
    pub readiness: &'static str,
    pub measurement: &'static str,
    pub details: Value,
}

impl StageResult {
    pub(super) fn ok(stage: &str, elapsed_ms: u64, engine: &str) -> Self {
        Self {
            stage: stage.into(),
            state: "ok",
            elapsed_ms,
            first_packet_ms: None,
            first_pcm_ms: None,
            first_token_ms: None,
            engine: Some(engine.into()),
            text: None,
            error: None,
            error_code: None,
            readiness: "configured",
            measurement: "preview",
            details: json!({}),
        }
    }
    pub(crate) fn error(stage: &str, code: &'static str, message: impl Into<String>) -> Self {
        let mut out = Self::ok(stage, 0, "");
        out.state = if matches!(
            code,
            "provider_unavailable" | "unsupported_action" | "managed_pipeline"
        ) {
            "unavailable"
        } else {
            "error"
        };
        out.engine = None;
        out.error_code = Some(code);
        out.error = Some(message.into());
        out.readiness = if matches!(
            code,
            "provider_error" | "output_limit" | "no_transcript" | "cancelled" | "no_speech"
        ) {
            "configured"
        } else {
            "unavailable"
        };
        out
    }
    pub(super) fn skipped(stage: &str, reason: &str) -> Self {
        let mut out = Self::ok(stage, 0, "");
        out.state = "skipped";
        out.engine = None;
        out.details = json!({"reason":reason});
        out
    }
}

#[derive(Serialize)]
pub struct BenchResult {
    #[serde(flatten)]
    pub result: StageResult,
    pub request_id: String,
    pub action_id: String,
    pub source: &'static str,
    pub revision: Option<String>,
    pub context_mode: &'static str,
    pub stages: Vec<StageResult>,
}

impl BenchResult {
    pub(crate) fn new(action: &str, revision: Option<String>, result: StageResult) -> Self {
        Self {
            result,
            request_id: uuid::Uuid::new_v4().to_string(),
            action_id: action.into(),
            source: "saved",
            revision,
            context_mode: "isolated",
            stages: vec![],
        }
    }
}

#[derive(Serialize)]
pub struct TestCapability {
    pub action_id: &'static str,
    pub available: bool,
    pub reason: Option<String>,
    pub readiness: &'static str,
    pub input_kind: &'static str,
}

pub(super) fn validate_request(req: &TestRequest) -> Result<(), String> {
    if req.action_id.is_empty() || req.action_id.len() > 80 {
        return Err("action_id is required (max 80 bytes)".into());
    }
    for s in [&req.text, &req.api_base, &req.api_key, &req.lang, &req.vcn]
        .into_iter()
        .flatten()
    {
        if s.len() > MAX_TEXT {
            return Err(format!("text/override exceeds {MAX_TEXT} bytes"));
        }
    }
    if req.sample_id.as_ref().is_some_and(|s| s.len() > 80) {
        return Err("sample_id exceeds 80 bytes".into());
    }
    if req.sample_id.is_some() && req.audio.is_some() {
        return Err("choose sample_id OR audio".into());
    }
    let audio_action = matches!(
        req.action_id.as_str(),
        "vad.sample" | "asr.sample" | "asr.recording" | "pipeline.audio"
    );
    if !audio_action && (req.sample_id.is_some() || req.audio.is_some()) {
        return Err("sample_id/audio require an audio action".into());
    }
    if req
        .expected_revision
        .as_ref()
        .is_some_and(|v| v.len() > 128)
    {
        return Err("expected_revision exceeds 128 bytes".into());
    }
    if req.lang.as_ref().is_some_and(|v| {
        v.len() > 32
            || !v
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }) {
        return Err("lang must be a short language identifier".into());
    }
    if req
        .speed
        .is_some_and(|s| !s.is_finite() || !(0.5..=2.0).contains(&s))
    {
        return Err("speed must be finite and within 0.5..2.0".into());
    }
    let override_llm = req.api_base.is_some() || req.api_key.is_some();
    let override_tts = req.lang.is_some() || req.vcn.is_some() || req.speed.is_some();
    if override_llm && req.action_id != "llm.chat" {
        return Err("LLM overrides require llm.chat".into());
    }
    if override_tts && req.action_id != "tts.synthesis" {
        return Err("TTS overrides require tts.synthesis".into());
    }
    Ok(())
}
