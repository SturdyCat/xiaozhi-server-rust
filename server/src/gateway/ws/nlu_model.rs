//! Side-effect-free direct candidate ONNX inference; never mutates production configuration.
use super::nlu_build::{self, Artifact};
use crate::{
    plugins::nlu::{bert_feature_enabled, build_bert_checked, BertHit},
    runtime::Engines,
};
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TestRequest {
    build_id: String,
    text: String,
    expected_intent: Option<String>,
}

fn validate(req: &TestRequest) -> Result<(), &'static str> {
    if !nlu_build::valid_id(&req.build_id) {
        return Err("build_id must be a canonical managed build UUID");
    }
    if req.text.trim().is_empty() || req.text.len() > 4096 {
        return Err("text must contain 1..4096 bytes");
    }
    if req
        .expected_intent
        .as_ref()
        .is_some_and(|s| s.is_empty() || s.len() > 256)
    {
        return Err("expected_intent must contain 1..256 bytes");
    }
    Ok(())
}

fn prediction(artifact: &Artifact, hit: BertHit, expected: Option<&str>) -> anyhow::Result<Value> {
    if !hit.confidence.is_finite() || !(0.0..=1.0).contains(&hit.confidence) {
        anyhow::bail!("invalid model confidence");
    }
    if !artifact.labels.contains(&hit.label) {
        anyhow::bail!("model output is outside verified label list");
    }
    let intent = artifact.nlu.bert_map_label(&hit.label);
    let reason = if hit.label == "other" || intent == "other" {
        Some("other")
    } else if hit.confidence < artifact.nlu.bert_threshold {
        Some("below_threshold")
    } else if !artifact.nlu.intents.iter().any(|i| i.name == intent) {
        Some("unmapped_skill")
    } else {
        None
    };
    let accepted = reason.is_none();
    let expected_match = expected.map(|e| {
        if e == "other" {
            !accepted
        } else {
            accepted && intent == e
        }
    });
    Ok(
        json!({"state":"ok","build_id":artifact.build_id,"source_revision":artifact.source_revision,
        "model_sha256":artifact.model_sha256(),"raw_label":hit.label,"intent":intent,
        "confidence":hit.confidence,"threshold":artifact.nlu.bert_threshold,"accepted":accepted,
        "reject_reason":reason,"expected_intent":expected,"expected_match":expected_match,
        "tokenize_ms":hit.tokenize_ms,"infer_ms":hit.infer_ms,"executed":false,
        "measurement":"real_onnx_inference","note":"Intent classification only; no slot resolution or skill execution"}),
    )
}

pub(super) async fn test_model(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    use nlu_build::error;
    if let Some(r) = nlu_build::access(&engines, &headers, &query, true) {
        return r;
    }
    if body.len() > 8192 {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "Request exceeds 8192 bytes",
        );
    }
    let req: TestRequest = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return error(StatusCode::BAD_REQUEST, "invalid_input", e),
    };
    if let Err(e) = validate(&req) {
        return error(StatusCode::BAD_REQUEST, "invalid_input", e);
    }
    if !bert_feature_enabled() {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "nlu_feature_disabled",
            "Rebuild server with --features nlu for real candidate inference",
        );
    }
    static GATE: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let permit = match GATE
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(1)))
        .clone()
        .try_acquire_owned()
    {
        Ok(p) => p,
        Err(_) => {
            return error(
                StatusCode::TOO_MANY_REQUESTS,
                "diagnostic_busy",
                "Another candidate diagnostic is still running",
            )
        }
    };
    let store = engines.config_store.clone();
    let task = tokio::task::spawn_blocking(
        move || -> Result<Value, (StatusCode, &'static str, String)> {
            let _permit = permit; // Retained by blocking task even if HTTP timeout/client disconnects.
            let cfg = store.load().map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "config_read_failed",
                    e.to_string(),
                )
            })?;
            let data = nlu_build::data_dir(&cfg);
            let (artifact, dir) = nlu_build::load_candidate(&data, &req.build_id).map_err(|e| {
                let not_found = e.chain().any(|e| {
                    e.downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                });
                (
                    if not_found {
                        StatusCode::NOT_FOUND
                    } else {
                        StatusCode::CONFLICT
                    },
                    "candidate_unavailable",
                    e.to_string(),
                )
            })?;
            let isolated = artifact.config(&dir);
            let bert = build_bert_checked(&isolated).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "model_load_failed",
                    e.to_string(),
                )
            })?;
            let hit = bert.classify_timed(&req.text).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "model_inference_failed",
                    e.to_string(),
                )
            })?;
            prediction(&artifact, hit, req.expected_intent.as_deref()).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "model_output_invalid",
                    e.to_string(),
                )
            })
        },
    );
    match tokio::time::timeout(std::time::Duration::from_secs(45), task).await {
        Ok(Ok(Ok(value))) => Json(value).into_response(),
        Ok(Ok(Err((status, code, e)))) => error(status, code, e),
        Ok(Err(e)) => error(StatusCode::INTERNAL_SERVER_ERROR, "diagnostic_failed", e),
        Err(_) => error(
            StatusCode::GATEWAY_TIMEOUT,
            "diagnostic_timeout",
            "Candidate inference exceeded 45 seconds; capacity held until worker finishes",
        ),
    }
}

#[cfg(test)]
mod tests;
