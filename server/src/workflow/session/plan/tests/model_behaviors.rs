//! Production Session behavior with fake model/transport and real SQLite APIs.
use super::*;
mod clock;
mod delivery_recovery;
mod mcp_permissions;
mod preflight;
mod tuya_control;
use crate::plugins::skill::{SkillAction, SkillInvocation};
use crate::skills::{SkillPlan, SkillStep};
use crate::workflow::session::command::now_ms;
use std::collections::HashMap;

fn install_model(s: &mut Session<'_, Wire>, label: &str, confidence: f32) -> Arc<AtomicUsize> {
    let model = LabelBert::new(label, confidence);
    let count = model.calls.clone();
    s.skills = SkillRegistry::new(
        &s.eff_config.command,
        &s.eff_config.nlu,
        Some(Arc::new(model)),
    );
    count
}
fn prime_audio(s: &Session<'_, Wire>, text: &str) {
    let key = crate::media::hot_audio::HotAudioKey::from_lease(&s.tts_lease, text);
    s.engines
        .tts_cache
        .insert(key.mem_key(s.params.output_format()), vec![vec![1, 2, 3]]);
}
fn invocation(steps: Value, reply: &str) -> SkillInvocation {
    SkillInvocation {
        name: "configured".into(),
        slots: HashMap::new(),
        reply: reply.into(),
        action: SkillAction::Plan {
            plan: serde_json::from_value(json!({"steps":steps})).unwrap(),
        },
    }
}
fn task_count(s: &Session<'_, Wire>) -> u64 {
    s.engines
        .skill_system
        .list_tasks(Some(&s.device_id), 100, 0)
        .unwrap()["total"]
        .as_u64()
        .unwrap()
}

#[tokio::test]
async fn disabled_rejected_and_invalid_scores_never_execute_keyword_or_template() {
    for (label, confidence, enabled, consumes) in [
        ("end_session", 0.99, false, false),
        ("other", 0.99, true, false),
        ("end_session", 0.1, true, false),
        ("end_session", f32::NAN, true, true),
    ] {
        let mut cfg = config(vec![binding("nlu.model", "nlu", 0, "stop")]);
        cfg.nlu.bert_enabled = enabled;
        let mut wire = Wire::default();
        let mut s = session(&mut wire, cfg, Arc::new(Memory::default()));
        let count = install_model(&mut s, label, confidence);
        assert_eq!(s.handle_skill_gate("退下").await, consumes);
        assert_eq!(count.load(Ordering::Relaxed), usize::from(enabled));
        assert!(!s.close_requested);
        assert!(s.transport.sent.is_empty());
        assert!(s.skills.tools().is_empty());
    }
}

#[tokio::test]
async fn old_aliases_cannot_expand_disabled_or_conditional_intent_authority() {
    for command_only in [false, true] {
        let id = if command_only {
            "command.gate"
        } else {
            "nlu.rules"
        };
        let pos = if command_only { "link.asr.nlu" } else { "nlu" };
        let mut cfg = config(vec![binding(id, pos, 0, "stop")]);
        cfg.nlu.intents = serde_json::from_value(json!([{
            "name":"custom", "templates":["退下"], "action":{"kind":"reply","reply":"must not speak"}
        }])).unwrap();
        let mut wire = Wire::default();
        let mut s = session(&mut wire, cfg, Arc::new(Memory::default()));
        install_model(
            &mut s,
            if command_only {
                "custom"
            } else {
                "end_session"
            },
            0.99,
        );
        assert!(!s.handle_skill_gate("退下").await);
        assert!(!s.close_requested);
        assert!(s.transport.sent.is_empty());
    }
}

#[tokio::test]
async fn missing_slots_clarify_only_the_selected_intent_without_task_or_llm_reselection() {
    let mut cfg = config(vec![binding("nlu.model", "nlu", 0, "stop")]);
    cfg.nlu.intents = serde_json::from_value(json!([{
        "name":"note", "templates":["记下{content}"],
        "action":{"kind":"plan","plan":{"steps":[{"api":"tasks.create","args":{"after":"5s","text":"{{slots.content}}"}}]}}
    }])).unwrap();
    cfg.nlu.slots =
        serde_json::from_value(json!([{"name":"content","values":[],"optional":false}])).unwrap();
    let mut wire = Wire::default();
    let mut s = session(&mut wire, cfg, Arc::new(Memory::default()));
    let count = install_model(&mut s, "note", 0.99);
    prime_audio(&s, "执行note还需要补充：content。请说明完整信息。");
    assert!(s.handle_skill_gate("没有足够参数的另一种表达").await);
    assert_eq!(count.load(Ordering::Relaxed), 1);
    assert_eq!(task_count(&s), 0);
    assert_eq!(s.transport.audio_frames, 1);
    assert!(s.skills.tools().is_empty());
    assert!(!s.close_requested);
}

#[tokio::test]
async fn plan_preflights_every_step_and_condition_before_any_persisted_effect() {
    for mode in 0..3 {
        let mut wire = Wire::default();
        let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
        let bad = if mode == 0 {
            json!({"api":"tasks.create","args":{"after":"5s","text":"{{slots.missing}}"}})
        } else if mode == 1 {
            json!({"api":"arbitrary.shell","args":{}})
        } else {
            json!({"api":"tasks.create","args":{"after":"5s","text":"ok","device_id":"another-device"}})
        };
        let inv = invocation(
            json!([{"api":"messages.send","args":{"text":"must not persist"}},bad]),
            "must not claim success",
        );
        assert!(s.execute_invocation(&inv, "model").await.is_err());
        assert!(s
            .engines
            .skill_system
            .claim_delivery(&s.device_id, "observer", now_ms())
            .unwrap()
            .is_none());
        assert_eq!(task_count(&s), 0);
        assert!(s.transport.sent.is_empty());
    }
    let mut wire = Wire::default();
    let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
    let plan: SkillPlan = serde_json::from_value(json!({"conditions":[{"kind":"device_kind","value":"echomuse"}],"steps":[{"api":"messages.send","args":{"text":"no"}}]})).unwrap();
    let inv = SkillInvocation {
        name: "conditional".into(),
        slots: HashMap::new(),
        reply: "no".into(),
        action: SkillAction::Plan { plan },
    };
    assert!(s.execute_invocation(&inv, "model").await.is_err());
    assert!(s
        .engines
        .skill_system
        .claim_delivery(&s.device_id, "observer", now_ms())
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn receipt_failure_occurs_after_persistence_and_same_turn_retry_is_idempotent() {
    let mut wire = Wire {
        fail: true,
        ..Default::default()
    };
    let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
    let inv = invocation(
        json!([{"api":"tasks.create","args":{"after":"5s","text":"persisted before receipt"}}]),
        "created",
    );
    assert!(s.execute_invocation(&inv, "model").await.is_err());
    assert_eq!(
        task_count(&s),
        1,
        "receipt error must not precede the durable action"
    );
    assert!(s.execute_invocation(&inv, "model").await.is_err());
    assert_eq!(
        task_count(&s),
        1,
        "stable session:turn:step key prevents duplicate durable effects"
    );
    assert!(s.transport.sent.is_empty());
}

#[tokio::test]
async fn action_failure_or_abort_never_emits_success_receipt_or_another_effect() {
    let mut wire = Wire::default();
    let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
    let inv = invocation(
        json!([{"api":"tasks.create","args":{"after":"ambiguous","text":"no"}}]),
        "success",
    );
    assert!(s.execute_invocation(&inv, "model").await.is_err());
    assert_eq!(task_count(&s), 0);
    assert!(s.transport.sent.is_empty());
    s.transport
        .incoming
        .push_back(IncomingFrame::Text(r#"{"type":"abort"}"#.into()));
    let inv = invocation(
        json!([{"api":"tasks.create","args":{"after":"5s","text":"no"}}]),
        "success",
    );
    assert!(s.execute_invocation(&inv, "model").await.is_err());
    assert_eq!(task_count(&s), 0);
    assert!(s.speak_canned_in_turn("not after abort").await.is_err());
    assert!(s.transport.sent.is_empty());
}

#[tokio::test]
async fn later_step_failure_is_not_atomic_and_never_emits_a_success_receipt() {
    let mut wire = Wire::default();
    let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
    let inv = invocation(
        json!([
            {"api":"tasks.create","args":{"after":"5s","text":"committed first step"}},
            {"api":"tasks.cancel","args":{"task_id":"nonexistent"}},
            {"api":"messages.send","args":{"text":"must not execute"}}
        ]),
        "all done",
    );
    assert!(s.execute_invocation(&inv, "model").await.is_err());
    assert_eq!(
        task_count(&s),
        1,
        "committed prior step is not falsely claimed to roll back"
    );
    assert!(s.transport.sent.is_empty());
    assert!(s
        .engines
        .skill_system
        .claim_delivery(&s.device_id, "observer", now_ms())
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn end_session_cannot_hide_unexecuted_actions_in_the_middle_of_a_plan() {
    let mut wire = Wire::default();
    let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
    let inv = invocation(
        json!([
            {"api":"sessions.end","args":{}},
            {"api":"tasks.create","args":{"after":"5s","text":"not executed"}}
        ]),
        "all done",
    );
    assert!(s.execute_invocation(&inv, "model").await.is_err());
    assert_eq!(task_count(&s), 0);
    assert!(!s.close_requested);
    assert!(s.transport.sent.is_empty());
}

#[tokio::test]
async fn task_delivery_uses_canned_audio_once_not_user_injection_or_model_recognition() {
    let mut wire = Wire::default();
    let mut s = session(
        &mut wire,
        config(vec![binding("nlu.model", "nlu", 0, "stop")]),
        Arc::new(Memory::default()),
    );
    let count = install_model(&mut s, "end_session", 0.99);
    let mut ctx = s.skill_context(0);
    ctx.now_ms = now_ms() - 10_000;
    let step = SkillStep {
        api: "tasks.create".into(),
        args: json!({"after":"1s","text":"退下"}),
    };
    s.engines.skill_system.execute_step(&ctx, &step).unwrap();
    assert_eq!(
        s.engines
            .skill_system
            .tick_due(now_ms())
            .unwrap()
            .materialized,
        1
    );
    prime_audio(&s, "退下");
    s.deliver_pending_message().await;
    let first_delivery_frames = s.transport.audio_frames;
    s.deliver_pending_message().await;
    assert_eq!(s.transport.audio_frames, first_delivery_frames);
    assert!(
        first_delivery_frames > 1,
        "local alarm precedes the one cached speech frame"
    );
    assert_eq!(count.load(Ordering::Relaxed), 0);
    assert!(
        !s.close_requested,
        "reminder text must not become an end_session intent"
    );
    assert!(s.history.is_empty());
    assert!(s
        .engines
        .skill_system
        .claim_delivery(&s.device_id, "observer", now_ms() + 1000)
        .unwrap()
        .is_none());
    let states: Vec<_> = s
        .transport
        .sent
        .iter()
        .map(|raw| serde_json::from_str::<Value>(raw).unwrap())
        .collect();
    assert!(states
        .iter()
        .any(|v| v["type"] == "tts" && v["state"] == "start"));
    assert!(states
        .iter()
        .any(|v| v["type"] == "tts" && v["state"] == "stop"));
    assert!(!states.iter().any(|v| v["type"] == "stt"));
}

#[tokio::test]
async fn model_end_session_preserves_farewell_order_and_all_four_mcp_fallbacks() {
    for mode in 0..5 {
        let mut wire = Wire::default();
        if mode == 2 {
            wire.mcp_response = Some(json!({"error":{"code":-32602,"message":"Unknown tool"}}));
        }
        if mode == 4 {
            wire.mcp_response = Some(json!({"result":{"content":[],"isError":false}}));
        }
        let mut cfg = config(vec![binding("nlu.model", "nlu", 0, "stop")]);
        cfg.command.reply = "bye".into();
        let mut s = session(&mut wire, cfg, Arc::new(Memory::default()));
        s.mcp_enabled = mode != 0;
        if mode == 1 {
            s.engines
                .mcp
                .record(crate::workflow::mcp::registry::ProbeResult {
                    device_id: s.device_id.clone(),
                    session_id: s.session_id.clone(),
                    probed_at_ms: 0,
                    board: String::new(),
                    firmware_version: String::new(),
                    protocol_version: String::new(),
                    tools: vec![],
                    error: None,
                });
        }
        prime_audio(&s, "bye");
        assert!(s.handle_skill_gate("退下").await);
        assert_eq!(s.transport.audio_frames, 1);
        assert_eq!(s.close_requested, mode != 4);
        let messages: Vec<Value> = s
            .transport
            .sent
            .iter()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        let mcp = messages
            .iter()
            .position(|v| v["payload"]["method"] == "tools/call");
        assert_eq!(mcp.is_some(), mode >= 2);
        if let Some(mcp) = mcp {
            let stop = messages
                .iter()
                .position(|v| v["type"] == "tts" && v["state"] == "stop")
                .unwrap();
            assert!(stop < mcp, "farewell must finish tts stop before MCP stop");
        }
    }
}

#[tokio::test]
async fn non_idle_test_and_aborted_sessions_do_not_claim_pending_delivery() {
    for mode in 0..3 {
        let mut wire = Wire::default();
        let mut s = session(&mut wire, config(vec![]), Arc::new(Memory::default()));
        let ctx = s.skill_context(0);
        s.engines
            .skill_system
            .execute_step(
                &ctx,
                &SkillStep {
                    api: "messages.send".into(),
                    args: json!({"text":"pending"}),
                },
            )
            .unwrap();
        if mode == 0 {
            s.delivery_idle = false;
        }
        if mode == 1 {
            s.is_test = true;
        }
        if mode == 2 {
            s.abort.store(true, Ordering::Relaxed);
        }
        s.deliver_pending_message().await;
        assert!(s.transport.sent.is_empty());
        assert!(s
            .engines
            .skill_system
            .claim_delivery(&s.device_id, "observer", now_ms())
            .unwrap()
            .is_some());
    }
}
