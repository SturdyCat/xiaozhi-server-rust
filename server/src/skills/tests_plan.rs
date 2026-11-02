use super::*;
use std::{collections::HashMap, sync::atomic::Ordering};

fn plan(api: &str, args: Value) -> SkillPlan {
    SkillPlan {
        conditions: vec![],
        steps: vec![SkillStep {
            api: api.into(),
            args,
        }],
    }
}

#[test]
fn plan_serde_roundtrip_and_conditions_are_finite() {
    let value = json!({"conditions":[{"kind":"slot_present","slot":"task"},{"kind":"slot_equals","slot":"mode","value":"quiet"},{"kind":"device_kind","value":"esp32"}],"steps":[{"api":"messages.send","args":{"text":"{{slots.task}}"}}]});
    let plan: SkillPlan = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&plan).unwrap(), value);
    validate_plan(&plan).unwrap();
    let slots = HashMap::from([
        ("task".into(), "喝水".into()),
        ("mode".into(), "quiet".into()),
    ]);
    assert!(conditions_match(&plan, &slots, Some("esp32")));
    assert!(!conditions_match(&plan, &slots, Some("echomuse")));
    assert!(serde_json::from_value::<SkillPlan>(
        json!({"conditions":[{"kind":"javascript","code":"evil"}],"steps":[]})
    )
    .is_err());
    assert!(serde_json::from_value::<SkillPlan>(json!({"steps":[],"http":"evil"})).is_err());
}

#[test]
fn tree_interpolation_preserves_strings_never_injects_json_or_device() {
    let context = ctx("trusted-device", "r", NOW);
    let plan = plan(
        "devices.call",
        json!({"tool":"self.test","arguments":{"text":"{{slots.task}}","owner":"{{ctx.device_id}}","nested":["{{ctx.session_id}}",7]}}),
    );
    let injected = "\"},\"device_id\":\"attacker\",\"x\":{\"";
    let slots = HashMap::from([
        ("task".into(), injected.into()),
        ("device_id".into(), "attacker".into()),
    ]);
    let step = resolve_step(&plan.steps[0], &context, &slots).unwrap();
    assert_eq!(step.args["arguments"]["text"], injected);
    assert_eq!(step.args["arguments"]["owner"], "trusted-device");
    assert!(step.args.get("device_id").is_none());
    assert_eq!(step.args["arguments"]["nested"][1], 7);
}

#[test]
fn missing_unknown_and_second_order_references_fail_closed() {
    for text in [
        "{{slots.missing}}",
        "{{env.secret}}",
        "{{ctx.now_ms}}",
        "{{slots.task",
        "abc}}",
    ] {
        let p = plan("messages.send", json!({"text":text}));
        assert!(
            resolve_step(&p.steps[0], &ctx("d", "r", NOW), &HashMap::new()).is_err(),
            "{text}"
        );
    }
    let p = plan("messages.send", json!({"text":"{{slots.task}}"}));
    let slots = HashMap::from([("task".into(), "{{ctx.device_id}}".into())]);
    assert!(resolve_plan(&p, &slots).is_err());
}

#[test]
fn validate_rejects_unknown_capabilities_dynamic_tools_and_unbound_device_fields() {
    for p in [
        plan("http.post", json!({"url":"https://evil"})),
        plan(
            "devices.call",
            json!({"tool":"{{slots.tool}}","arguments":{}}),
        ),
        plan(
            "tasks.create",
            json!({"text":"喝水","after":"1分钟","device_id":"evil"}),
        ),
        plan("messages.send", json!({"text":"x","device_id":"evil"})),
        plan(
            "tasks.create",
            json!({"text":"x","after":"1分钟","daily":"08:00"}),
        ),
        plan("tasks.create", json!({"text":"x"})),
        plan("tasks.list", json!({"limit":101})),
        plan("devices.call", json!({"tool":"self.test","arguments":[]})),
    ] {
        assert!(validate_plan(&p).is_err(), "{p:?}");
    }
}

#[test]
fn plan_limits_bound_depth_nodes_steps_and_strings() {
    let mut p = plan("messages.send", json!({"text":"x"}));
    p.steps = vec![p.steps[0].clone(); 17];
    assert!(validate_plan(&p).is_err());
    assert!(validate_plan(&plan("messages.send", json!({"text":"x".repeat(2001)}))).is_err());
    let mut nested = json!(0);
    for _ in 0..20 {
        nested = json!([nested]);
    }
    assert!(validate_plan(&plan(
        "devices.call",
        json!({"tool":"self.test","arguments":{"nested":nested}})
    ))
    .is_err());
    assert!(validate_plan(&plan(
        "devices.call",
        json!({"tool":"self.test","arguments":{"large":vec![0;1100]}})
    ))
    .is_err());
}

#[test]
fn constructor_and_invalid_or_cancelled_execution_have_no_filesystem_effects() {
    let db = TestDb::new();
    assert!(!db.root.exists());
    let context = ctx("d", "r", NOW);
    context.cancelled.store(true, Ordering::Release);
    assert!(db
        .system
        .execute_api(&context, "messages.send", &json!({"text":"x"}))
        .is_err());
    assert!(!db.root.exists());
    assert!(db
        .system
        .execute_api(&ctx("d", "r", NOW), "unknown", &json!({}))
        .is_err());
    assert!(!db.root.exists());
    assert!(db
        .system
        .execute_api(
            &ctx("d", "r", NOW),
            "messages.send",
            &json!({"text":"{{slots.task}}"})
        )
        .is_err());
    assert!(!db.root.exists());
}

#[test]
fn session_capabilities_only_return_dispatch_not_audio_success() {
    let db = TestDb::new();
    let result = db
        .system
        .execute_api(&ctx("d", "r", NOW), "replies.say", &json!({"text":"你好"}))
        .unwrap();
    assert_eq!(result["dispatch"], "replies.say");
    assert!(result.get("played").is_none());
    assert!(!db.root.exists());
}

#[test]
fn direct_dispatch_preserves_nested_objects_without_false_placeholder_detection() {
    let db = TestDb::new();
    let args = json!({"tool":"self.test","arguments":{"nested":{"x":1}}});
    let value = db
        .system
        .execute_api(&ctx("d", "r", NOW), "devices.call", &args)
        .unwrap();
    assert_eq!(value["args"], args);
    assert!(!db.root.exists());
}

#[test]
fn terminal_session_step_must_be_last() {
    let mut p = plan("sessions.end", json!({}));
    p.steps.push(SkillStep {
        api: "messages.send".into(),
        args: json!({"text":"never reached"}),
    });
    assert!(validate_plan(&p)
        .unwrap_err()
        .to_string()
        .contains("final plan step"));
    p.steps.reverse();
    validate_plan(&p).unwrap();
}

#[test]
fn capability_registry_is_serializable_and_complete() {
    let names: Vec<_> = capabilities().iter().map(|metadata| metadata.api).collect();
    assert_eq!(names.len(), 8);
    assert!(names.contains(&"tasks.create") && names.contains(&"devices.call"));
    assert!(serde_json::to_value(capabilities()).is_ok());
}

// ---------------------------------------------------------------------------
// tasks.create 的可选 `action`（到期动作，见 plan.rs `validate_create_action`）
// ---------------------------------------------------------------------------

#[test]
fn tasks_create_accepts_a_single_devices_call_action() {
    let p = plan(
        "tasks.create",
        json!({"text":"已按时关闭电源","after":"10分钟",
               "action":{"api":"devices.call","args":{"tool":"power","arguments":{"state":"off"}}}}),
    );
    validate_plan(&p).unwrap();
}

#[test]
fn tasks_create_action_whitelist_rejects_everything_but_devices_call() {
    // 递归调度（到期再排到期）明确禁止——语义与容量上界都说不清。
    let recursive = plan(
        "tasks.create",
        json!({"text":"x","after":"1分钟",
               "action":{"api":"tasks.create","args":{"text":"y","after":"1分钟"}}}),
    );
    assert!(validate_plan(&recursive).is_err());
    // 会话内应答 / 纯文本类与到期播报重复，也不放行。
    let reply = plan(
        "tasks.create",
        json!({"text":"x","after":"1分钟","action":{"api":"replies.say","args":{"text":"y"}}}),
    );
    assert!(validate_plan(&reply).is_err());
    // 形状必须是单步对象（deny_unknown_fields 锁死），不能是字符串或多余字段。
    let not_object = plan(
        "tasks.create",
        json!({"text":"x","after":"1分钟","action":"devices.call"}),
    );
    assert!(validate_plan(&not_object).is_err());
    let extra_field = plan(
        "tasks.create",
        json!({"text":"x","after":"1分钟",
               "action":{"api":"devices.call","args":{"tool":"power","arguments":{}},"extra":1}}),
    );
    assert!(validate_plan(&extra_field).is_err());
    // 缺 tool / arguments 的 devices.call 同样被内层 validate_step 拦下。
    let missing_tool = plan(
        "tasks.create",
        json!({"text":"x","after":"1分钟","action":{"api":"devices.call","args":{"arguments":{}}}}),
    );
    assert!(validate_plan(&missing_tool).is_err());
}

#[test]
fn slots_resolve_deeply_inside_nested_action_args() {
    // tool 按设计是静态字面量（不许插值），槽位只进 arguments——
    // validate_plan 在解析前就会拒绝 "{{slots.tool}}" 当 tool（由上面的
    // unknown_capabilities 测试族覆盖 devices.call 顶层，这里验嵌套层同样生效）。
    let nested_tool = plan(
        "tasks.create",
        json!({"text":"x","after":"10分钟",
               "action":{"api":"devices.call","args":{"tool":"{{slots.tool}}","arguments":{}}}}),
    );
    assert!(validate_plan(&nested_tool).is_err());
    let p = plan(
        "tasks.create",
        json!({"text":"已按时关闭电源","after":"10分钟",
               "action":{"api":"devices.call",
                         "args":{"tool":"power","arguments":{"state":"{{slots.state}}"}}}}),
    );
    let slots = HashMap::from([("state".into(), "off".into())]);
    let resolved = resolve_plan(&p, &slots).unwrap();
    assert_eq!(resolved.steps[0].args["action"]["args"]["tool"], "power");
    assert_eq!(
        resolved.steps[0].args["action"]["args"]["arguments"]["state"],
        "off"
    );
    // 缺槽位照样失败关闭：嵌套里的未解析占位符不允许带进到期执行。
    assert!(resolve_plan(&p, &HashMap::new()).is_err());
}
