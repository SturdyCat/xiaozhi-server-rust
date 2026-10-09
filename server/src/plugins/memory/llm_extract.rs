//! 记忆**抽取**（后台辅助 LLM 调用）：一轮对话 → 一条自包含摘要。
//!
//! ## 为什么只要摘要、不要三元组（对设计文档 §5.3 的一处收敛）
//!
//! 上游的导航层是 SPO 三元组 + 社区 + PPR，那套东西的价值在 P2（图排序）。
//! P1 若同时落"摘要 + 三元组"两张表，会先付出一份**未被使用**的复杂度（还要处理
//! 模型输出不稳定导致的三元组脏数据）。因此 v1 只落 `summary`（导航胶囊）+ `keywords`
//! （帮助词法召回），并在 `gm_summary_terms` 上建倒排索引 —— 证据层仍是**原始 Q/A**，
//! 这一点（"导航层不得成为事实载荷"）与上游完全一致。三元组/PPR 留给 P2。
//!
//! ## 失败隔离
//!
//! 抽取**绝不在关键路径**（会拖长 TTFA 并占用远端额度）：调用方在回复完成之后 `spawn`
//! 后台任务。失败不抛给用户，只记 `extraction_error` + 有限重试，超限入隔离
//! （对齐上游"失败隔离但不阻塞对话"）。
//!
//! ## 输出契约
//!
//! 只用**一个 JSON 对象**、只认三个键，且解析器对模型的常见失真（```json 围栏、
//! 前后废话、多余键）都容忍：抽取是增强功能，不该因为模型多打一个逗号就整轮失败。

use anyhow::{Context, Result};

use crate::plugins::prompt::TurnPrompt;

/// 抽取指令（作为 `instructions` 下发，与人格/记忆段完全隔离）。
pub const EXTRACT_INSTRUCTIONS: &str = "\
你是记忆抽取器。给你一轮语音助手的对话记录，\
你要输出一条**自包含**的记忆摘要，供日后检索使用。\n\
要求：\n\
1. 摘要 ≤60 个汉字，写清「谁/什么 · 做了什么决定或发生了什么 · 结论」，\
不要写「用户询问了某事」这种空壳句式，不要复述整句话。\n\
2. 不要记录寒暄、语气词、无信息量的闲聊。\n\
3. 不要臆测：对话里没说的不要写。\n\
4. 只输出一个 JSON 对象，不要解释、不要 Markdown 围栏。\n\
格式：{\"summary\": \"…\", \"outcome\": \"completed|partial|failed|informational|unknown\", \
\"keywords\": [\"关键词1\", \"关键词2\"]}\n\
outcome 含义：completed=事情办成了；partial=只做了一部分；failed=没办成/被拒绝；\
informational=只是提供或获取了信息；unknown=不确定。";

/// 合法的 outcome 取值（抽取结果落在集合外一律降级为 `unknown`）。
pub const OUTCOMES: &[&str] = &["completed", "partial", "failed", "informational", "unknown"];

/// 摘要长度上限（字符）：过长会挤占语音端的 token 预算。
const MAX_SUMMARY_CHARS: usize = 120;
/// 关键词个数上限。
const MAX_KEYWORDS: usize = 8;

/// 抽取结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extraction {
    pub summary: String,
    pub outcome: String,
    pub keywords: Vec<String>,
}

/// 抽取请求的 prompt 计划（无记忆、无人格：抽取器只认自己的指令）。
pub fn turn_prompt() -> TurnPrompt {
    TurnPrompt {
        instructions: EXTRACT_INSTRUCTIONS.to_string(),
        memory: None,
        position: Default::default(),
    }
}

/// 把一轮对话拼成抽取器的输入。
pub fn build_user_text(user: &str, assistant: &str) -> String {
    format!("【用户】{user}\n【助手】{assistant}")
}

/// 解析模型输出（容忍围栏/废话/多余键）。
pub fn parse(text: &str) -> Result<Extraction> {
    let start = text.find('{').context("模型输出里没有 JSON 对象")?;
    let end = text.rfind('}').context("模型输出里的 JSON 不完整")?;
    if end <= start {
        anyhow::bail!("模型输出里的 JSON 不完整");
    }
    let v: serde_json::Value =
        serde_json::from_str(&text[start..=end]).context("解析抽取 JSON 失败")?;
    let summary = v
        .get("summary")
        .and_then(|s| s.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if summary.is_empty() {
        anyhow::bail!("抽取结果缺少 summary（模型没有按要求输出）");
    }
    let outcome = v
        .get("outcome")
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_lowercase())
        .filter(|s| OUTCOMES.contains(&s.as_str()))
        .unwrap_or_else(|| "unknown".to_string());
    let keywords = v
        .get("keywords")
        .and_then(|k| k.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .take(MAX_KEYWORDS)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(Extraction {
        summary: truncate(&summary, MAX_SUMMARY_CHARS),
        outcome,
        keywords,
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests;
