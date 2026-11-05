//! `predict.rs` 的四条分支与判定顺序（纯函数，不需要 ONNX）。
//!
//! 这些用例是"门禁 / `/test` / 对比三处同源"的可执行定义：谁要是再写一份判定，
//! 这几条会在语义分叉的那一侧变红。

use serde_json::{json, Value};

use crate::plugins::nlu::{BertHit, NluConfig};

use super::*;

/// 两个技能 + `other` 的候选。`map` 用来造"旧 manifest 留下非空 label_map"的形态。
fn artifact(map: Value) -> Artifact {
    let nlu: NluConfig = serde_json::from_value(json!({
        "enabled": true,
        "intents": [
            {"name": "reminder_countdown", "templates": ["倒计时"], "action": {"kind": "reply", "reply": "好"}},
            {"name": "reminder_schedule", "templates": ["定时"], "action": {"kind": "reply", "reply": "好"}}
        ],
        "slots": [],
        "bert_threshold": 0.5,
        "bert_label_map": map
    }))
    .unwrap();
    Artifact {
        build_id: uuid::Uuid::new_v4().to_string(),
        source_revision: "saved".into(),
        source_input_sha256: "input-hash".into(),
        trainer_image: "dependency-image".into(),
        trainer_image_id: "image-id".into(),
        trainer_script_sha256: "script-hash".into(),
        labels: vec![
            "reminder_countdown".into(),
            "reminder_schedule".into(),
            "other".into(),
        ],
        catalog_source: None,
        base_model: None,
        created_at: None,
        files: vec![],
        nlu,
    }
}

fn hit(label: &str, confidence: f32) -> BertHit {
    BertHit {
        label: label.into(),
        confidence,
        tokenize_ms: 3,
        infer_ms: 7,
    }
}

#[test]
fn other_wins_over_threshold_even_at_zero_threshold() {
    let mut a = artifact(json!({}));
    a.nlu.bert_threshold = 0.0;
    let v = verdict(&a, &hit("other", 0.99)).unwrap();
    assert_eq!(v.reason, Reason::Other);
    assert_eq!(v.intent(), None, "other 是保留标签，永远不是技能");
    assert_eq!(v.mapped, "other", "拒识也要保留映射结果给界面");
    assert_eq!(v.confidence, 0.99);
}

#[test]
fn a_label_mapped_to_other_is_a_rejection() {
    // 旧 manifest 的非空 label_map：某个历史标签现在应当落到保留标签 other。
    let a = artifact(json!({"legacy_reminder": "other"}));
    let mut a = a;
    a.labels.push("legacy_reminder".into());
    let v = verdict(&a, &hit("legacy_reminder", 0.99)).unwrap();
    assert_eq!(
        v.reason,
        Reason::Other,
        "映射到 other 与直接输出 other 同义，必须拒识"
    );
    assert_eq!(v.label, "legacy_reminder");
    assert_eq!(v.intent(), None);
}

#[test]
fn a_label_mapped_to_a_missing_skill_is_unmapped_not_accepted() {
    // 这条是旧 `smoke::predict` 的真 bug：它只看原始 label，
    // 于是把"目录里根本没有 ghost"判成 accepted，路由到不存在的技能。
    let mut a = artifact(json!({"legacy_reminder": "ghost"}));
    a.labels.push("legacy_reminder".into());
    let v = verdict(&a, &hit("legacy_reminder", 0.99)).unwrap();
    assert_eq!(v.reason, Reason::UnmappedSkill);
    assert_eq!(v.mapped, "ghost", "上报要说清映射到哪个不存在的技能");
    assert_eq!(v.intent(), None);
}

#[test]
fn threshold_boundary_is_accepted_only_when_not_below() {
    let a = artifact(json!({}));
    // 判据是 `confidence < threshold`：等于阈值算**接受**（不许写成 `<=`）。
    let equal = verdict(&a, &hit("reminder_countdown", 0.5)).unwrap();
    assert_eq!(equal.reason, Reason::Accepted);
    assert_eq!(equal.intent(), Some("reminder_countdown"));

    let below = verdict(&a, &hit("reminder_countdown", 0.499)).unwrap();
    assert_eq!(below.reason, Reason::BelowThreshold);
    assert_eq!(below.intent(), None);
    assert_eq!(
        below.mapped, "reminder_countdown",
        "低于阈值也要给出它以为的技能"
    );
}

#[test]
fn invalid_confidence_and_output_outside_labels_are_errors_not_rejections() {
    let a = artifact(json!({}));
    // 越界/非法置信度是"模型坏了"，不能算拒识，否则门禁会把坏模型当成"保守"。
    assert!(verdict(&a, &hit("reminder_countdown", f32::NAN)).is_err());
    assert!(verdict(&a, &hit("reminder_countdown", 1.5)).is_err());
    assert!(verdict(&a, &hit("reminder_countdown", -0.1)).is_err());
    // 输出不在候选 labels 内：同样是错误（标签文件与模型不匹配）。
    assert!(verdict(&a, &hit("injected", 0.99)).is_err());
}

#[test]
fn reason_codes_are_the_wire_contract() {
    // `code()` 进 JSON，`as_str()` 进 state.json；改名等于改接口，必须显式改这里。
    assert_eq!(Reason::Accepted.code(), None);
    assert_eq!(Reason::Other.code(), Some("other"));
    assert_eq!(Reason::BelowThreshold.code(), Some("below_threshold"));
    assert_eq!(Reason::UnmappedSkill.code(), Some("unmapped_skill"));
    assert_eq!(Reason::Accepted.as_str(), "accepted");
    assert_eq!(Reason::Other.as_str(), "other");
    assert_eq!(Reason::BelowThreshold.as_str(), "below_threshold");
    assert_eq!(Reason::UnmappedSkill.as_str(), "unmapped_skill");
    assert!(Reason::Accepted.is_accepted());
    for r in [Reason::Other, Reason::BelowThreshold, Reason::UnmappedSkill] {
        assert!(!r.is_accepted(), "{r:?} 必须是拒识");
    }
}
