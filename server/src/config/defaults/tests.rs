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
        let pure_base_device = must == "echomuse.enabled" && cfg!(feature = "pure-base");
        assert_eq!(
            paths.contains(&must),
            !pure_base_device,
            "标准行为 {must} 的纯底座纳入状态不正确"
        );
        assert_eq!(
            get_path(&defaults, must),
            Some(&json!(!pure_base_device)),
            "{must} 代码默认值必须符合当前构建边界"
        );
    }
    // 需要凭据/模型文件的开关不在其内：缺了就启动失败，不能当"开箱即用"默认
    assert!(!paths.contains(&"aiui.enabled"));
    assert!(!paths.contains(&"nlu.bert_enabled"));
}

/// 结构补全：缺失 / 显式 null 的键补当前默认；用户已有值原样保留；
/// 缺键补整棵默认子树时路径记到该键为止。
#[test]
fn fill_missing_fills_absent_and_null_keys() {
    let mut doc = json!({
        "a": { "kept": 42 },
        "b": Value::Null,
    });
    let defaults = json!({
        "a": { "kept": 1, "added": { "deep": true } },
        "b": { "x": 1 },
        "c": [1, 2],
    });
    let filled = fill_missing(&mut doc, &defaults);
    assert_eq!(
        filled,
        vec!["a.added".to_string(), "b".to_string(), "c".to_string()],
        "补全清单必须可读"
    );
    assert_eq!(
        get_path(&doc, "a.kept"),
        Some(&json!(42)),
        "用户已有值不得覆盖"
    );
    assert_eq!(get_path(&doc, "a.added.deep"), Some(&json!(true)));
    assert_eq!(get_path(&doc, "b.x"), Some(&json!(1)));
    assert_eq!(get_path(&doc, "c"), Some(&json!([1, 2])));
}

/// 结构补全：幂等（补齐后再跑无事可做）；类型不匹配（默认对象 vs 库内标量）跳过不动。
#[test]
fn fill_missing_is_idempotent_and_skips_mismatches() {
    let mut doc = json!({ "sec": "库内标量（坏类型）", "ok": 1 });
    let defaults = json!({ "sec": { "x": 1 }, "ok": 2 });
    let filled = fill_missing(&mut doc, &defaults);
    assert!(filled.is_empty(), "类型不匹配不得补写: {filled:?}");
    assert_eq!(
        get_path(&doc, "sec"),
        Some(&json!("库内标量（坏类型）")),
        "类型不匹配不得覆盖，交给反序列化护栏报错"
    );
    assert_eq!(get_path(&doc, "ok"), Some(&json!(1)));
    // 已是完整形状 → 再跑无变更、文档不动
    let mut full = json!({ "sec": { "x": 9 } });
    let defaults2 = json!({ "sec": { "x": 1 } });
    assert!(fill_missing(&mut full, &defaults2).is_empty());
    assert_eq!(get_path(&full, "sec.x"), Some(&json!(9)));
}

/// 护栏：空文档经结构补全后必须与 `Config::default()` 序列化形状完全一致——
/// 这是"新配置段（如 [tuya]）在老库自动出现"的实现基准。
#[test]
fn fill_missing_from_empty_matches_code_defaults() {
    let defaults = serde_json::to_value(Config::default()).expect("序列化默认配置");
    let mut empty = json!({});
    let filled = fill_missing(&mut empty, &defaults);
    assert_eq!(empty, defaults, "空文档补全后必须与当前默认完全一致");
    assert!(
        filled.contains(&"tuya".to_string()),
        "新增配置段必须在补全清单里: {filled:?}"
    );
}
