//! 配置种子与已接受同技能抽槽测试，不测试或提供任何模板意图识别器。
use super::*;
use matcher::match_template;

#[test]
fn initial_skill_json_is_visible_editable_configuration() {
    let config = NluConfig::default();
    assert_eq!(config.intents.len(), 11);
    assert!(config.enabled && !config.bert_enabled);
    assert_eq!(config.bert_threshold, 0.75);
    for intent in &config.intents {
        assert_eq!(intent.action.kind, "plan");
        assert!(intent.action.reply.is_empty());
        assert!(
            intent.templates.len() >= 2,
            "each class needs distinct split examples"
        );
        let plan = intent.action.plan.as_ref().unwrap();
        crate::skills::validate_plan(plan).unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[1].api, "replies.say");
        assert_eq!(plan.steps[1].args["text"], "{{steps.0.speech}}");
        if plan.steps[0].api == "tasks.create" {
            assert_eq!(plan.steps[0].args["timezone"], "Asia/Shanghai");
        }
    }
    assert_eq!(
        config
            .intents
            .iter()
            .find(|i| i.name == "reminder_daily")
            .unwrap()
            .templates[0],
        "每天{daily_time}提醒我{task}"
    );
}

#[test]
fn omitted_fields_seed_defaults_but_explicit_empty_stays_empty() {
    let omitted: NluConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(omitted.intents.len(), 11);
    let cleared: NluConfig = serde_json::from_str(r#"{"intents":[],"slots":[]}"#).unwrap();
    assert!(cleared.intents.is_empty() && cleared.slots.is_empty());
    assert!(cleared.with_builtins().intents.is_empty());
}

#[test]
fn saved_legacy_reply_does_not_become_a_real_task() {
    let saved: NluConfig = serde_json::from_value(serde_json::json!({
        "intents":[{"name":"reminder_countdown","templates":["{delay}后提醒我{task}"],"action":{"kind":"reply","reply":"旧应答"}}]
    })).unwrap();
    let effective = saved.with_builtins();
    assert_eq!(effective.intents.len(), 1);
    assert_eq!(effective.intents[0].action.kind, "reply");
    assert!(effective.intents[0].action.plan.is_none());
}

#[test]
fn accepted_skill_extraction_preserves_clock_and_raw_payload() {
    let template = parse_template("每天{daily_time}提醒我{task}");
    let slots = match_template(
        "每天08:30提醒我喝水。",
        &template,
        &HashMap::new(),
        &HashMap::new(),
    )
    .unwrap();
    assert_eq!(slots["daily_time"], "08:30");
    assert_eq!(slots["task"], "喝水");
    let slots = match_template(
        "每天08:30提醒我发送 A:B",
        &template,
        &HashMap::new(),
        &HashMap::new(),
    )
    .unwrap();
    assert_eq!(slots["task"], "发送 A:B");
}

#[test]
fn extraction_requires_exact_boundaries_and_all_required_slots() {
    let template = parse_template("把{room}的灯{action}");
    let values = HashMap::from([
        ("room".into(), vec!["客厅".into()]),
        ("action".into(), vec!["打开".into(), "关闭".into()]),
    ]);
    for text in [
        "请把客厅的灯打开",
        "把大客厅的灯打开",
        "把客厅的灯打开谢谢",
        "把客厅的灯别打开",
        "把客厅的灯打开还是关闭",
        "把客厅的灯",
    ] {
        assert!(
            match_template(text, &template, &values, &HashMap::new()).is_none(),
            "不猜测槽位：{text}"
        );
    }
    assert!(match_template("把客厅的灯打开", &template, &values, &HashMap::new()).is_some());
}

#[test]
fn ambiguous_anchor_or_adjacent_slots_cannot_be_guessed() {
    let template = parse_template("{time}提醒我{task}");
    assert!(match_template(
        "八点提醒我告诉他提醒我喝水",
        &template,
        &HashMap::new(),
        &HashMap::new()
    )
    .is_none());
    assert!(match_template(
        "八点喝水",
        &parse_template("{time}{task}"),
        &HashMap::new(),
        &HashMap::new()
    )
    .is_none());
}

#[test]
fn optional_slot_can_be_empty_without_inventing_a_value() {
    let optional = HashMap::from([("singer".into(), true)]);
    let slots = match_template(
        "播放的歌",
        &parse_template("播放{singer}的歌"),
        &HashMap::new(),
        &optional,
    )
    .unwrap();
    assert_eq!(slots["singer"], "");
}

#[test]
fn adjacent_slots_require_a_unique_closed_prefix_not_numeric_token_splitting() {
    let template = parse_template("每周{weekday}{daily_time}提醒我{task}");
    let values = HashMap::from([("weekday".into(), vec!["一".into(), "1".into()])]);
    let extracted =
        match_template("每周一10点提醒我喝水", &template, &values, &HashMap::new()).unwrap();
    assert_eq!(extracted["weekday"], "一");
    assert_eq!(extracted["daily_time"], "10点");
    for text in ["每周10点提醒我喝水", "每周110点提醒我喝水"] {
        assert!(match_template(text, &template, &values, &HashMap::new()).is_none());
    }
    let ambiguous = HashMap::from([("weekday".into(), vec!["一".into(), "一十".into()])]);
    assert!(match_template(
        "每周一十点提醒我喝水",
        &template,
        &ambiguous,
        &HashMap::new()
    )
    .is_none());
    let optional = HashMap::from([("weekday".into(), true)]);
    assert!(match_template("每周一10点提醒我喝水", &template, &values, &optional).is_none());
}

#[test]
fn template_parser_only_describes_slots_and_literals() {
    assert_eq!(
        parse_template("把{room}的灯{action}").tokens,
        vec![
            template::Tok::Lit("把".into()),
            template::Tok::Slot("room".into()),
            template::Tok::Lit("的灯".into()),
            template::Tok::Slot("action".into())
        ]
    );
}
