//! Loopback HTTP compatibility tests use the production router and service, not a parallel stub API.
use super::*;
use axum::{
    http::{HeaderMap, StatusCode},
    routing::get,
    Json, Router,
};

#[tokio::test]
async fn every_test_endpoint_uses_the_same_admin_auth_boundary() {
    let factory = Arc::new(FakeProviders::default());
    let (service, store) = service(&Config::default(), factory.clone());
    let (base, handle) = http(service, "admin-token").await;
    let _guard = server_guard(handle);
    let client = reqwest::Client::new();
    for path in [
        "llm",
        "tts",
        "vad",
        "asr",
        "nlu",
        "pipeline",
        "run",
        "llm/models",
    ] {
        let response = client
            .post(format!("{base}/api/test/{path}"))
            .json(&json!({"text":"hello","expectedToken":"admin-token"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error_code"], "unauthorized");
    }
    let caps = client
        .get(format!("{base}/api/test/capabilities"))
        .send()
        .await
        .unwrap();
    assert_eq!(caps.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(store.loads.load(Ordering::Relaxed), 0);
    assert!(factory.seen.lock().unwrap().is_empty());
    let caps = client
        .get(format!("{base}/api/test/capabilities"))
        .bearer_auth("admin-token")
        .send()
        .await
        .unwrap();
    assert!(caps.status().is_success());
    // Existing auth_ok browser query fallback retained; never accept a token from JSON.
    let caps = client
        .get(format!("{base}/api/test/capabilities?token=admin-token"))
        .send()
        .await
        .unwrap();
    assert!(caps.status().is_success());
}

#[tokio::test]
async fn legacy_urls_and_original_result_fields_remain_compatible() {
    let (service, _) = service(&Config::default(), Arc::new(FakeProviders::default()));
    let (base, handle) = http(service, "").await;
    let _guard = server_guard(handle);
    let client = reqwest::Client::new();
    for (path, body, action) in [
        (
            "llm",
            json!({"text":"Hello","api_key":"override"}),
            "llm.chat",
        ),
        (
            "tts",
            json!({"text":"Hello","lang":"en","vcn":"voice","speed":1.1}),
            "tts.synthesis",
        ),
        ("vad", json!({}), "vad.sample"),
        ("asr", json!({}), "asr.sample"),
        ("nlu", json!({"text":"Hello"}), "nlu.classify"),
        ("pipeline", json!({"text":"Hello"}), "pipeline.text"),
    ] {
        let response = client
            .post(format!("{base}/api/test/{path}"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success(), "{path}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["state"], "ok", "{path}: {body}");
        assert_eq!(body["action_id"], action);
        for field in [
            "state",
            "elapsed_ms",
            "first_packet_ms",
            "engine",
            "text",
            "error",
            "request_id",
            "source",
            "revision",
            "error_code",
            "readiness",
            "stages",
        ] {
            assert!(body.get(field).is_some(), "{path} missing {field}");
        }
        assert!(body["revision"].is_string());
    }
}

#[tokio::test]
async fn run_endpoint_rejects_stale_revisions_unsupported_overlays_and_audio_paths() {
    let factory = Arc::new(FakeProviders::default());
    let (service, _) = service(&Config::default(), factory.clone());
    let (base, handle) = http(service, "").await;
    let _guard = server_guard(handle);
    let client = reqwest::Client::new();
    let stale = client
        .post(format!("{base}/api/test/run"))
        .json(&json!({"action_id":"llm.chat","text":"hello","expected_revision":"stale"}))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        stale.json::<Value>().await.unwrap()["error_code"],
        "revision_conflict"
    );
    for body in [
        json!({"action_id":"llm.chat","text":"hello","draft_patch":{"llm":{"api_key":"secret"}}}),
        json!({"action_id":"llm.chat","text":"hello","device_id":"device"}),
        json!({"action_id":"asr.sample","path":"/etc/passwd"}),
        json!({"action_id":"asr.sample","url":"https://bad.test/speech.wav"}),
        json!({"action_id":"asr.sample","sample_id":"../../etc/passwd"}),
    ] {
        let response = client
            .post(format!("{base}/api/test/run"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.json::<Value>().await.unwrap()["error_code"],
            "invalid_input"
        );
    }
    assert!(factory.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn typed_recording_input_and_http_payload_limit_are_enforced() {
    let factory = Arc::new(FakeProviders::default());
    let (service, _) = service(&Config::default(), factory.clone());
    let (base, handle) = http(service, "").await;
    let _guard = server_guard(handle);
    let client = reqwest::Client::new();
    let out=client.post(format!("{base}/api/test/run")).json(&json!({"action_id":"asr.recording","audio":{"format":"pcm_s16le","sample_rate":16000,"channels":1,"data_base64":"AQIDBA=="}})).send().await.unwrap();
    assert_eq!(out.status(), StatusCode::OK);
    let out: Value = out.json().await.unwrap();
    assert_eq!(out["state"], "ok");
    assert_eq!(out["details"]["input"]["kind"], "request_pcm");
    let count = factory.seen.lock().unwrap().len();
    let out = client
        .post(format!("{base}/api/test/run"))
        .json(&json!({"action_id":"llm.chat","text":"X".repeat(MAX_BODY_BYTES+1)}))
        .send()
        .await
        .unwrap();
    assert_eq!(out.status(), StatusCode::BAD_REQUEST);
    assert_eq!(factory.seen.lock().unwrap().len(), count);
}

#[tokio::test]
async fn model_catalogue_proxy_retains_fields_sorts_models_and_never_persists_overrides() {
    let upstream = Router::new().route(
        "/v1/models",
        get(|headers: HeaderMap| async move {
            assert_eq!(headers.get("authorization").unwrap(), "Bearer request-key");
            Json(json!({"data":[{"id":"zeta"},{"id":"alpha"},{"id":"zeta"}]}))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_base = format!("http://{}", listener.local_addr().unwrap());
    let upstream_handle =
        tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let _upstream_guard = server_guard(upstream_handle);
    let mut cfg = Config::default();
    cfg.llm.api_key = "saved-key".into();
    let (service, store) = service(&cfg, Arc::new(FakeProviders::default()));
    let (base, handle) = http(service, "").await;
    let _guard = server_guard(handle);
    let result = reqwest::Client::new()
        .post(format!("{base}/api/test/llm/models"))
        .json(&json!({"api_base":format!("{upstream_base}/v1/responses"),"api_key":"request-key"}))
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    let result: Value = result.json().await.unwrap();
    assert_eq!(result["state"], "ok");
    assert_eq!(result["models"], json!(["alpha", "zeta"]));
    assert_eq!(result["source"], "saved+request");
    for field in [
        "models",
        "state",
        "elapsed_ms",
        "error",
        "request_id",
        "source",
        "revision",
        "error_code",
    ] {
        assert!(result.get(field).is_some());
    }
    assert_eq!(store.loads.load(Ordering::Relaxed), 1);
    assert_eq!(store.writes.load(Ordering::Relaxed), 0);
    assert_eq!(store.db.load().unwrap().llm.api_key, "saved-key");
}
