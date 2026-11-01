//! 统一目录、初始数据与保存计划验证。
use super::*;
use serde_json::json;

#[test]
fn effective_training_catalog_contains_saved_command_examples_exactly_once() {
    let (mut command, nlu) = fixture();
    command.keywords = vec!["请退下吧。".into(), "停止聆听".into(), "请退下吧。".into()];
    let catalog = effective_catalog(&command, &nlu).unwrap();
    let ends: Vec<_> = catalog
        .intents
        .iter()
        .filter(|i| i.name == "end_session")
        .collect();
    assert_eq!(ends.len(), 1);
    assert_eq!(ends[0].templates, ["请退下吧。", "停止聆听"]);
    assert_eq!(ends[0].action.kind, "end_session");
    assert_eq!(ends[0].action.reply, command.reply);
}

#[test]
fn explicit_clear_is_not_repopulated_and_command_switch_is_independent() {
    let mut command = CommandConfig::default();
    let nlu = NluConfig {
        intents: vec![],
        slots: vec![],
        ..Default::default()
    };
    let catalog = effective_catalog(&command, &nlu).unwrap();
    assert_eq!(catalog.intents.len(), 1);
    assert_eq!(catalog.intents[0].name, "end_session");
    command.enabled = false;
    let catalog = effective_catalog(&command, &nlu).unwrap();
    assert!(!catalog.enabled && catalog.intents.is_empty());
}

#[test]
fn nlu_switch_removes_custom_training_and_runtime_definitions_only() {
    let (command, mut nlu) = fixture();
    nlu.enabled = false;
    let catalog = effective_catalog(&command, &nlu).unwrap();
    assert_eq!(
        catalog
            .intents
            .iter()
            .map(|i| i.name.as_str())
            .collect::<Vec<_>>(),
        ["end_session"]
    );
    assert!(catalog.slots.is_empty());
}

#[test]
fn saved_legacy_reminder_action_is_not_migrated_or_overridden() {
    let command = CommandConfig::default();
    let mut nlu = NluConfig::default();
    let reminder = nlu
        .intents
        .iter_mut()
        .find(|i| i.name == "reminder_countdown")
        .unwrap();
    reminder.action = ActionDef {
        kind: "reply".into(),
        reply: "旧文字应答".into(),
        ..Default::default()
    };
    let effective = effective_catalog(&command, &nlu).unwrap();
    let reminder = effective
        .intents
        .iter()
        .find(|i| i.name == "reminder_countdown")
        .unwrap();
    assert_eq!(reminder.action.kind, "reply");
    assert!(reminder.action.plan.is_none());
    assert_eq!(
        effective
            .intents
            .iter()
            .filter(|i| i.name == "reminder_countdown")
            .count(),
        1
    );
}

#[test]
fn reserved_and_duplicate_names_fail_closed_before_training_or_execution() {
    for name in ["", "other", "end_session", "control_ac", " help"] {
        let (command, mut nlu) = fixture();
        nlu.intents[0].name = name.into();
        assert!(
            effective_catalog(&command, &nlu).is_err(),
            "reserved/bad name: {name}"
        );
        let registry = SkillRegistry::new(&command, &nlu, None);
        assert!(matches!(
            registry.recognize("退下"),
            Recognition::Unavailable { .. }
        ));
    }
}

#[test]
fn other_remapping_and_unknown_target_cannot_authorize_actions() {
    let (command, mut nlu) = fixture();
    nlu.bert_label_map
        .insert("other".into(), "end_session".into());
    assert!(validate(&command, &nlu).is_err());
    nlu.bert_label_map.clear();
    nlu.bert_label_map
        .insert("label".into(), "unregistered".into());
    assert!(validate(&command, &nlu).is_err());
}

#[test]
fn valid_legacy_model_mapping_still_uses_only_real_classifier() {
    let (command, mut nlu) = fixture();
    nlu.bert_label_map.insert("label_0".into(), "help".into());
    let (registry, _) = registry_with(&command, &nlu, "label_0", 0.99, false);
    assert!(
        matches!(registry.recognize("不同表达"), Recognition::Accepted { inv, .. } if inv.name == "help")
    );
}

#[test]
fn bad_threshold_is_rejected_but_deprecated_heuristic_values_are_inert() {
    let (command, mut nlu) = fixture();
    nlu.model_threshold = f32::NAN;
    nlu.model_margin = 100.0;
    validate(&command, &nlu).unwrap();
    for threshold in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
        nlu.bert_threshold = threshold;
        assert!(validate(&command, &nlu).is_err());
    }
}

fn with_plan(plan: serde_json::Value) -> (CommandConfig, NluConfig) {
    let (command, mut nlu) = fixture();
    nlu.intents[0].action = ActionDef {
        kind: "plan".into(),
        plan: Some(serde_json::from_value(plan).unwrap()),
        ..Default::default()
    };
    (command, nlu)
}

#[test]
fn plan_validation_requires_registered_capabilities_and_safe_arguments() {
    let valid = json!({"conditions":[{"kind":"slot_present","slot":"room"}],"steps":[{"api":"devices.call","args":{"tool":"light.set","arguments":{"room":"{{slots.room}}","action":"{{slots.action}}"}}}]});
    let (command, nlu) = with_plan(valid);
    validate(&command, &nlu).unwrap();
    let unknown = json!({"steps":[{"api":"shell.execute","args":{"command":"anything"}}]});
    let (command, nlu) = with_plan(unknown);
    assert!(validate(&command, &nlu).is_err());
    let dynamic =
        json!({"steps":[{"api":"devices.call","args":{"tool":"{{slots.action}}","arguments":{}}}]});
    let (command, nlu) = with_plan(dynamic);
    assert!(validate(&command, &nlu).is_err());
}

#[test]
fn plan_references_are_limited_to_same_skill_and_trusted_context() {
    for reference in [
        "{{slots.unregistered}}",
        "{{ctx.admin}}",
        "{{user.device_id}}",
    ] {
        let (command, nlu) =
            with_plan(json!({"steps":[{"api":"messages.send","args":{"text":reference}}]}));
        assert!(
            validate(&command, &nlu).is_err(),
            "bad reference: {reference}"
        );
    }
    let (command, nlu) = with_plan(
        json!({"steps":[{"api":"messages.send","args":{"text":"{{slots.room}}/{{ctx.session_id}}"}}]}),
    );
    validate(&command, &nlu).unwrap();
}

#[test]
fn only_typed_conditions_interpret_slot_property_as_a_slot_name() {
    let (command, nlu) = with_plan(
        json!({"steps":[{"api":"devices.call","args":{"tool":"light.set","arguments":{"slot":"12","kind":"slot_present"}}}]}),
    );
    validate(&command, &nlu).unwrap();
    let (command, nlu) = with_plan(
        json!({"conditions":[{"kind":"slot_equals","slot":"unregistered","value":"12"}],"steps":[{"api":"messages.send","args":{"text":"hello"}}]}),
    );
    assert!(validate(&command, &nlu).is_err());
}

#[test]
fn plan_kind_is_required_and_empty_or_missing_plan_is_rejected() {
    let (command, mut nlu) = fixture();
    nlu.intents[0].action.kind = "plan".into();
    assert!(validate(&command, &nlu).is_err());
    let (command, mut nlu) = with_plan(json!({"steps":[]}));
    assert!(validate(&command, &nlu).is_err());
    nlu.intents[0].action.kind = "reply".into();
    assert!(validate(&command, &nlu).is_err());
}

#[test]
fn fresh_clock_catalog_has_eleven_seed_skills_and_independent_command() {
    let command = CommandConfig::default();
    let nlu = NluConfig::default();
    let catalog = effective_catalog(&command, &nlu).unwrap();
    let names: Vec<_> = catalog.intents.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "reminder_countdown",
            "reminder_schedule",
            "reminder_repeat",
            "reminder_daily",
            "reminder_workdays",
            "reminder_weekly",
            "reminder_monthly",
            "reminder_yearly",
            "reminder_hourly",
            "reminder_list",
            "reminder_cancel",
            "end_session",
        ]
    );
    for (name, key, expected) in [
        (
            "reminder_workdays",
            "workdays",
            json!("{{slots.daily_time}}"),
        ),
        (
            "reminder_weekly",
            "weekly",
            json!({"weekday":"{{slots.weekday}}","time":"{{slots.daily_time}}"}),
        ),
        (
            "reminder_monthly",
            "monthly",
            json!({"day":"{{slots.day}}","time":"{{slots.daily_time}}"}),
        ),
        (
            "reminder_yearly",
            "yearly",
            json!({"month":"{{slots.month}}","day":"{{slots.day}}","time":"{{slots.daily_time}}"}),
        ),
        ("reminder_hourly", "hourly", json!("{{slots.minute}}")),
    ] {
        let intent = catalog.intents.iter().find(|i| i.name == name).unwrap();
        let plan = intent.action.plan.as_ref().unwrap();
        assert_eq!(plan.steps[0].api, "tasks.create");
        assert_eq!(plan.steps[0].args[key], expected);
    }
}

#[test]
fn step_results_are_only_safe_backward_speech_not_action_parameters() {
    let (command, nlu) = with_plan(json!({"steps":[
        {"api":"tasks.list","args":{}},
        {"api":"replies.say","args":{"text":"{{steps.0.speech}}"}}
    ]}));
    validate(&command, &nlu).unwrap();
    for step in [
        json!({"api":"tasks.create","args":{"text":"{{steps.0.speech}}","after":"1分钟"}}),
        json!({"api":"tasks.cancel","args":{"text":"{{steps.0.speech}}"}}),
        json!({"api":"devices.call","args":{"tool":"{{steps.0.speech}}","arguments":{}}}),
        json!({"api":"replies.say","args":{"text":"{{steps.1.speech}}"}}),
        json!({"api":"replies.say","args":{"text":"{{steps.0.tasks}}"}}),
    ] {
        let (command, nlu) = with_plan(json!({"steps":[{"api":"tasks.list","args":{}},step]}));
        assert!(validate(&command, &nlu).is_err());
    }
}

#[test]
fn adjacent_slots_only_save_when_first_slot_has_required_finite_values() {
    let command = CommandConfig::default();
    let mut nlu = NluConfig::default();
    validate(&command, &nlu).unwrap();
    nlu.slots
        .iter_mut()
        .find(|s| s.name == "weekday")
        .unwrap()
        .values
        .clear();
    assert!(validate(&command, &nlu).is_err());
    let mut nlu = NluConfig::default();
    nlu.slots
        .iter_mut()
        .find(|s| s.name == "weekday")
        .unwrap()
        .optional = true;
    assert!(validate(&command, &nlu).is_err());

    // fixture.room是必填封闭词表，相邻槽现在合法；清空前槽词表才是真实非法例。
    let (command, mut nlu) = fixture();
    nlu.intents[0].templates = vec!["把{room}{action}".into()];
    validate(&command, &nlu).unwrap();
    nlu.slots
        .iter_mut()
        .find(|s| s.name == "room")
        .unwrap()
        .values
        .clear();
    assert!(validate(&command, &nlu).is_err(), "开放前槽不能相邻猜切分");
}

#[test]
fn malformed_or_undeclared_template_slots_cannot_enter_catalog() {
    for template in ["把{room的灯", "把{room}}的灯", "把{missing}的灯打开"] {
        let (command, mut nlu) = fixture();
        nlu.intents[0].templates = vec![template.into()];
        assert!(
            validate(&command, &nlu).is_err(),
            "bad template: {template}"
        );
    }
}
