use super::*;
use crate::media::media::AudioCodec;

fn create_due(s: &Session<'_, Wire>, index: usize, text: &str) -> String {
    let mut ctx = s.skill_context(index);
    ctx.now_ms = now_ms() - 10_000;
    ctx.cancelled = Arc::new(AtomicBool::new(false)); // Authenticated management, not the cancelled user turn.
    let result = s
        .engines
        .skill_system
        .execute_step(
            &ctx,
            &SkillStep {
                api: "tasks.create".into(),
                args: json!({"after":"1s","text":text}),
            },
        )
        .unwrap();
    assert_eq!(s.engines.skill_system.tick_due(now_ms()).unwrap().materialized, 1);
    result["task"]["id"].as_str().unwrap().into()
}
fn sentence_texts(s: &Session<'_, Wire>) -> Vec<String> {
    s.transport
        .sent
        .iter()
        .map(|raw| serde_json::from_str::<Value>(raw).unwrap())
        .filter(|v| v["type"] == "tts" && v["state"] == "sentence_start")
        .map(|v| v["text"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn management_cancel_during_a_alarm_does_not_poison_idle_delivery_of_b() {
    let mut wire = Wire::default();
    let mut cfg = config(vec![]);
    cfg.audio.downlink_lead_ms = 0;
    let mut s = session(&mut wire, cfg.clone(), Arc::new(Memory::default()));
    s.params = SessionParams::configured_defaults(&cfg, AudioCodec::PcmS16Le);
    let model_calls = install_model(&mut s, "end_session", 0.99);
    let a = create_due(&s, 0, "A must not speak");
    let system = s.engines.skill_system.clone();
    let cancel_a = a.clone();
    let ctx = s.skill_context(99);
    s.transport.on_first_audio = Some(Box::new(move || {
        system
            .execute_step(
                &ctx,
                &SkillStep {
                    api: "tasks.cancel".into(),
                    args: json!({"task_id":cancel_a}),
                },
            )
            .unwrap();
    }));
    prime_audio(&s, "A must not speak");
    s.deliver_pending_message().await;
    assert!(
        !s.abort.load(Ordering::Relaxed),
        "SQL/task cancellation is delivery-local, not a user abort"
    );
    assert!(!sentence_texts(&s)
        .iter()
        .any(|text| text == "A must not speak"));
    let a = s
        .engines
        .skill_system
        .get_task(&a, Some(&s.device_id))
        .unwrap()
        .unwrap();
    assert_eq!(a["delivery_state"], "cancelled");
    assert_eq!(s.transport.stop_attempts, 1);

    let b = create_due(&s, 1, "B still delivers");
    prime_audio(&s, "B still delivers");
    // The next idle tick, without a new user utterance or clearing any abort flag.
    s.deliver_pending_message().await;
    let b = s
        .engines
        .skill_system
        .get_task(&b, Some(&s.device_id))
        .unwrap()
        .unwrap();
    assert_eq!(b["delivery_state"], "submitted");
    assert_eq!(
        sentence_texts(&s),
        vec!["提醒铃声", "提醒铃声", "B still delivers"]
    );
    assert_eq!(s.transport.stop_attempts, 2);
    assert_eq!(model_calls.load(Ordering::Relaxed), 0);
    assert!(s.history.is_empty());
}

#[tokio::test]
async fn local_cancel_racing_native_cancel_is_not_mislabeled_user_abort() {
    let token = Arc::new(AtomicBool::new(false));
    let mut wire = Wire {
        local_cancel_during_native_write: Some(token.clone()),
        ..Default::default()
    };
    let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
    prime_audio(&s, "must not speak");
    assert!(s
        .speak_delivery("must not speak", true, token.clone())
        .await
        .is_err());
    assert!(token.load(Ordering::Relaxed));
    assert!(!s.abort.load(Ordering::Relaxed));
    assert_eq!(s.transport.stop_attempts, 1);
    assert_eq!(s.transport.audio_frames, 0);
}

#[tokio::test]
async fn write_error_attempts_stop_and_stop_error_never_masks_original_audio_error() {
    for fail_stop in [false, true] {
        let mut wire = Wire {
            fail_audio_after: Some(0),
            fail_stop,
            ..Default::default()
        };
        let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
        prime_audio(&s, "must not speak");
        let error = s
            .speak_delivery("must not speak", true, Arc::new(AtomicBool::new(false)))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("injected audio write failure"));
        assert_eq!(s.transport.stop_attempts, 1);
        assert!(!s.abort.load(Ordering::Relaxed));
        if !fail_stop {
            let states: Vec<String> = s
                .transport
                .sent
                .iter()
                .map(|raw| serde_json::from_str::<Value>(raw).unwrap())
                .filter(|v| v["type"] == "tts" && (v["state"] == "start" || v["state"] == "stop"))
                .map(|v| v["state"].as_str().unwrap().into())
                .collect();
            assert_eq!(states, vec!["start", "stop"]);
        }
    }
}

#[tokio::test]
async fn genuine_user_abort_and_native_cancel_stay_sticky_for_other_pending_tasks() {
    for native in [false, true] {
        let mut wire = Wire::default();
        if native {
            wire.cancel_audio_after = Some(0);
        } else {
            wire.abort_after_audio = Some(1);
        }
        let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
        create_due(&s, 0, "A");
        prime_audio(&s, "A");
        s.deliver_pending_message().await;
        assert!(s.abort.load(Ordering::Relaxed));
        let b = create_due(&s, 1, "B pending after real abort");
        prime_audio(&s, "B pending after real abort");
        let before = s.transport.sent.len();
        s.deliver_pending_message().await;
        assert_eq!(s.transport.sent.len(), before);
        assert_eq!(
            s.engines
                .skill_system
                .get_task(&b, Some(&s.device_id))
                .unwrap()
                .unwrap()["delivery_state"],
            "pending"
        );
    }
}
