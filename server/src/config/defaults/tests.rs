//! `config/defaults.rs` 的单元测试：三条对齐规则 + 与代码默认值的一致性护栏。

use serde_json::{json, Value};

use super::*;
use crate::config::Config;

/// 把一份"当前默认配置"改造成**旧默认**形状（每条变更路径都写回 `from`），
/// 模拟历史 bootstrap 落库的整份文档。
fn legacy_doc() -> Value {
    let mut doc = serde_json::to_value(Config::default()).expect("序列化默认配置");
    for change in STANDARD_BEHAVIOR_ON {
        set_path(&mut doc, change.path, change.from.to_json());
    }
    doc
}

fn set_path(doc: &mut Value, path: &str, v: Value) {
    let mut cur = doc;
    let segs: Vec<&str> = path.split('.').collect();
    for seg in &segs[..segs.len() - 1] {
        cur = cur
            .as_object_mut()
            .expect("对象")
            .entry((*seg).to_string())
            .or_insert_with(|| json!({}));
    }
    cur.as_object_mut()
        .expect("对象")
        .insert(segs[segs.len() - 1].to_string(), v);
}

fn get_path<'a>(doc: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = doc;
    for seg in path.split('.') {
        cur = cur.as_object()?.get(seg)?;
    }
    Some(cur)
}

/// 旧库（整份文档、全部标准行为为 false）→ 全部对齐为 true。
#[test]
fn legacy_doc_aligns_all_standard_switches() {
    let mut doc = legacy_doc();
    let report = align(&mut doc, 0);
    assert_eq!(report.from_version, 0);
    assert_eq!(report.to_version, CURRENT_DEFAULTS_VERSION);
    assert_eq!(
        report.changed.len(),
        STANDARD_BEHAVIOR_ON.len(),
        "每条声明都必须生效，实际: {}",
        report.summary()
    );
    assert!(report.filled.is_empty());
    assert!(report.kept.is_empty());
    assert!(report.broken.is_empty());
    for change in STANDARD_BEHAVIOR_ON {
        assert_eq!(
            get_path(&doc, change.path),
            Some(&change.to.to_json()),
            "{} 未对齐",
            change.path
        );
    }
    // 报告必须能被人读懂（部署者据此确认"哪些默认值被打开"）
    let summary = report.summary();
    assert!(summary.contains("wakeup.enabled"), "{summary}");
    assert!(summary.contains("我在"), "{summary}");
}

/// 用户显式改过的值（≠ 旧默认）必须原样保留。
#[test]
fn user_customized_values_are_kept() {
    let mut doc = legacy_doc();
    // 用户当年手动打开了人格，又手动改了问候语文案
    set_path(&mut doc, "soul.enabled", json!(true));
    set_path(&mut doc, "wakeup.greeting", json!("在呢"));
    let report = align(&mut doc, 0);
    assert!(
        report.kept.contains(&"soul.enabled".to_string()),
        "{}",
        report.summary()
    );
    assert_eq!(get_path(&doc, "soul.enabled"), Some(&json!(true)));
    // 文案不是开关，本来就不在变更表里，必须一字不动
    assert_eq!(get_path(&doc, "wakeup.greeting"), Some(&json!("在呢")));
    // 其余没动过的照常对齐
    assert_eq!(get_path(&doc, "wakeup.enabled"), Some(&json!(true)));
    assert_eq!(
        report.changed.len(),
        STANDARD_BEHAVIOR_ON.len() - 1,
        "只应少一项（soul）: {}",
        report.summary()
    );
}

/// 键缺失 / 显式 null → 补成新默认（不缺段、不报错）。
#[test]
fn missing_and_null_keys_are_filled() {
    let mut doc = json!({
        "wakeup": { "greeting": "我在" },
        "memory": Value::Null,
        "command": { "enabled": Value::Null },
    });
    let report = align(&mut doc, 0);
    assert_eq!(
        report.filled.len(),
        STANDARD_BEHAVIOR_ON.len(),
        "{}",
        report.summary()
    );
    assert!(report.changed.is_empty());
    for change in STANDARD_BEHAVIOR_ON {
        assert_eq!(
            get_path(&doc, change.path),
            Some(&change.to.to_json()),
            "{} 未补齐",
            change.path
        );
    }
    // 用户已有的字段不被覆盖
    assert_eq!(get_path(&doc, "wakeup.greeting"), Some(&json!("我在")));
}

/// 空对象（等价于"什么都没配"）→ 全部补齐，且段对象被就地创建。
#[test]
fn empty_doc_fills_everything() {
    let mut doc = json!({});
    let report = align(&mut doc, 0);
    assert_eq!(report.filled.len(), STANDARD_BEHAVIOR_ON.len());
    assert_eq!(get_path(&doc, "wakeup.enabled"), Some(&json!(true)));
    assert_eq!(get_path(&doc, "memory.recall_enabled"), Some(&json!(true)));
}

/// 已经是当前版本 → 幂等空操作（不写库、不改文档）。
#[test]
fn current_version_is_noop_and_idempotent() {
    let mut doc = legacy_doc();
    let _ = align(&mut doc, 0);
    let snapshot = doc.clone();
    let report = align(&mut doc, CURRENT_DEFAULTS_VERSION);
    assert!(report.is_noop(), "{}", report.summary());
    assert_eq!(doc, snapshot, "当前版本不得再改文档");
    // 再跑一次旧版本对齐也不会二次变更（值已是新默认，不再等于旧默认）
    let again = align(&mut doc, 0);
    assert!(again.changed.is_empty(), "{}", again.summary());
    assert_eq!(again.kept.len(), STANDARD_BEHAVIOR_ON.len());
}

/// 段结构损坏（不是对象）→ 跳过并记录，不把整份配置改烂。
#[test]
fn broken_section_is_skipped() {
    let mut doc = json!({ "wakeup": 5, "command": { "enabled": false } });
    let report = align(&mut doc, 0);
    assert!(report.broken.contains(&"wakeup.enabled".to_string()));
    assert_eq!(get_path(&doc, "wakeup"), Some(&json!(5)), "坏段不得被覆盖");
    assert_eq!(get_path(&doc, "command.enabled"), Some(&json!(true)));
}

/// 护栏：变更表里的 `to` 必须等于**当前代码默认值**（防止改了默认值忘了登记，
/// 也防止登记了一个与代码不符的值）。
#[test]
fn targets_match_code_defaults() {
    assert_eq!(
        BATCHES.last().map(|b| b.version),
        Some(CURRENT_DEFAULTS_VERSION),
        "CURRENT_DEFAULTS_VERSION 必须与最后一批一致"
    );
    let defaults = serde_json::to_value(Config::default()).expect("序列化默认配置");
    for batch in BATCHES {
        for change in batch.changes {
            assert_ne!(
                change.from, change.to,
                "{} 的 from/to 相同，等于没变更",
                change.path
            );
            assert_eq!(
                get_path(&defaults, change.path),
                Some(&change.to.to_json()),
                "{} 登记的新默认值 != 代码默认值",
                change.path
            );
        }
    }
}

/// 护栏：标准行为默认必须全部是"开"，且覆盖范围含唤醒问候语（用户诉求的锚点）。
#[test]
fn standard_behaviors_default_on() {
    let defaults = serde_json::to_value(Config::default()).expect("序列化默认配置");
    let paths: Vec<&str> = STANDARD_BEHAVIOR_ON.iter().map(|c| c.path).collect();
    for must in [
        "wakeup.enabled",
        "command.enabled",
        "nlu.enabled",
        "soul.enabled",
        "memory.enabled",
        "echomuse.enabled",
        "llm.enabled",
    ] {
        assert!(paths.contains(&must), "标准行为 {must} 未纳入默认开启清单");
        assert_eq!(
            get_path(&defaults, must),
            Some(&json!(true)),
            "{must} 代码默认值必须为 true"
        );
    }
    // 需要凭据/模型文件的开关不在其内：缺了就启动失败，不能当"开箱即用"默认
    assert!(!paths.contains(&"aiui.enabled"));
    assert!(!paths.contains(&"nlu.bert_enabled"));
}
