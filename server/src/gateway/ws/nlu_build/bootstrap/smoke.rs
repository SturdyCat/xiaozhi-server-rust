//! 冒烟门禁（S1–S7）：训练脚本自报用例 → 服务端**自己重跑**判定。
//!
//! 不信脚本自报的"结果"，只取它的**文本**：脚本说什么都不影响判定，
//! 这样"训练脚本被改坏"和"模型效果不够"不会混成一件事。
//!
//! 判定按**危险程度分级**（team-lead 裁定）：
//! - `bad_class`（接受但判错类）**必须为 0**：会路由到错误技能并可能执行动作，零容忍；
//! - `ok / positives` **≥ 90%**：拒识只是"用户重说一遍"，可容忍极少量；
//! - 负例 **3/3 全部拒识**：这 3 句在训练语料里就是 `other`，漏一条说明训练或量化坏了。
//!
//! 阈值取 90% 而不是 `positives - 1`：后者在意图少时更松（4 意图 → 75%），
//! 等于允许"模板句都被拒识一条"的模型自动上线，与"门禁只证明模型没坏"冲突。
//! 若日后有**实测数据**证明 int8 模型会偶发掉一条，再单独讨论"n≥10 时允许 1 条"。

use serde::Deserialize;
use std::{collections::HashSet, path::Path};

use crate::plugins::nlu::build_bert_checked;

// 判定口径**只有一份**（`nlu_build::predict`）：门禁与 `/test`、对比面板共用，
// 这里不再自己写 `predict`。改判定只改那边。
use super::super::{classify, Artifact, Reason, Verdict};
use super::state::{MissDetail, SmokeSummary};

/// 冒烟门禁总预算（S7）。
pub(super) const SMOKE_BUDGET_MS: u128 = 60_000;
/// 训练脚本自报冒烟用例的日志前缀。
const SMOKE_PREFIX: &str = "[train] smoke=";
const MAX_SMOKE_CASES: usize = 256;
/// 负例条数固定 3 条（`train.py` 的 `other_count`）。
///
/// 少一条就不能证明"拒识"，属**数据缺失**而不是模型不合格——所以报 `smoke_missing`，
/// 不能报 `smoke_negative`（`rejected < total` 在 total=1 时是"理由假的"）。
pub(super) const REQUIRED_NEGATIVES: usize = 3;
/// 明细落 `state.json` 的封顶条数：状态文件是给人看的，不是日志。
const MAX_DETAIL: usize = 5;

// ---------------------------------------------------------------------------
// 输入：脚本自报的用例
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(super) struct SmokeCase {
    pub(super) text: String,
    pub(super) expected: String,
}

#[derive(Debug, Deserialize)]
struct SmokeLine {
    cases: Vec<SmokeCase>,
    /// 脚本因日志预算裁剪过用例。`#[serde(default)]` 兼容上一版没有该键的行。
    #[serde(default)]
    truncated: bool,
}

/// `parse_smoke` 的结果。
///
/// 带上 `truncated` 而不是只返回 `Vec`：签名不带信息，调用方就只能猜，
/// 于是必然出现"负数不足报 smoke_negative"这类假理由。
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SmokePlan {
    pub(super) cases: Vec<SmokeCase>,
    pub(super) truncated: bool,
}

#[derive(Debug, PartialEq)]
pub(super) struct GateFailure {
    pub(super) kind: &'static str,
    pub(super) detail: String,
}

impl GateFailure {
    pub(super) fn new(kind: &'static str, detail: impl std::fmt::Display) -> Self {
        Self {
            kind,
            detail: detail.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// 输出：门禁实测值
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub(super) struct GateReport {
    /// 正例条数（`total` 的语义就是它）。
    pub(super) positives: usize,
    /// 接受且判对了。
    pub(super) ok: usize,
    /// **危险失败**：接受了但判成别的技能。
    pub(super) bad_class: usize,
    pub(super) negatives: usize,
    /// 负例被拒识总数（= `rejected_other + rejected_threshold`）。
    pub(super) rejected: usize,
    /// 模型直接输出 `other`。
    pub(super) rejected_other: usize,
    /// 判成某技能但低于阈值——**该查阈值而不是查模型**，所以要分开记。
    pub(super) rejected_threshold: usize,
    pub(super) bad_class_detail: Vec<MissDetail>,
    pub(super) rejected_detail: Vec<MissDetail>,
    pub(super) elapsed_ms: u128,
}

impl GateReport {
    /// 落 `state.json` 的精简摘要（不带明细）。
    pub(super) fn summary(&self) -> SmokeSummary {
        SmokeSummary {
            ok: self.ok,
            bad_class: self.bad_class,
            rejected: self.rejected,
            positives: self.positives,
            negatives: self.negatives,
        }
    }
}

// 单句判定的类型与实现都在 `nlu_build::predict`（`Verdict` / `Reason`）：
// 门禁、`/api/nlu/model/test`、批量对比必须用同一份，否则"同一句话两处结论不同"。

// ---------------------------------------------------------------------------
// 解析（S3）
// ---------------------------------------------------------------------------

/// 从 `job.log` 尾部取训练脚本自报的冒烟用例。
///
/// 取不到 / 解析不了 / 句子带 `{slot}` 花括号（模板没展开，等于喂训练时没见过的句子）
/// 一律 `smoke_missing`；**绝不**"取不到就当通过"。
pub(super) fn parse_smoke(log: &str, labels: &[String]) -> Result<SmokePlan, GateFailure> {
    let payload = log
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix(SMOKE_PREFIX))
        .ok_or_else(|| {
            GateFailure::new(
                "smoke_missing",
                "日志尾没有 [train] smoke= 行（训练脚本未输出或被 8192 字节尾窗截断）",
            )
        })?;
    let report: SmokeLine = serde_json::from_str(payload)
        .map_err(|e| GateFailure::new("smoke_missing", format!("smoke JSON 解析失败: {e}")))?;
    // 裁剪过就无法证明"负例 3/3"和"覆盖每个意图"，单独一个失败码：
    // 报成 smoke_negative/smoke_missing 都会把"脚本预算不够"说成"模型不好"。
    if report.truncated {
        return Err(GateFailure::new(
            "smoke_truncated",
            format!(
                "训练脚本因日志预算裁剪了冒烟用例（当前 {} 条），无法完整验证；请缩短语料模板或提高脚本预算",
                report.cases.len()
            ),
        ));
    }
    if report.cases.is_empty() || report.cases.len() > MAX_SMOKE_CASES {
        return Err(GateFailure::new(
            "smoke_missing",
            format!(
                "smoke 用例数 {} 不在 1..={MAX_SMOKE_CASES}",
                report.cases.len()
            ),
        ));
    }
    let mut negatives = 0usize;
    let mut positives = 0usize;
    let mut seen: HashSet<&str> = HashSet::new();
    for case in &report.cases {
        if case.text.trim().is_empty() || case.text.len() > 512 {
            return Err(GateFailure::new(
                "smoke_missing",
                "smoke 用例文本为空或过长（>512 字节）",
            ));
        }
        if case.text.contains('{') || case.text.contains('}') {
            return Err(GateFailure::new(
                "smoke_missing",
                format!("smoke 用例含未展开的花括号占位符: {:?}", case.text),
            ));
        }
        if case.expected == "other" {
            negatives += 1;
            continue;
        }
        if !labels.iter().any(|l| l == &case.expected) {
            return Err(GateFailure::new(
                "smoke_missing",
                format!("smoke 用例期望标签 {:?} 不在候选 labels 里", case.expected),
            ));
        }
        // 同一意图重复出现会让命中率被单条句子放大，等于换了个判定口径。
        if !seen.insert(case.expected.as_str()) {
            return Err(GateFailure::new(
                "smoke_missing",
                format!("smoke 用例里意图 {:?} 出现了不止一次", case.expected),
            ));
        }
        positives += 1;
    }
    if negatives != REQUIRED_NEGATIVES {
        return Err(GateFailure::new(
            "smoke_missing",
            format!("smoke 负例 {negatives} 条，必须正好 {REQUIRED_NEGATIVES} 条（数据缺失，不是模型不合格）"),
        ));
    }
    if positives == 0 {
        return Err(GateFailure::new(
            "smoke_missing",
            "smoke 正例 0 条（数据缺失）——0/0 不是 100%，无法验证命中率",
        ));
    }
    Ok(SmokePlan {
        cases: report.cases,
        truncated: false,
    })
}

// ---------------------------------------------------------------------------
// 推理与统计（S1、S2、S4、S5、S6）
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// 推理与统计（S1、S2、S4、S5、S6）
// ---------------------------------------------------------------------------

/// `UnmappedSkill` 是**契约问题**（标签映射指向技能目录里不存在的技能），不是模型能力问题。
///
/// 所以报 `labels_mismatch` 而不是 `smoke_bad_class`：后者的 next_step 是"修语料重建"，
/// 前者的 next_step 正是"标签/映射与目录不同源，按最新配置重建"——动作对得上。
/// 交给 `gate_verdict` 是错的：它只看计数，而"哪一条映射到了不存在的技能"这个信息
/// 必须原样带给人，压成一个计数就丢了。
fn unmapped_failure(artifact: &Artifact, guess: &Verdict) -> Option<GateFailure> {
    if guess.reason != Reason::UnmappedSkill {
        return None;
    }
    Some(GateFailure::new(
        "labels_mismatch",
        format!(
            "候选输出 label「{}」→ 技能「{}」，当前技能目录（{} 个技能）里没有这个技能：\
             bert_label_map 与技能目录不一致，模型输出会与技能错位",
            guess.label,
            guess.mapped,
            artifact.nlu.intents.len()
        ),
    ))
}

/// S1–S7 全过才算过门禁。同步阻塞（ONNX 加载 + 逐条推理），调用方放 `spawn_blocking`。
pub(super) fn run_gate(
    artifact: &Artifact,
    dir: &Path,
    plan: &SmokePlan,
    snapshot: &SnapshotGuard,
) -> Result<GateReport, GateFailure> {
    let started = std::time::Instant::now();
    // S2：labels 必须等于"技能目录顺序 + other"，非空且唯一。
    let expected: Vec<String> = artifact
        .nlu
        .intents
        .iter()
        .map(|i| i.name.clone())
        .chain(std::iter::once("other".into()))
        .collect();
    let mut unique = HashSet::new();
    if artifact.labels != expected
        || artifact.labels.is_empty()
        || !artifact.labels.iter().all(|l| unique.insert(l))
    {
        return Err(GateFailure::new(
            "labels_mismatch",
            format!("候选 labels {:?} 与目录顺序不一致", artifact.labels),
        ));
    }
    // S6：阈值与标签映射必须仍是构建快照里的值——不许为了让冒烟过而放宽。
    if (artifact.nlu.bert_threshold - snapshot.threshold).abs() > f32::EPSILON
        || artifact.nlu.bert_label_map != snapshot.label_map
    {
        return Err(GateFailure::new(
            "threshold_changed",
            format!(
                "候选阈值/标签映射与构建快照不一致（快照 {} / 候选 {}），拒绝自动采用（不替用户改判定条件）",
                snapshot.threshold, artifact.nlu.bert_threshold
            ),
        ));
    }
    // S1：真实加载（复用生产同一条 `build_bert_checked` 路径）。
    // 与"文件哈希不对"分开报：前者该查文件，这里该查 ONNX 能不能跑。
    let isolated = artifact.config(dir);
    let bert = build_bert_checked(&isolated)
        .map_err(|e| GateFailure::new("model_load_failed", format!("候选 ONNX 加载失败: {e}")))?;
    let mut report = GateReport {
        positives: 0,
        ok: 0,
        bad_class: 0,
        negatives: 0,
        rejected: 0,
        rejected_other: 0,
        rejected_threshold: 0,
        bad_class_detail: Vec::new(),
        rejected_detail: Vec::new(),
        elapsed_ms: 0,
    };
    for case in &plan.cases {
        // Err（置信度非法/输出越界/推理崩溃）= 模型坏了，不是"拒识"：
        // 当成拒识的话，一个输出 NaN 的坏产物会以"全部拒识"混过 bad_class 那一级，
        // 再靠正例通过率慢慢暴露甚至不暴露。所以一律硬失败，不计入任何 rejected 计数。
        let guess = classify(bert.as_ref(), artifact, &case.text).map_err(|e| {
            GateFailure::new("smoke_bad_class", format!("推理失败（模型坏了不能当拒识吞掉）: {e}"))
        })?;
        // 契约问题当场报，不等计数：哪一条映射到了不存在的技能，信息必须原样带给人。
        if let Some(failure) = unmapped_failure(artifact, &guess) {
            return Err(failure);
        }
        if case.expected == "other" {
            report.negatives += 1;
            match guess.reason {
                Reason::Accepted => {}
                Reason::Other => {
                    report.rejected += 1;
                    report.rejected_other += 1;
                }
                Reason::BelowThreshold => {
                    report.rejected += 1;
                    report.rejected_threshold += 1;
                }
                // unmapped_failure 已拦；走到这里算防御，按拒识计数保持 rejected==negatives 一致。
                Reason::UnmappedSkill => report.rejected += 1,
            }
            // 负例明细：被接受的那几条要能一眼看出"判成什么、多少分"。
            if report.rejected_detail.len() < MAX_DETAIL {
                report
                    .rejected_detail
                    .push(miss(&case.text, &case.expected, &guess));
            }
            continue;
        }
        report.positives += 1;
        match guess.intent() {
            Some(intent) if intent == case.expected.as_str() => report.ok += 1,
            // 危险失败：接受即会路由技能，判错类就是执行错动作。
            Some(_) => {
                report.bad_class += 1;
                if report.bad_class_detail.len() < MAX_DETAIL {
                    report
                        .bad_class_detail
                        .push(miss(&case.text, &case.expected, &guess));
                }
            }
            None => {}
        }
    }
    report.elapsed_ms = started.elapsed().as_millis();
    gate_verdict(&report)?;
    Ok(report)
}

fn miss(text: &str, expected: &str, guess: &Verdict) -> MissDetail {
    MissDetail {
        text: text.chars().take(60).collect(),
        expected: expected.to_string(),
        got: guess.label.clone(),
        reason: match guess.reason {
            Reason::Other => "rejected_other".to_string(),
            Reason::BelowThreshold => "below_threshold".to_string(),
            Reason::UnmappedSkill => "unmapped_skill".to_string(),
            Reason::Accepted => format!("wrong_intent:{}", guess.mapped),
        },
        confidence: guess.confidence,
    }
}

/// S4a/S4b/S5/S7 的判定规则（纯函数：单测直接锁三条阈值与判定顺序）。
///
/// 顺序有意义：**先拦危险失败**（错类接受），再谈命中率，最后看拒识。
/// 反过来的话，`bad_class` 会被算进普通 miss 里被 90% 吃掉。
pub(super) fn gate_verdict(report: &GateReport) -> Result<(), GateFailure> {
    // S7：预算。超时说明机器或模型有问题，不是"不准"，必须先报这个。
    if report.elapsed_ms > SMOKE_BUDGET_MS {
        return Err(GateFailure::new(
            "smoke_timeout",
            format!(
                "冒烟推理 {}ms 超过 {}ms（疑似 CPU 被争抢或模型过大）",
                report.elapsed_ms, SMOKE_BUDGET_MS
            ),
        ));
    }
    // S4a：危险失败零容忍。
    if report.bad_class > 0 {
        let sample = report
            .bad_class_detail
            .first()
            .map(|d| {
                format!(
                    "「{}」期望 {}，实际 {}（置信度 {:.3}）",
                    d.text, d.expected, d.reason, d.confidence
                )
            })
            .unwrap_or_else(|| "（明细见 state.json）".to_string());
        return Err(GateFailure::new(
            "smoke_bad_class",
            format!(
                "{} 条被\"接受但判错类别\"：{sample}。错类接受会路由到错误技能，比拒识危险",
                report.bad_class
            ),
        ));
    }
    // 正例为 0 是数据缺失（真值不会出现，parse_smoke 已拦；这里防御性保留）。
    if report.positives == 0 {
        return Err(GateFailure::new(
            "smoke_missing",
            "冒烟正例为 0 条，无法计算命中率",
        ));
    }
    // S4b：正例命中率 ≥90%（不写 `positives - 1`）。
    if report.ok * 10 < report.positives * 9 {
        return Err(GateFailure::new(
            "smoke_positive",
            format!(
                "正例命中 {}/{} 低于 90%（阈值未调整，不会为了通过而放宽）",
                report.ok, report.positives
            ),
        ));
    }
    // 负例条数不足属**数据缺失**（正常已被 `parse_smoke` 拦成 `smoke_missing`，
    // 这里是兜底）：报成 `smoke_negative` 就是"理由假的"，会把人引去查模型。
    // 判据是 `<` 而不是 `!=`，与 `parse_smoke` 的两处口径**故意不同**：
    // - `parse_smoke` 的 `!=` 是**对训练脚本的契约检查**（对应 `other_count=3`），
    //   多于 3 条说明脚本口径变了，该拦；
    // - 这里是**证据充分性检查**：4 条负例全拒识比 3 条全拒识是更强的证据，拦掉反而是错的。
    // 放宽负例条数时只需改 `parse_smoke` 一处。`>` 方向由 `rejected != negatives` 兜住。
    if report.negatives < REQUIRED_NEGATIVES {
        return Err(GateFailure::new(
            "smoke_missing",
            format!(
                "冒烟负例仅 {} 条，少于 {REQUIRED_NEGATIVES} 条（数据缺失，不是模型拒识不合格）",
                report.negatives
            ),
        ));
    }
    // S5：负例必须**全部**拒识。这 3 句在训练语料里就是 other，漏一条说明训练或
    // 量化有功能性缺陷，不是难度问题。文案用实测条数，不硬编码 3（将来加负例也说得通）。
    if report.rejected != report.negatives {
        return Err(GateFailure::new(
            "smoke_negative",
            format!(
                "闲聊句仅 {}/{} 被拒识（要求全部拒识）；这几句在训练语料里就是 other，\
                 漏判说明训练或量化有功能性缺陷，不是难度问题",
                report.rejected, report.negatives
            ),
        ));
    }
    Ok(())
}

/// 构建快照里必须在门禁时保持不变的两个判定条件。
pub(super) struct SnapshotGuard {
    pub(super) threshold: f32,
    pub(super) label_map: std::collections::HashMap<String, String>,
}

#[cfg(test)]
#[path = "smoke_tests.rs"]
mod tests;
