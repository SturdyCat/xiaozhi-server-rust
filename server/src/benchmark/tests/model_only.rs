//! Intent diagnostics must never relabel keyword/template/heuristic output as model inference.
use super::*;

#[tokio::test]
async fn query_and_cancel_clock_plans_are_diagnostics_only_without_fabricated_speech() {
    for (name, text) in [
        ("reminder_list", "查询我的提醒"),
        ("reminder_cancel", "取消提醒喝水"),
    ] {
        let factory = Arc::new(FakeProviders {
            model_label: Some(name.into()),
            ..Default::default()
        });
        let (diagnostic, store) = service(&Config::default(), factory.clone());
        let out = diagnostic.run(request("pipeline.text", Some(text))).await;
        assert_eq!(out.result.state, "ok");
        let nlu = out.stages.iter().find(|s| s.stage == "nlu").unwrap();
        assert_eq!(nlu.details["skill"], name);
        assert_eq!(nlu.details["executed"], false);
        assert_eq!(nlu.measurement, "test_double_inference");
        assert_eq!(nlu.text.as_deref(), Some(""));
        assert!(nlu.details["action"]
            .as_str()
            .unwrap()
            .contains("{{steps.0.speech}}"));
        assert_eq!(
            out.stages.iter().find(|s| s.stage == "tts").unwrap().state,
            "skipped"
        );
        assert!(!factory
            .seen
            .lock()
            .unwrap()
            .iter()
            .any(|event| event["stage"] == "llm.input"));
        assert_eq!(store.writes.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test]
async fn native_disabled_model_has_no_keyword_fallback_or_inference_claim() {
    let cfg = Config::default();
    let (service, store) = service(&cfg, Arc::new(providers::NativeProviders));
    for action in ["nlu.classify", "command.match"] {
        let out = service.run(request(action, Some("退下"))).await;
        assert_eq!(out.result.details["matched"], false);
        assert_eq!(out.result.details["reject_reason"], "model_disabled");
        assert_eq!(out.result.measurement, "model_not_inferred");
        assert_ne!(out.result.readiness, "provider_verified");
        assert_eq!(out.result.details["executed"], false);
    }
    assert_eq!(store.writes.load(Ordering::Relaxed), 0);
    let caps = service.capabilities().await;
    for action in ["nlu.classify", "command.match"] {
        assert_eq!(
            caps["actions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["action_id"] == action)
                .unwrap()["available"],
            false
        );
    }
}

#[tokio::test]
async fn unavailable_model_does_not_silently_fall_back_to_keywords_or_llm() {
    let factory = Arc::new(FakeProviders {
        model_unavailable: true,
        ..Default::default()
    });
    let (service, _) = service(&Config::default(), factory.clone());
    let out = service.run(request("pipeline.text", Some("退下"))).await;
    assert_eq!(out.result.error_code, Some("provider_unavailable"));
    assert_eq!(
        out.stages.iter().find(|s| s.stage == "llm").unwrap().state,
        "skipped"
    );
    assert!(!factory
        .seen
        .lock()
        .unwrap()
        .iter()
        .any(|v| v["stage"] == "llm.input"));
}

#[tokio::test]
async fn needs_slots_is_clarification_and_not_llm_reclassification_or_task_execution() {
    let factory = Arc::new(FakeProviders {
        model_label: Some("reminder_countdown".into()),
        ..Default::default()
    });
    let (service, store) = service(&Config::default(), factory.clone());
    let out = service
        .run(request("pipeline.text", Some("请帮忙设一下")))
        .await;
    let nlu = out.stages.iter().find(|s| s.stage == "nlu").unwrap();
    assert_eq!(nlu.details["next"], "clarify");
    assert_eq!(nlu.details["reject_reason"], "needs_slots");
    assert_eq!(nlu.details["executed"], false);
    assert_eq!(nlu.measurement, "test_double_inference");
    assert_eq!(
        out.stages.iter().find(|s| s.stage == "llm").unwrap().state,
        "skipped"
    );
    assert_eq!(store.writes.load(Ordering::Relaxed), 0);
}
