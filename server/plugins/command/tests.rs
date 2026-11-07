//! command只提供保存态训练数据，不得独立识别意图。
use super::*;

#[test]
fn initial_json_defaults_are_enabled_but_never_a_rule_gate() {
    let config = CommandConfig::default();
    assert!(config.enabled && config.active());
    assert_eq!(config.keywords, ["退下", "闭嘴", "关闭"]);
    assert!(!config.reply.is_empty());
    let gate = CommandGate::new(&config);
    for text in ["退下", "退下吧。", "闭嘴", "关闭闹钟", "随便说什么"] {
        assert!(
            gate.hit(text).is_none(),
            "旧入口不得在无模型时产生授权：{text}"
        );
    }
}

#[test]
fn contains_and_max_chars_are_inert_compatibility_fields() {
    let config = CommandConfig {
        match_mode: MatchMode::Contains,
        max_chars: 0,
        ..Default::default()
    };
    assert!(CommandGate::new(&config).hit("请把灯关闭").is_none());
    let long = CommandConfig {
        keywords: vec!["请结束这一次会话并返回待唤醒状态".into()],
        max_chars: 1,
        ..config
    };
    long.validate().unwrap();
}

#[test]
fn saved_training_examples_keep_particles_and_punctuation() {
    let mut config = CommandConfig {
        keywords: vec![
            " 退下吧。 ".into(),
            "退下吧。".into(),
            "别说了".into(),
            "".into(),
        ],
        ..Default::default()
    };
    config.normalize();
    assert_eq!(config.keywords, ["退下吧。", "别说了"]);
    let once = config.signature();
    config.normalize();
    assert_eq!(config.signature(), once);
}

#[test]
fn empty_training_catalog_is_actionable_error_not_match_all() {
    let mut config = CommandConfig {
        keywords: vec![" ".into()],
        ..Default::default()
    };
    assert!(!config.active());
    assert!(config
        .validate()
        .unwrap_err()
        .to_string()
        .contains("keywords"));
    config.enabled = false;
    config.validate().unwrap();
    assert!(CommandGate::new(&config).hit("退下").is_none());
}

#[test]
fn command_samples_cannot_inject_slot_templates() {
    let config = CommandConfig {
        keywords: vec!["退下{room}".into()],
        ..Default::default()
    };
    assert!(config.validate().is_err());
}

#[test]
fn signatures_cover_effective_configuration_not_deprecated_rules() {
    let mut config = CommandConfig::default();
    let initial = config.signature();
    config.max_chars = 0;
    config.match_mode = MatchMode::Contains;
    assert_eq!(config.signature(), initial);
    config.reply.push('!');
    assert_ne!(config.signature(), initial);
}

#[test]
fn legacy_serde_keys_are_still_readable() {
    let config: CommandConfig =
        serde_json::from_value(serde_json::json!({"match_mode":"contains","max_chars":42}))
            .unwrap();
    assert_eq!(config.match_mode, MatchMode::Contains);
    assert_eq!(config.max_chars, 42);
    assert_eq!(config.keywords, CommandConfig::default().keywords);
}

#[test]
fn normalizer_is_only_a_value_comparison_helper() {
    assert_eq!(normalize_utterance(" 客 厅。 "), "客厅");
    assert_eq!(normalize_utterance("退下吧。"), "退下");
}
