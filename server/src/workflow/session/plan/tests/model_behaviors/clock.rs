use super::*;
use crate::media::media::{AudioCodec, AudioFormat};

fn texts(s: &Session<'_, Wire>) -> Vec<String> {
    s.transport
        .sent
        .iter()
        .map(|raw| serde_json::from_str::<Value>(raw).unwrap())
        .filter(|v| v["type"] == "tts" && v["state"] == "sentence_start")
        .map(|v| v["text"].as_str().unwrap().to_owned())
        .collect()
}
fn due_task(s: &Session<'_, Wire>, text: &str) {
    let mut ctx = s.skill_context(0);
    ctx.now_ms = now_ms() - 10_000;
    s.engines
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
}
fn retry_available(s: &Session<'_, Wire>) -> bool {
    s.engines
        .skill_system
        .claim_delivery(&s.device_id, "retry-observer", now_ms() + 2000)
        .unwrap()
        .is_some()
}

#[tokio::test]
async fn task_alarm_then_text_share_one_tts_round_and_messages_have_no_alarm() {
    for task in [true, false] {
        let mut wire = Wire::default();
        let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
        if task {
            due_task(&s, "文案");
        } else {
            s.engines
                .skill_system
                .execute_step(
                    &s.skill_context(0),
                    &SkillStep {
                        api: "messages.send".into(),
                        args: json!({"text":"文案"}),
                    },
                )
                .unwrap();
        }
        prime_audio(&s, "文案");
        s.deliver_pending_message().await;
        assert_eq!(
            texts(&s),
            if task {
                vec!["提醒铃声", "文案"]
            } else {
                vec!["文案"]
            }
        );
        let states: Vec<Value> = s
            .transport
            .sent
            .iter()
            .map(|raw| serde_json::from_str(raw).unwrap())
            .collect();
        for state in ["start", "stop"] {
            assert_eq!(
                states
                    .iter()
                    .filter(|v| v["type"] == "tts" && v["state"] == state)
                    .count(),
                1
            );
        }
        assert_eq!(s.transport.audio_frames > 1, task);
        assert!(!retry_available(&s));
    }
}

#[tokio::test]
async fn abort_native_cancel_and_audio_failure_during_alarm_never_submit_or_start_speech() {
    for mode in 0..3 {
        let mut wire = Wire::default();
        if mode == 0 {
            wire.abort_after_audio = Some(1);
        }
        if mode == 1 {
            wire.cancel_audio_after = Some(0);
        }
        if mode == 2 {
            wire.fail_audio_after = Some(0);
        }
        let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
        due_task(&s, "must not be spoken");
        prime_audio(&s, "must not be spoken");
        s.deliver_pending_message().await;
        assert!(!texts(&s).iter().any(|text| text == "must not be spoken"));
        assert!(retry_available(&s), "failed delivery must remain retryable");
        assert_eq!(
            s.abort.load(Ordering::Relaxed),
            mode != 2,
            "only genuine user/native cancellation is sticky, not a failed delivery write"
        );
        let states: Vec<String> = s
            .transport
            .sent
            .iter()
            .map(|raw| serde_json::from_str::<Value>(raw).unwrap())
            .filter(|v| v["type"] == "tts" && (v["state"] == "start" || v["state"] == "stop"))
            .map(|v| v["state"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            states,
            vec!["start", "stop"],
            "even an audio write Err must finish TTS state"
        );
    }
}

#[tokio::test]
async fn tts_failure_after_audible_alarm_does_not_become_false_submitted() {
    let mut wire = Wire::default();
    let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
    due_task(&s, "uncached TTS is unavailable in dev engines");
    s.deliver_pending_message().await;
    assert!(
        s.transport.audio_frames > 0,
        "the local alarm was really encoded and submitted"
    );
    assert!(
        retry_available(&s),
        "alarm frames cannot mask a failed speech source"
    );
}

#[tokio::test]
async fn alarm_respects_echomuse_pcm_geometry_not_esp_opus_packets() {
    let mut wire = Wire::default();
    let cfg = config(vec![]);
    let mut s = session(&mut wire, cfg.clone(), Arc::new(Memory::default()));
    s.params = SessionParams::configured_defaults(&cfg, AudioCodec::PcmS16Le);
    prime_audio(&s, "原生音频");
    s.speak_delivery("原生音频", true, Arc::new(AtomicBool::new(false)))
        .await
        .unwrap();
    assert!(s.transport.audio_frames > 1);
    assert!(s
        .transport
        .audio_formats
        .iter()
        .all(|format| *format == AudioFormat::echo_speaker()));
    assert_eq!(texts(&s), vec!["提醒铃声", "原生音频"]);
}

#[tokio::test]
async fn native_cancelled_farewell_cannot_start_a_following_mcp_action() {
    let mut wire = Wire {
        cancel_audio_after: Some(0),
        ..Default::default()
    };
    let mut cfg = config(vec![binding("nlu.model", "nlu", 0, "stop")]);
    cfg.command.reply = "bye".into();
    let mut s = session(&mut wire, cfg, Arc::new(Memory::default()));
    s.mcp_enabled = true;
    prime_audio(&s, "bye");
    assert!(s.handle_skill_gate("退下").await);
    assert!(s.abort.load(Ordering::Relaxed));
    assert!(!s.close_requested);
    assert!(!s
        .transport
        .sent
        .iter()
        .any(|raw| raw.contains("tools/call")));
}

#[tokio::test]
async fn real_task_list_service_speech_flows_only_into_deferred_reply() {
    let mut wire = Wire::default();
    let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
    let expected = s
        .engines
        .skill_system
        .execute_step(
            &s.skill_context(4),
            &SkillStep {
                api: "tasks.list".into(),
                args: json!({}),
            },
        )
        .unwrap()["speech"]
        .as_str()
        .unwrap()
        .to_owned();
    prime_audio(&s, &expected);
    let inv = invocation(
        json!([
            {"api":"tasks.list","args":{}},
            {"api":"replies.say","args":{"text":"{{steps.0.speech}}"}}
        ]),
        "",
    );
    s.execute_invocation(&inv, "model").await.unwrap();
    assert_eq!(texts(&s), vec![expected]);
    assert_eq!(s.transport.audio_frames, 1);
    assert_eq!(task_count(&s), 0);
}

#[tokio::test]
async fn ambiguous_named_cancel_is_audible_once_and_preserves_both_tasks() {
    let mut wire = Wire::default();
    let mut cfg = config(vec![binding("nlu.model", "nlu", 0, "stop")]);
    cfg.nlu = serde_json::from_value(json!({"enabled":true,"bert_enabled":true,"intents":[{
        "name":"cancel","templates":["取消{task}"],"action":{"kind":"plan","reply":"",
        "plan":{"conditions":[],"steps":[
            {"api":"tasks.cancel","args":{"text":"{{slots.task}}"}},
            {"api":"replies.say","args":{"text":"{{steps.0.speech}}"}}
        ]}}}],"slots":[{"name":"task","values":[]}]}))
    .unwrap();
    let mut s = session(&mut wire, cfg, Arc::new(Memory::default()));
    let calls = install_model(&mut s, "cancel", 0.99);
    for index in [10, 11] {
        s.engines
            .skill_system
            .execute_step(
                &s.skill_context(index),
                &SkillStep {
                    api: "tasks.create".into(),
                    args: json!({"after":"5s","text":"喝水"}),
                },
            )
            .unwrap();
    }
    let error = s
        .engines
        .skill_system
        .execute_step(
            &s.skill_context(12),
            &SkillStep {
                api: "tasks.cancel".into(),
                args: json!({"text":"喝水"}),
            },
        )
        .unwrap_err();
    let speech = error
        .downcast_ref::<crate::skills::SkillActionError>()
        .unwrap()
        .speech
        .clone();
    prime_audio(&s, &speech);
    assert!(s.handle_skill_gate("取消喝水").await);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(texts(&s), vec![speech]);
    assert_eq!(task_count(&s), 2);
    assert_eq!(s.transport.audio_frames, 1);
}

#[tokio::test]
async fn query_service_speech_never_leaks_another_devices_tasks() {
    let mut wire = Wire::default();
    let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
    let mut foreign = s.skill_context(8);
    foreign.device_id = "other-device".into();
    s.engines
        .skill_system
        .execute_step(
            &foreign,
            &SkillStep {
                api: "tasks.create".into(),
                args: json!({"after":"5s","text":"private other device reminder"}),
            },
        )
        .unwrap();
    let list = s
        .engines
        .skill_system
        .execute_step(
            &s.skill_context(9),
            &SkillStep {
                api: "tasks.list".into(),
                args: json!({}),
            },
        )
        .unwrap();
    let speech = list["speech"].as_str().unwrap();
    assert!(!speech.contains("private other"));
    prime_audio(&s, speech);
    s.execute_invocation(
        &invocation(
            json!([
                {"api":"tasks.list","args":{}},
                {"api":"replies.say","args":{"text":"{{steps.0.speech}}"}}
            ]),
            "",
        ),
        "model",
    )
    .await
    .unwrap();
    assert_eq!(texts(&s), vec![speech]);
}

#[tokio::test]
async fn result_forward_refs_and_effect_injection_preflight_before_any_commit() {
    for steps in [
        json!([
            {"api":"tasks.create","args":{"after":"5s","text":"must not commit"}},
            {"api":"replies.say","args":{"text":"{{steps.2.speech}}"}},
            {"api":"tasks.list","args":{}}
        ]),
        json!([
            {"api":"tasks.create","args":{"after":"5s","text":"must not commit"}},
            {"api":"messages.send","args":{"text":"{{steps.0.speech}}"}}
        ]),
    ] {
        let mut wire = Wire::default();
        let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
        assert!(s
            .execute_invocation(&invocation(steps, ""), "model")
            .await
            .is_err());
        assert_eq!(task_count(&s), 0);
        assert!(s.transport.sent.is_empty());
    }
}
