//! 模型门禁与目录安全契约。注入分类器只验证接线，不代表真实ONNX质量验收。
use super::*;
use crate::plugins::nlu::{ActionDef, BertHit, IntentDef, SlotList};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Classifier {
    label: &'static str,
    score: f32,
    calls: Arc<AtomicUsize>,
    fail: bool,
}
impl BertClassify for Classifier {
    fn classify_timed(&self, _: &str) -> Result<BertHit> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            bail!("injected inference failure");
        }
        Ok(BertHit {
            label: self.label.into(),
            confidence: self.score,
            tokenize_ms: 0,
            infer_ms: 0,
        })
    }
}
fn fixture() -> (CommandConfig, NluConfig) {
    let nlu = NluConfig {
        bert_enabled: true,
        intents: vec![
            IntentDef {
                name: "control_light".into(),
                templates: vec!["把{room}的灯{action}".into()],
                action: ActionDef {
                    kind: "mcp:light.set".into(),
                    reply: "好的".into(),
                    ..Default::default()
                },
            },
            IntentDef {
                name: "control_ac".into(),
                templates: vec!["把{room}的空调{action}".into()],
                action: Default::default(),
            },
            IntentDef {
                name: "help".into(),
                templates: vec!["打开使用指南".into()],
                action: ActionDef {
                    kind: "reply".into(),
                    reply: "这里是指南".into(),
                    ..Default::default()
                },
            },
        ],
        slots: vec![
            SlotList {
                name: "room".into(),
                values: vec!["客厅".into(), "卧室".into()],
                optional: false,
            },
            SlotList {
                name: "action".into(),
                values: vec!["打开".into(), "关闭".into()],
                optional: false,
            },
        ],
        ..Default::default()
    };
    (CommandConfig::default(), nlu)
}
fn make_registry(label: &'static str, score: f32) -> (SkillRegistry, Arc<AtomicUsize>) {
    let (command, nlu) = fixture();
    registry_with(&command, &nlu, label, score, false)
}
fn registry_with(
    command: &CommandConfig,
    nlu: &NluConfig,
    label: &'static str,
    score: f32,
    fail: bool,
) -> (SkillRegistry, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let classifier = Classifier {
        label,
        score,
        calls: calls.clone(),
        fail,
    };
    (
        SkillRegistry::new(command, nlu, Some(Arc::new(classifier))),
        calls,
    )
}
fn slots(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[test]
fn model_is_called_once_even_when_command_or_template_looks_exact() {
    let (registry, calls) = make_registry("end_session", 0.99);
    let Recognition::Accepted { inv, .. } = registry.recognize("退下") else {
        panic!("expected model acceptance");
    };
    assert_eq!(inv.action, SkillAction::EndSession);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let (registry, calls) = make_registry("control_light", 0.99);
    let Recognition::Accepted { inv, .. } = registry.recognize("把客厅的灯打开") else {
        panic!("expected same-skill extraction");
    };
    assert_eq!(inv.slots, slots(&[("room", "客厅"), ("action", "打开")]));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn disabled_model_never_uses_injected_classifier_or_keyword_rules() {
    let (command, mut nlu) = fixture();
    nlu.bert_enabled = false;
    let (registry, calls) = registry_with(&command, &nlu, "end_session", 1.0, false);
    assert_eq!(registry.recognize("退下"), Recognition::Disabled);
    assert_eq!(registry.recognize("把客厅的灯打开"), Recognition::Disabled);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(registry.tools().is_empty());
}

#[test]
fn enabled_missing_or_broken_model_fails_closed_instead_of_rules() {
    let (command, nlu) = fixture();
    let missing = SkillRegistry::new(&command, &nlu, None);
    assert!(matches!(
        missing.recognize("退下"),
        Recognition::Unavailable { .. }
    ));
    assert!(missing.try_parse_staged("退下").is_err());
    let (broken, calls) = registry_with(&command, &nlu, "end_session", 1.0, true);
    assert!(matches!(
        broken.recognize("退下"),
        Recognition::Unavailable { .. }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn other_and_low_score_cannot_be_overridden_by_templates_or_keywords() {
    for (label, score) in [
        ("other", 0.99),
        ("control_light", 0.5),
        ("end_session", 0.5),
    ] {
        let (registry, _) = make_registry(label, score);
        for text in ["退下", "把客厅的灯打开", "帮我把大客厅的灯打开"] {
            assert_eq!(registry.recognize(text), Recognition::Rejected);
        }
    }
}

#[test]
fn classifier_selects_intent_and_template_cannot_switch_to_another() {
    let (registry, _) = make_registry("control_ac", 0.99);
    assert!(
        matches!(registry.recognize("把客厅的灯打开"), Recognition::NeedsSlots { name, .. } if name == "control_ac")
    );
    let (registry, _) = make_registry("help", 0.99);
    let Recognition::Accepted { inv, .. } = registry.recognize("与指南训练模板完全不同的表达")
    else {
        panic!("slotless needs no literal gate");
    };
    assert_eq!(inv.name, "help");
}

#[test]
fn missing_ambiguous_or_invalid_slots_only_request_clarification() {
    let (registry, _) = make_registry("control_light", 0.99);
    for text in [
        "帮我把大客厅的灯打开",
        "把厨房的灯打开",
        "把客厅的灯别打开",
        "把客厅的灯打开还是关闭",
        "打开灯",
    ] {
        assert!(
            matches!(registry.recognize(text), Recognition::NeedsSlots { name, missing, .. } if name == "control_light" && missing == ["action", "room"])
        );
    }
}

#[test]
fn unknown_labels_and_nonfinite_scores_are_strict_errors() {
    let (registry, _) = make_registry("not_registered", 0.99);
    assert!(matches!(
        registry.recognize("退下"),
        Recognition::Unavailable { .. }
    ));
    for score in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        let (registry, _) = make_registry("end_session", score);
        assert!(matches!(
            registry.recognize("退下"),
            Recognition::Unavailable { .. }
        ));
    }
}

#[test]
fn known_disabled_labels_reject_instead_of_failing_or_executing() {
    let (mut command, mut nlu) = fixture();
    nlu.enabled = false;
    let (registry, _) = registry_with(&command, &nlu, "control_light", 0.99, false);
    assert_eq!(registry.recognize("把客厅的灯打开"), Recognition::Rejected);
    nlu.enabled = true;
    command.enabled = false;
    let (registry, _) = registry_with(&command, &nlu, "end_session", 0.99, false);
    assert_eq!(registry.recognize("退下"), Recognition::Rejected);
}

#[test]
fn all_catalogs_disabled_skip_classifier_entirely() {
    let (mut command, mut nlu) = fixture();
    command.enabled = false;
    nlu.enabled = false;
    let (registry, calls) = registry_with(&command, &nlu, "end_session", 1.0, true);
    assert_eq!(registry.recognize("退下"), Recognition::Disabled);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn llm_has_no_skill_catalog_or_action_backchannel() {
    let (registry, _) = make_registry("other", 1.0);
    assert!(registry.tools().is_empty());
    let call = ToolCall {
        id: "1".into(),
        name: TOOL_NAME.into(),
        arguments: serde_json::json!({"skill":"end_session","slots":{}}).to_string(),
    };
    assert!(registry.from_tool_call(&call).is_err());
}

#[test]
fn deprecated_thresholds_cannot_enable_heuristic_fallback() {
    let (command, mut nlu) = fixture();
    nlu.model_threshold = 0.0;
    nlu.model_margin = 0.0;
    let (registry, _) = registry_with(&command, &nlu, "other", 0.99, false);
    assert_eq!(registry.recognize("把客厅的灯打开"), Recognition::Rejected);
}

#[test]
#[allow(deprecated)] // Deliberately exercise the diagnostic-only compatibility wrapper.
fn legacy_parser_wrapper_is_still_model_only() {
    let (registry, _) = make_registry("control_light", 0.99);
    let (stage, inv) = registry
        .try_parse_staged("把客厅的灯打开")
        .unwrap()
        .unwrap();
    assert_eq!(stage, SkillStage::Model);
    assert_eq!(inv.name, "control_light");
    let (registry, _) = make_registry("other", 0.99);
    assert!(registry.parse_staged("退下").is_none());
}

#[test]
fn resolve_enforces_same_skill_schema_values_and_size() {
    let (registry, _) = make_registry("control_light", 0.99);
    assert!(registry.resolve("nope", HashMap::new()).is_err());
    assert!(registry.resolve("control_light", HashMap::new()).is_err());
    assert!(registry
        .resolve(
            "control_light",
            slots(&[("room", "厨房"), ("action", "打开")])
        )
        .is_err());
    assert!(registry
        .resolve(
            "control_light",
            slots(&[("room", "客厅"), ("action", "打开"), ("extra", "x")])
        )
        .is_err());
    let inv = registry
        .resolve(
            "control_light",
            slots(&[("room", "客 厅"), ("action", "打 开")]),
        )
        .unwrap();
    assert_eq!(inv.slots["room"], "客厅");
}

#[test]
fn accepted_default_plan_keeps_raw_slot_time_and_uses_real_task_api() {
    let command = CommandConfig::default();
    let nlu = NluConfig {
        bert_enabled: true,
        ..Default::default()
    };
    let (registry, _) = registry_with(&command, &nlu, "reminder_daily", 0.99, false);
    let Recognition::Accepted { inv, .. } = registry.recognize("每天08:30提醒我喝水") else {
        panic!("expected daily slots");
    };
    assert_eq!(inv.slots["daily_time"], "08:30");
    let SkillAction::Plan { plan } = inv.action else {
        panic!("initial reminder must be a real plan");
    };
    assert_eq!(plan.steps[0].api, "tasks.create");
    let resolved = crate::skills::resolve_plan(&plan, &inv.slots).unwrap();
    assert_eq!(resolved.steps[0].args["daily"], "08:30");
}

#[test]
fn registry_snapshot_does_not_gain_new_saved_definitions() {
    let (command, mut nlu) = fixture();
    let old = SkillRegistry::new(&command, &nlu, None);
    nlu.intents.push(IntentDef {
        name: "new_help".into(),
        templates: vec!["新指南".into()],
        ..Default::default()
    });
    let (new, _) = registry_with(&command, &nlu, "new_help", 0.99, false);
    assert!(matches!(
        new.recognize("新的表达"),
        Recognition::Accepted { .. }
    ));
    assert!(old.resolve("new_help", HashMap::new()).is_err());
}

#[path = "catalog_tests.rs"]
mod catalog_tests;
#[path = "clock_tests.rs"]
mod clock_tests;
