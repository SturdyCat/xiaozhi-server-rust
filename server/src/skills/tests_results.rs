use super::*;
use std::collections::HashMap;

fn plan(source: &str, receipt: &str) -> SkillPlan {
    SkillPlan {
        conditions: vec![],
        steps: vec![
            SkillStep {
                api: source.into(),
                args: match source {
                    "tasks.create" => json!({"text":"喝水","after":"5分钟"}),
                    "tasks.cancel" => json!({"text":"喝水"}),
                    _ => json!({}),
                },
            },
            SkillStep {
                api: "replies.say".into(),
                args: json!({"text":receipt}),
            },
        ],
    }
}
fn ms(text: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(text)
        .unwrap()
        .timestamp_millis()
}

#[test]
fn result_references_preserve_preflight_then_resolve_successful_speech_once() {
    for source in ["tasks.create", "tasks.list", "tasks.cancel"] {
        let p = plan(source, "结果：{{steps.0.speech}}");
        validate_plan(&p).unwrap();
        let receipt = resolve_step(&p.steps[1], &ctx("a", "r", NOW), &HashMap::new()).unwrap();
        assert_eq!(receipt.args["text"], "结果：{{steps.0.speech}}");
        let resolved =
            resolve_step_results(&receipt, &[json!({"speech":"实际持久化结果"})]).unwrap();
        assert_eq!(resolved.args["text"], "结果：实际持久化结果");
    }
}

#[test]
fn forward_self_unknown_non_speech_and_side_effect_references_are_rejected() {
    for text in [
        "{{steps.1.speech}}",
        "{{steps.2.speech}}",
        "{{steps.-1.speech}}",
        "{{steps.00.speech}}",
        "{{steps.0.task_id}}",
        "{{steps.16.speech}}",
    ] {
        assert!(validate_plan(&plan("tasks.list", text)).is_err(), "{text}");
    }
    let mut p = plan("messages.send", "{{steps.0.speech}}");
    p.steps[0].args = json!({"text":"x"});
    assert!(validate_plan(&p).is_err());
    for (api, args) in [
        (
            "tasks.create",
            json!({"text":"{{steps.0.speech}}","after":"5分钟"}),
        ),
        ("tasks.cancel", json!({"task_id":"{{steps.0.speech}}"})),
        (
            "devices.call",
            json!({"tool":"self.test","arguments":{"text":"{{steps.0.speech}}"}}),
        ),
    ] {
        assert!(validate_step(&SkillStep {
            api: api.into(),
            args
        })
        .is_err());
    }
}

#[test]
fn result_resolution_rejects_missing_typed_oversize_and_second_order_payloads() {
    let step = plan("tasks.list", "{{steps.0.speech}}").steps.remove(1);
    assert!(resolve_step_results(&step, &[]).is_err());
    for result in [
        json!({}),
        json!({"speech":7}),
        json!({"speech":"{{slots.task}}"}),
        json!({"speech":"{{steps.1.speech}}"}),
        json!({"speech":"x".repeat(2001)}),
    ] {
        assert!(resolve_step_results(&step, &[result]).is_err());
    }
}

#[test]
fn creation_and_list_read_back_actual_local_time_for_once_interval_and_calendar() {
    let db = TestDb::new();
    let now = ms("2028-01-31T09:00:20+08:00");
    for (index, mut args) in [
        json!({"text":"喝水","after":"5秒"}),
        json!({"text":"开会","interval":"5分钟"}),
        json!({"text":"总结","monthly":{"day":31,"time":"08:00"}}),
    ]
    .into_iter()
    .enumerate()
    {
        args["timezone"] = json!("Asia/Shanghai");
        let result = db
            .system
            .execute_api(
                &ctx("a", &format!("create{index}"), now),
                "tasks.create",
                &args,
            )
            .unwrap();
        let speech = result["speech"].as_str().unwrap();
        assert!(speech.contains("Asia/Shanghai"));
        if index == 0 {
            assert!(speech.contains("2028年01月31日09时00分25秒"));
            assert!(speech.contains("倒计时"));
        }
        if index == 2 {
            assert!(speech.contains("2028年03月31日08时00分00秒"));
        }
    }
    let list = db
        .system
        .execute_api(&ctx("a", "list", now), "tasks.list", &json!({}))
        .unwrap();
    assert_eq!(list["total"], 3);
    assert!(list["speech"]
        .as_str()
        .unwrap()
        .contains("2028年01月31日09时00分25秒"));
    assert!(list["speech"]
        .as_str()
        .unwrap()
        .contains("2028年01月31日09时05分20秒"));
    assert!(!list["speech"].as_str().unwrap().contains("（UTC）"));
}

#[test]
fn exact_text_cancellation_is_device_scoped_and_ambiguous_matches_do_not_mutate() {
    let db = TestDb::new();
    let first = create(
        &db.system,
        &ctx("a", "one", NOW),
        json!({"text":"喝水","after":"1分钟"}),
    );
    create(
        &db.system,
        &ctx("a", "two", NOW),
        json!({"text":"喝水","after":"2分钟"}),
    );
    create(
        &db.system,
        &ctx("b", "other", NOW),
        json!({"text":"喝水","after":"1分钟"}),
    );
    let error = db
        .system
        .execute_api(
            &ctx("a", "cancel", NOW),
            "tasks.cancel",
            &json!({"text":"喝水"}),
        )
        .unwrap_err();
    let spoken = error.downcast_ref::<SkillActionError>().unwrap();
    assert!(spoken.speech.contains("多个") && spoken.speech.contains("管理任务列表"));
    assert_eq!(
        db.system.get_task(&first, None).unwrap().unwrap()["state"],
        "active"
    );
    let result = db
        .system
        .execute_api(
            &ctx("b", "cancel", NOW),
            "tasks.cancel",
            &json!({"text":"喝水"}),
        )
        .unwrap();
    assert_eq!(result["cancelled"], true);
    assert!(result["speech"].as_str().unwrap().contains("已取消"));
    assert_eq!(
        db.system
            .execute_api(&ctx("b", "list", NOW), "tasks.list", &json!({}))
            .unwrap()["total"],
        0
    );
    for args in [json!({"text":"喝水","task_id":first}), json!({})] {
        assert!(validate_step(&SkillStep {
            api: "tasks.cancel".into(),
            args
        })
        .is_err());
    }
    let error = db
        .system
        .execute_api(
            &ctx("a", "missing", NOW),
            "tasks.cancel",
            &json!({"text":"不存在"}),
        )
        .unwrap_err();
    assert!(error.downcast_ref::<SkillActionError>().is_some());
}

#[test]
fn malformed_time_has_safe_clarification_without_creating_a_task() {
    let db = TestDb::new();
    let error = db
        .system
        .execute_api(
            &ctx("a", "bad", NOW),
            "tasks.create",
            &json!({"text":"x","yearly":{"month":2,"day":30,"time":"08:00"}}),
        )
        .unwrap_err();
    assert!(error.downcast_ref::<SkillActionError>().is_some());
    assert_eq!(db.system.list_tasks(None, 100, 0).unwrap()["total"], 0);
}
