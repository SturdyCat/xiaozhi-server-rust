//! 灵魂的**可观测性**：最终提示词预览（`POST /api/soul/preview` 的数据来源）。
//!
//! 为什么单独一个模块：这块逻辑（分段构成 + 字段来源 + 生效路径）与"组装"是两件事——
//! 组装关心**怎么拼**，预览关心**给别人看什么**，而且它是唯一需要解释
//! "哪些字段来自预设、哪些是你自己填的"的地方。
//!
//! ⚠️ 预览必须与真正发出去的 prompt **同源**：分段用 `prompt::assemble_parts`（`assemble` 的
//! 同一套排序/丢空规则），不做第二次拼装——否则"预览的和实际发的"迟早会漂移。

use crate::plugins::prompt;

use super::{presets, vars_for, SoulConfig};

/// 段摘要（[`Preview`] 用；只带长度，不带正文——正文由 `instructions` 完整给出）。
#[derive(Debug, Clone)]
pub struct SegmentInfo {
    /// 段 order（越小越靠前）。
    pub order: i32,
    /// 段名。
    pub name: &'static str,
    /// 插值后的字符数。
    pub chars: usize,
}

/// `/api/soul/preview` 的报告：**最终提示词** + 它由哪些段组成 + 每个字段的来源。
///
/// 为什么需要它：人格是"看不见的"——预设与字段合并之后到底发了什么，用户没法从表单上看出来，
/// 于是"改了没用/不敢改"就成必然（与记忆的「测试召回」同一个道理）。这里把
/// **最终文本**和**字段来源**（来自预设 / 被你覆盖）都摊开。
#[derive(Debug, Clone)]
pub struct Preview {
    /// `"soul"` = 走了人格组装；`"system_prompt"` = 未启用人格，实际发的是 `[llm].system_prompt`。
    pub source: &'static str,
    /// `[soul].enabled` 的实际取值。
    pub enabled: bool,
    /// 规范化后的预设 id。
    pub preset: String,
    /// 预设展示名（`none` 哨兵也有名字；未知值为 `None`——但未知值在预览里会直接报错）。
    pub preset_name: Option<&'static str>,
    /// 最终发给模型的 `instructions` 原文。
    pub instructions: String,
    /// `instructions` 的字符数（中文按 1 计）。
    pub chars: usize,
    /// 各段（顺序即最终拼接顺序）。
    pub segments: Vec<SegmentInfo>,
    /// 由预设提供的字段（你留空，或填的值与预设一致）。
    pub from_preset: Vec<String>,
    /// 你自己定的字段（与预设取值不同，或预设没有而你填了）。
    pub overridden: Vec<String>,
    /// **合并后的完整档案**（预设补上的字段都在里面）。
    ///
    /// 用途：客户端「载入预设内容到表单」把这份对象直接喂回表单，于是默认人格变成可逐条
    /// 修改的显式值——客户端因此**不需要**复制一份"留空=用预设"的合并规则。
    pub effective: SoulConfig,
}

impl Preview {
    /// wire 形状（客户端按这些键取值）。
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "source": self.source,
            "enabled": self.enabled,
            "preset": self.preset,
            "preset_name": self.preset_name,
            "instructions": self.instructions,
            "chars": self.chars,
            "segments": self
                .segments
                .iter()
                .map(|s| serde_json::json!({
                    "order": s.order,
                    "name": s.name,
                    "chars": s.chars,
                }))
                .collect::<Vec<_>>(),
            "from_preset": self.from_preset,
            "overridden": self.overridden,
            "effective": serde_json::to_value(&self.effective).unwrap_or(serde_json::Value::Null),
        })
    }
}

/// 生成预览（纯函数，无 IO；不改变运行状态）。入口：`soul::preview::build`。
///
/// 未知 preset / 非法变量引用会返回 `Err`——这正是它作为**保存前校验器**的价值：
/// 表单里点一下就能知道"这份人格能不能用、最终长什么样"。
pub fn build(cfg: &crate::config::Config, device_id: &str) -> anyhow::Result<Preview> {
    let soul = cfg.soul.effective()?;
    let sections = soul.sections(&cfg.llm.system_prompt);
    // 同源两次调用（各微秒级）：assemble 给正文，assemble_parts 给分段——不重写拼接规则
    let vars = vars_for(&soul, device_id);
    let instructions = prompt::assemble(&sections, &vars)?;
    let segments: Vec<SegmentInfo> = prompt::assemble_parts(&sections, &vars)?
        .iter()
        .map(|p| SegmentInfo {
            order: p.order,
            name: p.name,
            chars: p.text.chars().count(),
        })
        .collect();

    // 字段来源：只在启用且用了预设时统计（否则"来自预设"没有意义）。
    // 判定按**效果**而不是"你有没有敲过字"：`name`/`address_user` 有非空的 serde 默认值，
    // 若按"字段非空"统计，用户一个字没填也会被报成"你定的"——那正是这个报告要避免的误导。
    // 因此：预设有该字段且（你留空 或 你填的和预设一样）→ 来自预设；否则你填了 → 你自己定的。
    let (mut from_preset, mut overridden) = (Vec::new(), Vec::new());
    if soul.enabled && !presets::is_none(soul.preset_id()) {
        if let Some(p) = presets::get(soul.preset_id()) {
            let provided = presets::provided_fields(&(p.profile)());
            if let Ok(serde_json::Value::Object(user)) = serde_json::to_value(&cfg.soul) {
                for (k, v) in &user {
                    // `enabled`/`preset` 不是人格内容；布尔/数值恒有值，不算"覆盖"
                    if k == "preset" || k == "enabled" {
                        continue;
                    }
                    let filled = match v {
                        serde_json::Value::String(s) => !s.trim().is_empty(),
                        serde_json::Value::Array(a) => !a.is_empty(),
                        _ => continue,
                    };
                    match provided.get(k) {
                        Some(base) if !filled || base == v => from_preset.push(k.clone()),
                        _ if filled => overridden.push(k.clone()),
                        _ => {}
                    }
                }
            }
        }
    }
    from_preset.sort();
    overridden.sort();

    Ok(Preview {
        source: if soul.enabled { "soul" } else { "system_prompt" },
        enabled: soul.enabled,
        preset_name: presets::display_name(soul.preset_id()),
        preset: soul.preset_id().to_string(),
        chars: instructions.chars().count(),
        instructions,
        segments,
        from_preset,
        overridden,
        // 最后一项取所有权（其余字段只读借用，顺序不能反）
        effective: soul,
    })
}
