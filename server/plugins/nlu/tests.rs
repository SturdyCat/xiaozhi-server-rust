//! 配置种子与已接受同技能抽槽测试，不测试或提供任何模板意图识别器。
use super::*;
use matcher::match_template;

#[test]
fn initial_skill_json_is_visible_editable_configuration() {
    let config = NluConfig::default();
    assert_eq!(config.intents.len(), 17);
    assert!(config.enabled && !config.bert_enabled);
    assert_eq!(config.bert_threshold, 0.75);
    for intent in &config.intents {
        assert!(
            intent.templates.len() >= 2,
            "each class needs distinct split examples"
        );
        if intent.action.kind == "plan" {
            let plan = intent.action.plan.as_ref().unwrap();
            crate::skills::validate_plan(plan).unwrap();
            assert_eq!(plan.steps.len(), 2);
            assert_eq!(plan.steps[1].api, "replies.say");
            assert_eq!(plan.steps[1].args["text"], "{{steps.0.speech}}");
            if plan.steps[0].api == "tasks.create" {
                assert_eq!(plan.steps[0].args["timezone"], "Asia/Shanghai");
            }
        } else {
            // 设备域示例技能：单工具直调（静态字面量），reply 留空走既有降级话术。
            assert!(
                intent.action.kind.starts_with("mcp:"),
                "非 plan 种子必须是 mcp:<tool> 直调：{}",
                intent.action.kind
            );
            assert!(intent.action.plan.is_none());
            assert!(intent.action.reply.is_empty());
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
fn seed_domains_follow_executor_and_device_skills_call_firmware_tools() {
    let config = NluConfig::default();
    for intent in &config.intents {
        if intent.name.starts_with("reminder_") || intent.name == "weather_query" {
            assert_eq!(
                intent.domain,
                Domain::Server,
                "{} 应归服务端域",
                intent.name
            );
        } else if intent.name == "iot_device_on" || intent.name == "iot_device_off" {
            // iot 域语音开关：服务端执行的 plan 能力组合（tuya.control），非固件直调。
            assert_eq!(intent.domain, Domain::Iot, "{} 应归 IoT 域", intent.name);
            assert_eq!(intent.action.kind, "plan");
            let plan = intent.action.plan.as_ref().unwrap();
            assert_eq!(plan.steps[0].api, "tuya.control");
            assert_eq!(plan.steps[0].args["name"], "{{slots.device_name}}");
            let action = intent.name.ends_with("_on");
            assert_eq!(
                plan.steps[0].args["action"],
                serde_json::json!(if action { "on" } else { "off" })
            );
        } else {
            assert_eq!(intent.domain, Domain::Device, "{} 应归设备域", intent.name);
            assert!(intent.action.kind.starts_with("mcp:"));
        }
    }
    let volume = config
        .intents
        .iter()
        .find(|i| i.name == "device_volume_set")
        .unwrap();
    // 槽位名 = 固件工具参数名（volume），工具名是静态字面量。
    assert_eq!(volume.action.kind, "mcp:self.audio_speaker.set_volume");
    assert_eq!(volume.templates[0], "音量调到{volume}");
    assert!(config.slots.iter().any(|s| s.name == "volume"));
    // iot 开放槽位：values 空（任意设备名），与 task/delay 同模式。
    let device_name = config
        .slots
        .iter()
        .find(|s| s.name == "device_name")
        .unwrap();
    assert!(device_name.values.is_empty() && !device_name.optional);
}

#[test]
fn saved_state_without_domain_field_defaults_to_server_and_unknown_domain_fails() {
    // 旧保存态 JSON 缺 domain 字段：必须照常反序列化，缺省为服务端域，不改写保存态。
    let legacy: IntentDef = serde_json::from_value(serde_json::json!({
        "name": "reminder_countdown",
        "templates": ["{delay}后提醒我{task}"],
        "action": {"kind": "reply", "reply": "旧应答"}
    }))
    .unwrap();
    assert_eq!(legacy.domain, Domain::Server);
    // 序列化会带上缺省 domain，保留判定（skill_api 的 default/custom 比较）依旧成立。
    assert_eq!(
        serde_json::to_value(&legacy).unwrap()["domain"],
        serde_json::json!("server")
    );
    // 枚举即白名单：非法域值直接反序列化失败，进入既有"保存态无效"链路。
    assert!(serde_json::from_value::<IntentDef>(serde_json::json!({
        "name": "x", "domain": "cloud"
    }))
    .is_err());
    assert!(serde_json::from_value::<IntentDef>(serde_json::json!({
        "name": "x", "domain": "device"
    }))
    .is_ok());
}

#[test]
fn omitted_fields_seed_defaults_but_explicit_empty_stays_empty() {
    let omitted: NluConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(omitted.intents.len(), 17);
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
