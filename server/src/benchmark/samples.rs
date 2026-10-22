//! Closed, embedded sample catalogue. No filesystem paths or remote URLs from requests.
use super::contracts::{TestRequest, MAX_AUDIO_SAMPLES};
use base64::Engine;
use serde_json::{json, Value};

pub const DEFAULT_SAMPLE: &str = "harvard.en.first";
const HARVARD: &[u8] = include_bytes!("../../test_assets/harvard_first_sentence_16k.pcm");

pub(super) fn catalogue() -> Value {
    json!([{"id": DEFAULT_SAMPLE, "label":"English · Harvard sentence (4 s)",
        "transcript":"The stale smell of old beer lingers.", "language":"en", "duration_ms":4000,
        "format":"pcm_s16le", "sample_rate":16000, "channels":1,
        "license":"MIT (Real Python repository)",
        "source":"https://github.com/realpython/python-speech-recognition/tree/0c07b810808c01144a9611faf84739f24513184e",
        "attribution":"Copyright (c) 2018 Real Python; source and derivation in docs/node-test-samples.md"}])
}

pub(super) fn audio(req: &TestRequest) -> Result<(Vec<f32>, Value), String> {
    let (bytes, source) = if let Some(audio) = &req.audio {
        if audio.format != "pcm_s16le" || audio.sample_rate != 16000 || audio.channels != 1 {
            return Err("audio requires pcm_s16le, sample_rate=16000, channels=1".into());
        }
        if audio.data_base64.len() > (MAX_AUDIO_SAMPLES * 2).div_ceil(3) * 4 {
            return Err("audio exceeds 15 seconds".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&audio.data_base64)
            .map_err(|_| "audio.data_base64 is not standard base64")?;
        (
            bytes,
            json!({"kind":"request_pcm", "provenance":"caller-supplied; speech content not attested"}),
        )
    } else {
        let id = req.sample_id.as_deref().unwrap_or(DEFAULT_SAMPLE);
        if id != DEFAULT_SAMPLE {
            return Err("unknown sample_id (use capability samples catalogue)".into());
        }
        (
            HARVARD.to_vec(),
            json!({"kind":"server_sample", "sample_id":id, "license":"MIT (Real Python repository)"}),
        )
    };
    if bytes.is_empty() || bytes.len() % 2 != 0 || bytes.len() > MAX_AUDIO_SAMPLES * 2 {
        return Err("PCM must be nonempty, even-length and at most 15 seconds".into());
    }
    let samples = bytes
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect();
    Ok((samples, source))
}
