//! `plugins/memory/llm_extract.rs` 的回归测试（逻辑见 `llm_extract.rs`；分文件见 `AGENTS.md` §5.10）。
//!
//! 抽取的解析器必须容忍模型的常见失真（围栏、废话、多余键），但**不能**容忍空摘要：
//! 空的"记忆"会把噪音写进库，还不如这轮失败。

use super::*;

#[test]
fn parses_plain_json() {
    let e = parse(
        r#"{"summary":"用户决定下周三去杭州","outcome":"completed","keywords":["杭州","出差"]}"#,
    )
    .unwrap();
    assert_eq!(e.summary, "用户决定下周三去杭州");
    assert_eq!(e.outcome, "completed");
    assert_eq!(e.keywords, vec!["杭州", "出差"]);
}

#[test]
fn parses_json_inside_markdown_fence_with_surrounding_prose() {
    let raw = "好的，以下是结果：\n```json\n{\"summary\":\"用户喜欢美式咖啡\",\"outcome\":\"informational\"}\n```\n希望有帮助！";
    let e = parse(raw).unwrap();
    assert_eq!(e.summary, "用户喜欢美式咖啡");
    assert_eq!(e.outcome, "informational");
    assert!(e.keywords.is_empty());
}

#[test]
fn missing_summary_is_an_error() {
    assert!(parse(r#"{"outcome":"completed"}"#).is_err());
    assert!(parse(r#"{"summary":"   "}"#).is_err());
    assert!(parse("完全没有 JSON").is_err());
    assert!(parse("{\"summary\":\"没有闭合").is_err());
}

#[test]
fn unknown_outcome_degrades_to_unknown() {
    let e = parse(r#"{"summary":"摘要","outcome":"whatever"}"#).unwrap();
    assert_eq!(e.outcome, "unknown");
    // 大小写不敏感
    let e = parse(r#"{"summary":"摘要","outcome":" COMPLETED "}"#).unwrap();
    assert_eq!(e.outcome, "completed");
}

#[test]
fn long_summary_is_truncated_and_keywords_capped() {
    let long = "很".repeat(300);
    let many = (0..20)
        .map(|i| format!("\"k{i}\""))
        .collect::<Vec<_>>()
        .join(",");
    let raw = format!(r#"{{"summary":"{long}","keywords":[{many}]}}"#);
    let e = parse(&raw).unwrap();
    assert!(
        e.summary.chars().count() <= MAX_SUMMARY_CHARS + 1,
        "{}",
        e.summary.chars().count()
    );
    assert!(e.summary.ends_with('…'));
    assert_eq!(e.keywords.len(), MAX_KEYWORDS);
}

#[test]
fn build_user_text_marks_roles() {
    let t = build_user_text("我问", "我答");
    assert!(t.contains("【用户】我问"));
    assert!(t.contains("【助手】我答"));
}

#[test]
fn extraction_prompt_has_no_persona_or_memory() {
    let p = turn_prompt();
    assert!(p.memory.is_none(), "抽取请求不得注入记忆（会自我循环放大）");
    assert_eq!(p.instructions, EXTRACT_INSTRUCTIONS);
    assert!(p.instructions.contains("JSON"));
}
