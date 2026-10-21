//! Regression tests exercise the real service orchestrator with SQLite + injected providers.
use super::*;
use fakes::*;
use serde_json::json;
mod fakes;
mod http;
mod limits;
mod profiles;

#[tokio::test]
async fn capability_query_is_saved_read_only_and_never_proves_native_readiness() {
    let cfg = Config::default();
    let factory = Arc::new(FakeProviders::default());
    let (service, store) = service(&cfg, factory.clone());
    let body = service.capabilities().await;
    assert_eq!(body["source"], "saved");
    assert_eq!(
        body["revision"],
        crate::app::ws::config_revision(&store.db.load().unwrap())
    );
    assert_eq!(store.loads.load(Ordering::Relaxed), 1);
    assert_eq!(store.writes.load(Ordering::Relaxed), 0);
    assert!(factory.seen.lock().unwrap().is_empty());
    let actions = body["actions"].as_array().unwrap();
    assert!(actions
        .iter()
        .all(|a| a["readiness"] != "provider_verified"));
    for id in [
        "vad.sample",
        "asr.sample",
        "asr.recording",
        "nlu.classify",
        "llm.chat",
        "tts.synthesis",
        "tts.playback",
        "pipeline.text",
        "pipeline.audio",
        "input.profile",
        "wakeup.preview",
        "command.match",
        "soul.preview",
        "memory.recall",
        "output.profile",
        "memory.writeback.dry_run",
    ] {
        assert!(actions.iter().any(|a| a["action_id"] == id), "missing {id}");
    }
    for id in ["memory.recall", "memory.writeback.dry_run", "tts.playback"] {
        assert_eq!(
            actions.iter().find(|a| a["action_id"] == id).unwrap()["available"],
            false
        );
    }
    assert!(body["samples"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["id"] == samples::DEFAULT_SAMPLE));
}

#[test]
fn licensed_sample_is_real_nonzero_bounded_pcm_and_typed_input_is_strict() {
    let (audio, meta) = samples::audio(&request("asr.sample", None)).unwrap();
    assert_eq!(audio.len(), 64000);
    assert_eq!(meta["kind"], "server_sample");
    assert!(audio.iter().any(|v| v.abs() > 0.02));
    assert!(audio.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    let invalid = [
        json!({"format":"wav","sample_rate":16000,"channels":1,"data_base64":"AQI="}),
        json!({"format":"pcm_s16le","sample_rate":44100,"channels":1,"data_base64":"AQI="}),
        json!({"format":"pcm_s16le","sample_rate":16000,"channels":2,"data_base64":"AQI="}),
        json!({"format":"pcm_s16le","sample_rate":16000,"channels":1,"data_base64":"AQ=="}),
        json!({"format":"pcm_s16le","sample_rate":16000,"channels":1,"data_base64":""}),
    ];
    for audio in invalid {
        let req: TestRequest =
            serde_json::from_value(json!({"action_id":"asr.recording","audio":audio})).unwrap();
        assert!(samples::audio(&req).is_err());
    }
    for raw in [
        json!({"action_id":"asr.sample","path":"/etc/passwd"}),
        json!({"action_id":"asr.sample","url":"https://example.test/speech"}),
        json!({"action_id":"llm.chat","draft_patch":{}}),
        json!({"action_id":"llm.chat","device_id":"device"}),
    ] {
        assert!(serde_json::from_value::<TestRequest>(raw).is_err());
    }
}

#[tokio::test]
async fn all_standard_actions_reach_real_service_paths_with_fake_providers() {
    let cfg = Config::default();
    let factory = Arc::new(FakeProviders::default());
    let (service, store) = service(&cfg, factory.clone());
    for action in [
        "vad.sample",
        "asr.sample",
        "nlu.classify",
        "llm.chat",
        "tts.synthesis",
    ] {
        let result = service
            .run(request(
                action,
                Some("Tell me an unusual story about galaxies"),
            ))
            .await;
        assert_eq!(
            result.result.state,
            "ok",
            "{action}: {}",
            serde_json::to_string(&result).unwrap()
        );
        assert_eq!(
            result.result.readiness, "configured",
            "fake must not claim native readiness"
        );
        assert_eq!(result.source, "saved");
        assert!(result.revision.is_some());
        assert!(!result.request_id.is_empty());
        assert_eq!(result.stages.len(), 1);
    }
    assert_eq!(store.loads.load(Ordering::Relaxed), 5);
    assert_eq!(store.writes.load(Ordering::Relaxed), 0);
    let seen = factory.seen.lock().unwrap();
    let input = seen.iter().find(|v| v["stage"] == "asr.input").unwrap();
    assert_eq!(input["samples"], 64000);
    assert!(input["energy"].as_f64().unwrap() > 0.0);
}

#[tokio::test]
async fn nlu_uses_actual_skill_registry_nomination_with_no_device_execution() {
    let mut cfg = Config::default();
    cfg.command.enabled = true;
    let (service, store) = service(&cfg, Arc::new(FakeProviders::default()));
    let out = service.run(request("nlu.classify", Some("退下。"))).await;
    assert_eq!(out.result.state, "ok");
    assert_eq!(out.result.details["matched"], true);
    assert_eq!(out.result.details["recognition_stage"], "rules");
    assert_eq!(out.result.details["skill"], "end_session");
    assert_eq!(out.result.details["executed"], false);
    assert_eq!(out.result.details["dry_run"], true);
    assert_eq!(store.writes.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn nlu_cascade_reuses_actual_light_model_not_a_parallel_fake_classifier() {
    let mut cfg = Config::default();
    cfg.nlu=serde_json::from_value(json!({"enabled":true,"intents":[{"name":"control_light","templates":["把{room}的灯{action}"],"action":{"kind":"mcp:light","reply":"好的"}}],"slots":[{"name":"room","values":["客厅","卧室"]},{"name":"action","values":["打开","关闭"]}]})).unwrap();
    let (service, _) = service(&cfg, Arc::new(FakeProviders::default()));
    let out = service
        .run(request("nlu.classify", Some("帮我把大客厅的灯打开")))
        .await;
    assert_eq!(out.result.state, "ok");
    assert_eq!(out.result.details["matched"], true);
    assert_eq!(out.result.details["recognition_stage"], "model");
    assert_eq!(out.result.details["slots"]["room"], "客厅");
    assert_eq!(out.result.details["executed"], false);
}

#[tokio::test]
async fn text_pipeline_explicitly_skips_audio_and_keeps_one_snapshot_during_save() {
    let mut cfg = Config::default();
    cfg.llm.model = "snapshot-model".into();
    cfg.soul.enabled = true;
    let store = CountingStore::new(&cfg);
    let factory = Arc::new(FakeProviders {
        mutate_on_nlu: Some(store.clone()),
        ..Default::default()
    });
    let gates = crate::app::inference::InferenceGates::new(&cfg);
    let service = TestService {
        store: store.clone(),
        asr_gate: gates.asr(),
        tts_gate: gates.tts(),
        factory: factory.clone(),
        limit: Arc::new(tokio::sync::Semaphore::new(2)),
        read_limit: Arc::new(tokio::sync::Semaphore::new(4)),
        timeout: Duration::from_secs(45),
    };
    let out = service
        .run(request(
            "pipeline.text",
            Some("Tell me an unusual story about galaxies"),
        ))
        .await;
    assert_eq!(out.result.state, "ok");
    assert_eq!(out.context_mode, "pipeline_preview");
    assert_eq!(
        out.stages.iter().find(|s| s.stage == "vad").unwrap().state,
        "skipped"
    );
    assert_eq!(
        out.stages.iter().find(|s| s.stage == "asr").unwrap().state,
        "skipped"
    );
    assert_eq!(
        out.revision.as_deref(),
        Some(crate::app::ws::config_revision(&cfg).as_str())
    );
    assert_eq!(store.loads.load(Ordering::Relaxed), 1);
    assert_eq!(store.writes.load(Ordering::Relaxed), 0);
    assert_eq!(store.db.load().unwrap().llm.model, "new-after-snapshot");
    let seen = factory.seen.lock().unwrap();
    for event in seen.iter().filter(|e| e.get("model").is_some()) {
        assert_eq!(event["model"], "snapshot-model");
    }
    let llm = seen.iter().find(|v| v["stage"] == "llm.input").unwrap();
    assert_eq!(llm["history"], 0);
    assert!(llm["memory"].is_null());
    assert_eq!(llm["tools"], false);
    assert_eq!(
        llm["instructions"],
        crate::plugins::soul::preview::build(&cfg, "<node-test>")
            .unwrap()
            .instructions
    );
    assert_eq!(out.result.details["context"]["production_fullflow"], false);
    let tts = out.stages.iter().find(|s| s.stage == "tts").unwrap();
    assert!(tts.first_pcm_ms.is_some());
    assert_eq!(tts.details["played"], false);
    assert_eq!(tts.details["socket_written"], false);
}

#[tokio::test]
async fn audio_pipeline_invokes_each_stage_and_records_provider_only_timings() {
    let (service, _) = service(&Config::default(), Arc::new(FakeProviders::default()));
    let out = service.run(request("pipeline.audio", None)).await;
    assert_eq!(out.result.state, "ok");
    for stage in ["vad", "asr", "nlu", "llm", "tts"] {
        assert_eq!(
            out.stages.iter().find(|s| s.stage == stage).unwrap().state,
            "ok",
            "{stage}"
        );
    }
    assert!(out
        .stages
        .iter()
        .find(|s| s.stage == "llm")
        .unwrap()
        .first_token_ms
        .is_some());
}

#[tokio::test]
async fn absent_speech_does_not_send_silence_to_asr_or_llm() {
    let factory = Arc::new(FakeProviders {
        no_speech: true,
        ..Default::default()
    });
    let (service, _) = service(&Config::default(), factory.clone());
    let out = service.run(request("pipeline.audio", None)).await;
    assert_eq!(out.result.error_code, Some("no_speech"));
    assert_eq!(
        out.stages.iter().find(|s| s.stage == "asr").unwrap().state,
        "skipped"
    );
    assert!(factory.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn revision_conflict_and_snapshot_failure_never_build_or_fall_back_to_live_config() {
    let factory = Arc::new(FakeProviders::default());
    let (service, store) = service(&Config::default(), factory.clone());
    let mut req = request("llm.chat", Some("Hello"));
    req.expected_revision = Some("stale".into());
    assert_eq!(
        service.run(req).await.result.error_code,
        Some("revision_conflict")
    );
    store.fail.store(true, Ordering::Relaxed);
    let out = service.run(request("llm.chat", Some("Hello"))).await;
    assert_eq!(out.result.error_code, Some("snapshot_unavailable"));
    assert!(factory.seen.lock().unwrap().is_empty());
    assert!(!serde_json::to_string(&out)
        .unwrap()
        .contains("secret database error"));
}

#[tokio::test]
async fn legacy_overrides_are_transient_and_isolated_context_is_not_fullflow() {
    let mut cfg = Config::default();
    cfg.llm.api_key = "saved-key".into();
    cfg.llm.system_prompt = "literal {{old brace}}".into();
    cfg.soul.enabled = true;
    let factory = Arc::new(FakeProviders::default());
    let (service, store) = service(&cfg, factory.clone());
    let mut req = request("llm.chat", Some("Hello"));
    req.api_key = Some("request-key".into());
    let out = service.run(req).await;
    assert_eq!(out.source, "saved+request");
    assert_eq!(out.context_mode, "isolated");
    let seen = factory.seen.lock().unwrap();
    assert_eq!(
        seen.iter().find(|v| v["stage"] == "llm").unwrap()["api_key"],
        "request-key"
    );
    assert_eq!(
        seen.iter().find(|v| v["stage"] == "llm.input").unwrap()["instructions"],
        cfg.llm.system_prompt
    );
    assert_eq!(store.db.load().unwrap().llm.api_key, "saved-key");
    assert_eq!(store.writes.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn provider_errors_and_unavailable_features_do_not_fake_success_or_leak_provider_secrets() {
    let (service, _) = service(
        &Config::default(),
        Arc::new(FakeProviders {
            fail: true,
            ..Default::default()
        }),
    );
    let out = service.run(request("tts.synthesis", Some("Hello"))).await;
    assert_eq!(out.result.state, "unavailable");
    assert_eq!(out.result.readiness, "unavailable");
    let wire = serde_json::to_string(&out).unwrap();
    assert!(!wire.contains("sk-private"));
    assert!(!wire.contains("password"));
    for action in ["memory.recall", "memory.writeback.dry_run", "tts.playback"] {
        assert_eq!(
            service
                .run(request(action, Some("Hello")))
                .await
                .result
                .state,
            "unavailable"
        );
    }
}

#[tokio::test]
async fn disabled_llm_pipeline_echoes_without_cloud_or_soul_calls() {
    let mut cfg = Config::default();
    cfg.llm.enabled = false;
    let factory = Arc::new(FakeProviders::default());
    let (service, _) = service(&cfg, factory.clone());
    let out = service
        .run(request("pipeline.text", Some("Echo this unusual phrase")))
        .await;
    assert_eq!(out.result.state, "ok");
    assert_eq!(out.result.text.as_deref(), Some("Echo this unusual phrase"));
    assert_eq!(
        out.stages.iter().find(|s| s.stage == "llm").unwrap().state,
        "skipped"
    );
    assert!(!factory
        .seen
        .lock()
        .unwrap()
        .iter()
        .any(|e| e["stage"] == "llm" || e["stage"] == "llm.input"));
}

#[tokio::test]
async fn empty_asr_and_cancelled_llm_are_not_success() {
    let (service, _) = service(
        &Config::default(),
        Arc::new(FakeProviders {
            empty_asr: true,
            cancelled_llm: true,
            ..Default::default()
        }),
    );
    assert_eq!(
        service
            .run(request("asr.sample", None))
            .await
            .result
            .error_code,
        Some("no_transcript")
    );
    assert_eq!(
        service
            .run(request("llm.chat", Some("hello")))
            .await
            .result
            .error_code,
        Some("cancelled")
    );
}

#[tokio::test]
async fn input_validation_precedes_snapshot_and_provider_builds() {
    let factory = Arc::new(FakeProviders::default());
    let (service, store) = service(&Config::default(), factory.clone());
    let mut req = request("llm.chat", Some("Hello"));
    req.text = Some("X".repeat(4097));
    assert_eq!(
        service.run(req).await.result.error_code,
        Some("invalid_input")
    );
    assert_eq!(store.loads.load(Ordering::Relaxed), 0);
    let mut req = request("asr.sample", None);
    req.sample_id = Some("/etc/passwd".into());
    assert_eq!(
        service.run(req).await.result.error_code,
        Some("invalid_input")
    );
    assert!(factory.seen.lock().unwrap().is_empty());
    assert_eq!(
        service
            .run(request("asr.recording", None))
            .await
            .result
            .error_code,
        Some("invalid_input")
    );
}

#[tokio::test]
async fn timeout_retains_admission_and_production_inference_gate_until_worker_actually_exits() {
    let blocker = Arc::new(Blocker::default());
    let release = ReleaseOnDrop(blocker.clone());
    let mut cfg = Config::default();
    cfg.tts.max_concurrent = 1;
    let (mut service, _) = service(
        &cfg,
        Arc::new(FakeProviders {
            blocker: Some(blocker.clone()),
            ..Default::default()
        }),
    );
    let s = Arc::get_mut(&mut service).unwrap();
    s.timeout = Duration::from_millis(50);
    s.limit = Arc::new(tokio::sync::Semaphore::new(1));
    let worker = {
        let service = service.clone();
        tokio::spawn(async move { service.run(request("tts.synthesis", Some("hello"))).await })
    };
    tokio::time::timeout(Duration::from_secs(1), blocker.entered.notified())
        .await
        .unwrap();
    assert_eq!(
        service
            .run(request("nlu.classify", Some("hello")))
            .await
            .result
            .error_code,
        Some("busy")
    );
    let result = worker.await.unwrap();
    assert_eq!(result.result.error_code, Some("timeout"));
    assert_eq!(service.limit.available_permits(), 0);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), service.tts_gate.acquire())
            .await
            .is_err()
    );
    drop(release);
    let admission = tokio::time::timeout(
        Duration::from_secs(1),
        service.limit.clone().acquire_owned(),
    )
    .await
    .unwrap()
    .unwrap();
    let gate = tokio::time::timeout(Duration::from_secs(1), service.tts_gate.acquire())
        .await
        .unwrap();
    assert!(gate.is_some());
    drop(admission);
}

#[cfg(not(feature = "sherpa"))]
#[tokio::test]
async fn default_build_never_runs_audio_compilation_stubs_as_real_tests() {
    let (service, _) = service(&Config::default(), Arc::new(providers::NativeProviders));
    for action in ["vad.sample", "asr.sample", "tts.synthesis"] {
        let out = service.run(request(action, Some("hello"))).await;
        assert_eq!(out.result.state, "unavailable");
        assert_eq!(out.result.readiness, "unavailable");
    }
}
