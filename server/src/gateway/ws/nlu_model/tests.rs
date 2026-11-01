//! Pure decision + authenticated HTTP-handler contracts, never a fake ONNX implementation.
use super::*;
use crate::{
    config::{
        store::{sqlite::SqliteConfigStore, ConfigStore},
        Config,
    },
    plugins::nlu::NluConfig,
};

fn artifact() -> Artifact {
    let nlu = NluConfig::default().with_builtins();
    Artifact {
        build_id: uuid::Uuid::new_v4().to_string(),
        source_revision: "saved".into(),
        source_input_sha256: "input-hash".into(),
        trainer_image: "dependency-image".into(),
        trainer_image_id: "image-id".into(),
        trainer_script_sha256: "script-hash".into(),
        labels: nlu
            .intents
            .iter()
            .map(|i| i.name.clone())
            .chain(std::iter::once("other".into()))
            .collect(),
        files: vec![],
        nlu,
    }
}
fn hit(label: &str, confidence: f32) -> BertHit {
    BertHit {
        label: label.into(),
        confidence,
        tokenize_ms: 0,
        infer_ms: 7,
    }
}
fn dev_engines() -> Arc<Engines> {
    let mut cfg = Config::default();
    cfg.server.expected_token = "test-admin-token".into();
    cfg.memory.db_path = std::env::temp_dir()
        .join(format!("xz-nlu-dev-{}", uuid::Uuid::new_v4()))
        .join("data.db")
        .display()
        .to_string();
    let store = SqliteConfigStore::open(":memory:").unwrap();
    store.persist(&cfg).unwrap();
    Engines::new_dev(&cfg, store).unwrap()
}

#[test]
fn other_is_refusal_even_when_high_confidence_and_zero_threshold() {
    let mut a = artifact();
    a.nlu.bert_threshold = 0.;
    let v = prediction(&a, hit("other", 0.99), Some("other")).unwrap();
    assert_eq!(v["accepted"], false);
    assert_eq!(v["reject_reason"], "other");
    assert_eq!(v["expected_match"], true);
    assert_eq!(v["executed"], false);
}
#[test]
fn below_threshold_is_refusal_and_high_score_is_only_classification() {
    let a = artifact();
    let label = "reminder_countdown";
    let low = prediction(&a, hit(label, 0.2), Some(label)).unwrap();
    assert_eq!(low["accepted"], false);
    assert_eq!(low["reject_reason"], "below_threshold");
    assert_eq!(low["expected_match"], false);
    let high = prediction(&a, hit(label, 0.99), Some(label)).unwrap();
    assert_eq!(high["accepted"], true);
    assert_eq!(high["executed"], false);
    assert_eq!(high["measurement"], "real_onnx_inference");
}
#[test]
fn diagnostic_does_not_enable_or_rewrite_global_nlu() {
    let mut a = artifact();
    a.nlu.bert_enabled = false;
    a.nlu.bert_threshold = 0.83;
    let before = serde_json::to_value(&a.nlu).unwrap();
    let isolated = a.config(std::path::Path::new("/managed/version"));
    assert!(isolated.bert_enabled);
    assert_eq!(isolated.bert_threshold, 0.83);
    let _ = prediction(&a, hit("reminder_repeat", 0.91), None).unwrap();
    assert_eq!(serde_json::to_value(&a.nlu).unwrap(), before);
}
#[test]
fn invalid_model_output_and_arbitrary_input_paths_are_rejected() {
    let a = artifact();
    assert!(prediction(&a, hit("injected", 0.99), None).is_err());
    assert!(prediction(&a, hit("other", f32::NAN), None).is_err());
    assert!(serde_json::from_value::<TestRequest>(
        json!({"build_id":a.build_id,"text":"test","bert_model":"/tmp/evil"})
    )
    .is_err());
    assert!(validate(&TestRequest {
        build_id: "../../external".into(),
        text: "test".into(),
        expected_intent: None
    })
    .is_err());
    assert!(validate(&TestRequest {
        build_id: a.build_id,
        text: " ".into(),
        expected_intent: None
    })
    .is_err());
}
#[tokio::test]
async fn candidate_http_handler_authenticates_before_dev_guard() {
    let engines = dev_engines();
    let denied = test_model(
        State(engines.clone()),
        HeaderMap::new(),
        Query(HashMap::new()),
        Bytes::new(),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer test-admin-token".parse().unwrap());
    let dev = test_model(
        State(engines.clone()),
        headers,
        Query(HashMap::new()),
        Bytes::new(),
    )
    .await;
    assert_eq!(dev.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = axum::body::to_bytes(dev.into_body(), 8192).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap()["error_code"],
        "dev_inference_disabled"
    );
    assert!(!engines.config_store.load().unwrap().nlu.bert_enabled);
}
