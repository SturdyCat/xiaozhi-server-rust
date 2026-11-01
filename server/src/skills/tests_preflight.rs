use super::*;
use std::collections::HashMap;

fn receipt(text: &str) -> SkillPlan {
    SkillPlan {
        conditions: vec![],
        steps: vec![
            SkillStep {
                api: "tasks.list".into(),
                args: json!({}),
            },
            SkillStep {
                api: "replies.say".into(),
                args: json!({"text":text}),
            },
        ],
    }
}

#[test]
fn interpolation_rejects_new_refs_across_slot_slot_and_literal_slot_boundaries() {
    for (text, a, b) in [
        ("{{slots.a}}{{slots.b}}}", "{", "{steps.0.speech}"),
        ("{{slots.a}}{{slots.b}}}", "{", "{ctx.device_id}"),
        ("{{slots.a}}{steps.0.speech}{{slots.b}}", "{", "}"),
        (
            "{{slots.a}}{{slots.b}}} / {{steps.0.speech}}",
            "{",
            "{steps.0.speech}",
        ),
    ] {
        let p = receipt(text);
        validate_plan(&p).unwrap();
        let slots = HashMap::from([("a".into(), a.into()), ("b".into(), b.into())]);
        assert!(resolve_plan(&p, &slots).is_err(), "resolve_plan: {text}");
        assert!(
            resolve_step(&p.steps[1], &ctx("a", "r", NOW), &slots).is_err(),
            "resolve_step: {text}"
        );
    }
}

#[test]
fn interpolation_tracks_ctx_ctx_ctx_literal_and_ctx_slot_sources() {
    let mut context = ctx("{", "r", NOW);
    context.session_id = "{steps.0.speech}".into();
    context.check().unwrap(); // Each individual value has no double-brace marker.
    let p = receipt("{{ctx.device_id}}{{ctx.session_id}}}");
    let slots = HashMap::new();
    let preserved = resolve_plan(&p, &slots).unwrap();
    assert!(resolve_step(&p.steps[1], &context, &slots).is_err());
    assert!(resolve_step(&preserved.steps[1], &context, &slots).is_err());
    context.session_id = "}".into();
    let p = receipt("{{ctx.device_id}}{steps.0.speech}{{ctx.session_id}}");
    assert!(resolve_step(&p.steps[1], &context, &slots).is_err());
    let p = receipt("{{ctx.device_id}}{{slots.a}}}");
    let slots = HashMap::from([("a".into(), "{steps.0.speech}".into())]);
    assert!(resolve_step(&p.steps[1], &context, &slots).is_err());
}

#[test]
fn interpolation_preserves_declared_result_refs_and_normal_single_braces() {
    let p =
        receipt("{{slots.a}} {literal} {{ctx.device_id}} {{steps.0.speech}} {{ctx.session_id}}");
    let slots = HashMap::from([("a".into(), "{喝水}".into())]);
    let mut context = ctx("device{one}", "r", NOW);
    context.session_id = "session{two}".into();
    let preserved = resolve_plan(&p, &slots).unwrap();
    assert_eq!(
        preserved.steps[1].args["text"],
        "{喝水} {literal} {{ctx.device_id}} {{steps.0.speech}} {{ctx.session_id}}"
    );
    for step in [&p.steps[1], &preserved.steps[1]] {
        let resolved = resolve_step(step, &context, &slots).unwrap();
        let final_step =
            resolve_step_results(&resolved, &[json!({"speech":"结果{单括号}"})]).unwrap();
        assert_eq!(
            final_step.args["text"],
            "{喝水} {literal} device{one} 结果{单括号} session{two}"
        );
    }
}

#[test]
fn nested_arguments_cannot_introduce_a_new_later_phase_context_ref() {
    let step = SkillStep {
        api: "devices.call".into(),
        args: json!({"tool":"self.test","arguments":{"items":[{"text":"{{slots.a}}{{slots.b}}}"}]}}),
    };
    let p = SkillPlan {
        conditions: vec![],
        steps: vec![step.clone()],
    };
    let slots = HashMap::from([
        ("a".into(), "{".into()),
        ("b".into(), "{ctx.device_id}".into()),
    ]);
    assert!(resolve_plan(&p, &slots).is_err());
    assert!(resolve_step(&step, &ctx("a", "r", NOW), &slots).is_err());
}

#[test]
fn pure_preflight_rejects_deterministic_bad_time_parameters_with_typed_speech() {
    for mut args in [
        json!({"after":"5秒","timezone":"External/Zone"}),
        json!({"after":"一二分钟"}),
        json!({"after":"0秒"}),
        json!({"interval":"367天"}),
        json!({"at":"周一八点"}),
        json!({"at":"2028-02-30T08:00:00Z"}),
        json!({"hourly":"60分"}),
        json!({"weekly":{"weekday":8,"time":"08:00"}}),
        json!({"yearly":{"month":2,"day":30,"time":"08:00"}}),
    ] {
        args["text"] = json!("x");
        let step = SkillStep {
            api: "tasks.create".into(),
            args,
        };
        validate_step(&step).unwrap();
        let error = preflight_step_args(&step).unwrap_err();
        assert!(
            error.downcast_ref::<SkillActionError>().is_some(),
            "{step:?}"
        );
    }
}

#[test]
fn pure_preflight_has_no_clock_or_storage_and_preserves_result_phase() {
    let db = TestDb::new();
    for mut args in [
        json!({"at":"2001-01-01T08:00:00Z"}),
        json!({"at":"今天八点"}),
        json!({"at":"大后天下午3"}),
        json!({"monthly":{"day":31,"time":"08:00"}}),
        json!({"yearly":{"month":2,"day":29,"time":"08:00"}}),
        json!({"interval":"5秒"}),
    ] {
        args["text"] = json!("x");
        preflight_step_args(&SkillStep {
            api: "tasks.create".into(),
            args,
        })
        .unwrap();
    }
    preflight_step_args(&receipt("{{steps.0.speech}}").steps[1]).unwrap();
    assert!(!db.root.exists());
}

#[test]
fn pure_preflight_does_not_break_idempotent_replay_after_absolute_time_has_passed() {
    let db = TestDb::new();
    let at = chrono::DateTime::from_timestamp_millis(NOW + 60_000)
        .unwrap()
        .to_rfc3339();
    let step = SkillStep {
        api: "tasks.create".into(),
        args: json!({"text":"x","at":at}),
    };
    let original = db
        .system
        .execute_step(&ctx("a", "same", NOW), &step)
        .unwrap();
    preflight_step_args(&step).unwrap();
    let retry = db
        .system
        .execute_step(&ctx("a", "same", NOW + 120_000), &step)
        .unwrap();
    assert_eq!(original["task"]["id"], retry["task"]["id"]);
    assert_eq!(retry["deduplicated"], true);
    assert_eq!(db.system.list_tasks(None, 100, 0).unwrap()["total"], 1);
}
