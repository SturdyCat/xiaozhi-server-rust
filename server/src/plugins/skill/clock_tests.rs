//! 默认时钟技能只验证模型授权后的抽槽/计划，不代表真实分类器准确率。
use super::*;

fn accepted_clock(label: &'static str, text: &str) -> SkillInvocation {
    let nlu = NluConfig {
        bert_enabled: true,
        ..Default::default()
    };
    let (registry, calls) = registry_with(&CommandConfig::default(), &nlu, label, 0.99, false);
    let Recognition::Accepted { inv, confidence } = registry.recognize(text) else {
        panic!("expected same-skill clock extraction: {label} / {text}");
    };
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(confidence, 0.99);
    assert_eq!(inv.name, label);
    assert!(
        inv.reply.is_empty(),
        "only the real service result is spoken"
    );
    let SkillAction::Plan { plan } = &inv.action else {
        panic!("clock seed must use stable APIs");
    };
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.steps[1].api, "replies.say");
    assert_eq!(plan.steps[1].args["text"], "{{steps.0.speech}}");
    inv
}

#[test]
fn weekly_chinese_and_arabic_with_or_without_separator_preserve_raw_time() {
    for (text, weekday, time) in [
        ("每周1的10点提醒我喝水", "1", "10点"),
        ("每周一的10点提醒我喝水", "一", "10点"),
        ("每周一10点提醒我喝水", "一", "10点"),
        ("每周1晚上十点提醒我喝水", "1", "晚上十点"),
        ("每星期日的下午八点半提醒我喝水", "日", "下午八点半"),
        ("每周天早上八点提醒我喝水", "天", "早上八点"),
    ] {
        let inv = accepted_clock("reminder_weekly", text);
        assert_eq!(inv.slots["weekday"], weekday);
        assert_eq!(inv.slots["daily_time"], time);
        assert_eq!(inv.slots["task"], "喝水");
    }
}

#[test]
fn clock_variants_do_not_strip_period_date_or_chinese_numeric_slots() {
    for (label, text, expected) in [
        (
            "reminder_countdown",
            "十分钟后提醒我喝水",
            vec![("delay", "十分钟")],
        ),
        (
            "reminder_schedule",
            "在大后天晚上十点提醒我喝水",
            vec![("time", "大后天晚上十点")],
        ),
        (
            "reminder_schedule",
            "明天下午三点提醒我喝水",
            vec![("time", "明天下午三点")],
        ),
        (
            "reminder_repeat",
            "每隔两小时提醒我喝水",
            vec![("interval", "两小时")],
        ),
        (
            "reminder_daily",
            "每天下午十点提醒我喝水",
            vec![("daily_time", "下午十点")],
        ),
        (
            "reminder_workdays",
            "每个工作日早上八点提醒我喝水",
            vec![("daily_time", "早上八点")],
        ),
        (
            "reminder_workdays",
            "每工作日10点提醒我喝水",
            vec![("daily_time", "10点")],
        ),
        (
            "reminder_workdays",
            "工作日晚上八点半提醒我喝水",
            vec![("daily_time", "晚上八点半")],
        ),
        (
            "reminder_monthly",
            "每月31日晚上十点提醒我喝水",
            vec![("day", "31"), ("daily_time", "晚上十点")],
        ),
        (
            "reminder_yearly",
            "每年二月二十九日早上八点提醒我喝水",
            vec![
                ("month", "二"),
                ("day", "二十九"),
                ("daily_time", "早上八点"),
            ],
        ),
        (
            "reminder_hourly",
            "每小时的20分提醒我喝水",
            vec![("minute", "20")],
        ),
        (
            "reminder_hourly",
            "每小时二十分提醒我喝水",
            vec![("minute", "二十")],
        ),
    ] {
        let inv = accepted_clock(label, text);
        for (key, value) in expected {
            assert_eq!(inv.slots[key], value, "{label} / {text} / {key}");
        }
    }
}

#[test]
fn list_and_cancel_remain_configured_capabilities_not_new_recognition_branches() {
    let inv = accepted_clock("reminder_list", "模型已接受的查询表达，无需再匹配字面词");
    assert!(inv.slots.is_empty());
    let SkillAction::Plan { plan } = inv.action else {
        unreachable!()
    };
    assert_eq!(plan.steps[0].api, "tasks.list");
    assert_eq!(plan.steps[0].args, serde_json::json!({}));
    for text in ["取消提醒喝水", "取消喝水的提醒"] {
        let inv = accepted_clock("reminder_cancel", text);
        assert_eq!(inv.slots["task"], "喝水");
        let SkillAction::Plan { plan } = inv.action else {
            unreachable!()
        };
        assert_eq!(plan.steps[0].api, "tasks.cancel");
        assert_eq!(
            plan.steps[0].args,
            serde_json::json!({"text":"{{slots.task}}"})
        );
    }
}

#[test]
fn clock_looking_text_cannot_override_rejection_or_switch_accepted_intent() {
    let nlu = NluConfig {
        bert_enabled: true,
        ..Default::default()
    };
    let command = CommandConfig::default();
    let (registry, _) = registry_with(&command, &nlu, "other", 0.99, false);
    assert_eq!(
        registry.recognize("每周一10点提醒我喝水"),
        Recognition::Rejected
    );
    let (registry, _) = registry_with(&command, &nlu, "reminder_yearly", 0.99, false);
    assert!(
        matches!(registry.recognize("每周一10点提醒我喝水"), Recognition::NeedsSlots { name, .. } if name == "reminder_yearly")
    );
    let (registry, _) = registry_with(&command, &nlu, "reminder_weekly", 0.99, false);
    for text in ["每周八10点提醒我喝水", "每周10点提醒我喝水"] {
        assert!(
            matches!(registry.recognize(text), Recognition::NeedsSlots { name, .. } if name == "reminder_weekly")
        );
    }
}

#[test]
fn equally_specific_conflicting_slot_boundaries_still_clarify() {
    let nlu = NluConfig {
        bert_enabled: true,
        intents: vec![IntentDef {
            name: "ambiguous".into(),
            templates: vec!["{time}a{task}".into(), "{time}b{task}".into()],
            action: ActionDef::default(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let (registry, _) = registry_with(&CommandConfig::default(), &nlu, "ambiguous", 0.99, false);
    assert!(matches!(
        registry.recognize("早ab晚"),
        Recognition::NeedsSlots { .. }
    ));
}
