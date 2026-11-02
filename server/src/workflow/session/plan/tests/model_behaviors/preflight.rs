use super::*;

#[tokio::test]
async fn deterministic_invalid_time_arguments_preflight_before_earlier_message_and_clarify_once() {
    for args in [
        json!({"text":"later task","after":"5s","timezone":"Europe/Paris"}),
        json!({"text":"later task","after":"not a duration"}),
        json!({"text":"later task","after":"0秒"}),
        json!({"text":"later task","interval":"367天"}),
        json!({"text":"later task","at":"whenever convenient"}),
        json!({"text":"later task","yearly":{"month":"2","day":"30","time":"08:00"}}),
    ] {
        let step = SkillStep {
            api: "tasks.create".into(),
            args: args.clone(),
        };
        let error = crate::skills::preflight_step_args(&step).unwrap_err();
        let clarification = error
            .downcast_ref::<crate::skills::SkillActionError>()
            .expect("deterministic time errors must offer safe speech")
            .speech
            .clone();
        let mut cfg = config(vec![binding("nlu.model", "nlu", 0, "stop")]);
        cfg.nlu = serde_json::from_value(json!({
            "enabled":true,"bert_enabled":true,"slots":[],"intents":[{
                "name":"configured_plan","templates":["执行计划"],
                "action":{"kind":"plan","reply":"must not claim success","plan":{"steps":[
                    {"api":"messages.send","args":{"text":"must never queue"}},
                    {"api":"tasks.create","args":args}
                ]}}
            }]
        }))
        .unwrap();
        let mut wire = Wire::default();
        let mut s = session(&mut wire, cfg, Arc::new(Memory::default()));
        let calls = install_model(&mut s, "configured_plan", 0.99);
        prime_audio(&s, &clarification);
        assert!(s.handle_skill_gate("执行计划").await);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(s.transport.audio_frames, 1);
        let sentences: Vec<String> = s
            .transport
            .sent
            .iter()
            .map(|raw| serde_json::from_str::<Value>(raw).unwrap())
            .filter(|v| v["type"] == "tts" && v["state"] == "sentence_start")
            .map(|v| v["text"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(sentences, vec![clarification]);
        assert_eq!(task_count(&s), 0);
        assert!(
            s.engines
                .skill_system
                .claim_delivery(&s.device_id, "observer", now_ms())
                .unwrap()
                .is_none(),
            "a deterministic error in a later step must prevent the earlier message commit"
        );
    }
}

#[tokio::test]
async fn pure_preflight_preserves_idempotent_replay_of_a_now_past_explicit_at() {
    let mut wire = Wire::default();
    let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
    let now = now_ms();
    let at = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now - 5_000)
        .unwrap()
        .to_rfc3339();
    let step = SkillStep {
        api: "tasks.create".into(),
        args: json!({"text":"existing task","at":at}),
    };
    let mut original = s.skill_context(0);
    original.now_ms = now - 10_000;
    s.engines
        .skill_system
        .execute_step(&original, &step)
        .unwrap();
    let inv = invocation(json!([{"api":step.api,"args":step.args}]), "");
    s.execute_invocation(&inv, "model").await.unwrap();
    assert_eq!(task_count(&s), 1);
    assert!(s.transport.sent.is_empty());
}
