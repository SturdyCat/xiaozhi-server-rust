//! 单句判定的**唯一**实现：门禁冒烟、`/api/nlu/model/test`、批量对比共用同一份口径。
//!
//! 三处各写一份的下场是可预见的：阈值用 `<` 还是 `<=`、`other` 优先还是阈值优先、
//! "映射到 other / 映射到不存在的技能"算不算拒识——每一条都会在某次改动后只改了两处，
//! 于是同一句话 `/test` 说拒、`/compare` 说接受，而两边都"有测试"。
//!
//! 判定顺序（[`verdict`]）：
//! 1. 置信度非有限或越界 → **Err**（模型坏了，不是"拒识"，不能混成同一个 `None`）；
//! 2. 原始标签不在候选 `labels` 内 → **Err**（输出越界，同样不是拒识）；
//! 3. 原始标签**或**映射后名字是 `other` → [`Reason::Other`]；
//! 4. 置信度低于候选阈值 → [`Reason::BelowThreshold`]；
//! 5. 映射后名字不在当前技能目录里 → [`Reason::UnmappedSkill`]；
//! 6. 其余 → [`Reason::Accepted`]。
//!
//! 第 3、5 条是这次合并**补齐**的严格判定：旧的 `bootstrap::smoke::predict` 只看原始
//! `label == "other"`，既不判映射结果、也不判目录，因此会**接受一个不存在的技能**
//! （`intent = Some("ghost")`）；而 `nlu_model::prediction` 是拒的。旧 manifest 里
//! `bert_label_map` 可能非空（`training_catalog` 只对新构建 `clear()`），构建历史把
//! 这类候选放出来做对比时最容易踩到，所以统一按严格版。

use crate::plugins::nlu::{BertClassify, BertHit};

use super::Artifact;

/// 一条句子的判定结果分类。四态而不是"两个 bool 拼"：
/// "被接受 / 判 other / 低于阈值 / 映射到不存在的技能"的**运维动作完全不同**
/// （该改语料 / 该调阈值 / 该重建目录），压成 bool 就只能靠猜。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    Accepted,
    Other,
    BelowThreshold,
    UnmappedSkill,
}

impl Reason {
    /// 上报给前端的错误码。`Accepted` 没有"原因"，给 `None`（JSON 里就是 `null`）。
    pub(crate) fn code(self) -> Option<&'static str> {
        match self {
            Self::Accepted => None,
            Self::Other => Some("other"),
            Self::BelowThreshold => Some("below_threshold"),
            Self::UnmappedSkill => Some("unmapped_skill"),
        }
    }

    /// 稳定英文名，用于 `state.json` 明细与日志。与 `code()` 分开是为了**文案可以改、
    /// 落盘字段不许改**：`code()` 是给界面的，`as_str()` 是给历史文件对账的。
    pub(crate) fn as_str(self) -> &'static str {
        self.code().unwrap_or("accepted")
    }

    pub(crate) fn is_accepted(self) -> bool {
        self == Self::Accepted
    }
}

/// 单句判定。`label` 与 `mapped` 分开存，是因为两者回答不同的问题：
/// "模型原始输出什么"（查模型/查标签文件）与"最终会路由到哪个技能"（查目录/查映射）。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Verdict {
    /// 模型原始输出标签（**未经** `bert_label_map` 映射）。
    pub(crate) label: String,
    /// 过 `bert_label_map` 后的名字。**拒识时也给**：界面要能回答
    /// "它以为是什么、多少分"，否则运维无法判断该调阈值还是该重建。
    pub(crate) mapped: String,
    pub(crate) confidence: f32,
    pub(crate) reason: Reason,
    pub(crate) tokenize_ms: u64,
    pub(crate) infer_ms: u64,
}

impl Verdict {
    pub(crate) fn accepted(&self) -> bool {
        self.reason.is_accepted()
    }

    /// 路由目标：**只有被接受时才存在**（门禁原来 `intent: Option` 的语义）。
    /// 拒识时 `None`，不要拿 `mapped` 当技能用。
    pub(crate) fn intent(&self) -> Option<&str> {
        if self.accepted() {
            Some(self.mapped.as_str())
        } else {
            None
        }
    }
}

/// 纯函数判定：无 ONNX、无 IO，单测直接锁四条分支与判定顺序。
pub(crate) fn verdict(artifact: &Artifact, hit: &BertHit) -> anyhow::Result<Verdict> {
    if !hit.confidence.is_finite() || !(0.0..=1.0).contains(&hit.confidence) {
        anyhow::bail!("模型置信度非法: {}", hit.confidence);
    }
    if !artifact.labels.contains(&hit.label) {
        anyhow::bail!("模型输出不在候选 labels 内: {}", hit.label);
    }
    let mapped = artifact.nlu.bert_map_label(&hit.label).to_string();
    let reason = if hit.label == "other" || mapped == "other" {
        // `other` 是保留标签，**永远不是技能**：映射结果落到 other 与直接输出 other 同义。
        Reason::Other
    } else if hit.confidence < artifact.nlu.bert_threshold {
        Reason::BelowThreshold
    } else if !artifact.nlu.intents.iter().any(|i| i.name == mapped) {
        Reason::UnmappedSkill
    } else {
        Reason::Accepted
    };
    Ok(Verdict {
        label: hit.label.clone(),
        mapped,
        confidence: hit.confidence,
        reason,
        tokenize_ms: hit.tokenize_ms,
        infer_ms: hit.infer_ms,
    })
}

/// 唯一的推理入口：`classify_timed` + [`verdict`]。
///
/// 收 `&dyn BertClassify` 而不是 `Arc`：调用方本来就有 `Arc<dyn BertClassify>`
/// （`build_bert_checked` 的返回），`as_ref()` 即可；这样也不需要第二个 trait 边界。
pub(crate) fn classify(
    bert: &dyn BertClassify,
    artifact: &Artifact,
    text: &str,
) -> anyhow::Result<Verdict> {
    let hit = bert.classify_timed(text)?;
    verdict(artifact, &hit)
}

#[cfg(test)]
#[path = "predict_tests.rs"]
mod tests;
