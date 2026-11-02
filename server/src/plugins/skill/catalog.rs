//! 保存态技能目录：生产、训练输入、manifest 与候选映射必须同源。

use super::{slot_names, SkillAction};
use crate::plugins::command::CommandConfig;
use crate::plugins::nlu::{ActionDef, IntentDef, NluConfig};
use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;

/// 不注入隐藏内置技能；默认 JSON 只在首次缺省配置中播种。
/// command 仅提供 end_session 的训练样本，不执行关键词匹配。
pub fn effective_catalog(command: &CommandConfig, nlu: &NluConfig) -> Result<NluConfig> {
    validate(command, nlu)?;
    let mut catalog = nlu.clone();
    if !nlu.enabled {
        catalog.intents.clear();
        catalog.slots.clear();
    }
    if command.active() {
        let mut templates = Vec::new();
        for keyword in &command.keywords {
            let keyword = keyword.trim();
            if !keyword.is_empty() && !templates.iter().any(|s| s == keyword) {
                templates.push(keyword.to_string());
            }
        }
        catalog.intents.push(IntentDef {
            name: "end_session".into(),
            templates,
            action: ActionDef {
                kind: "end_session".into(),
                reply: command.reply.clone(),
                ..Default::default()
            },
        });
    }
    catalog.enabled = !catalog.intents.is_empty();
    Ok(catalog)
}

pub(super) fn compile_action(action: &ActionDef) -> Result<SkillAction> {
    if action.kind != "plan" && action.plan.is_some() {
        bail!("action.plan 只能与 kind=plan 一同保存，请移除 plan 或改用 plan 动作");
    }
    Ok(match action.kind.as_str() {
        "end_session" => SkillAction::EndSession,
        "" | "reply" | "local_command" => SkillAction::LocalCommand {
            kind: "reply".into(),
        },
        "plan" => {
            let plan = action
                .plan
                .as_ref()
                .context("kind=plan 缺少 action.plan，请填写声明式步骤")?;
            crate::skills::validate_plan(plan)?;
            SkillAction::Plan { plan: plan.clone() }
        }
        kind if kind.starts_with("mcp:")
            && kind.len() > 4
            && !kind.contains('{')
            && !kind.contains('}') =>
        {
            SkillAction::LocalCommand { kind: kind.into() }
        }
        _ => bail!("未知 action.kind，请使用 reply / end_session / mcp:<固定工具名> / plan"),
    })
}

/// 校验不读取远端、不加载模型、不执行计划；真正动作仍需当前设备/会话上下文授权。
pub fn validate(command: &CommandConfig, nlu: &NluConfig) -> Result<()> {
    command.validate()?;
    if !nlu.bert_threshold.is_finite() || !(0.0..=1.0).contains(&nlu.bert_threshold) {
        bail!("[nlu].bert_threshold 必须为0~1的有限数，请修正模型阈值");
    }
    let mut names = BTreeSet::new();
    let mut slot_names_config = BTreeSet::new();
    for slot in &nlu.slots {
        if slot.name.trim().is_empty() || !slot_names_config.insert(slot.name.clone()) {
            bail!(
                "[nlu].slots 名称为空或重复，请使用唯一槽位名称：{}",
                slot.name
            );
        }
    }
    let closed_slots: BTreeSet<_> = nlu
        .slots
        .iter()
        .filter(|s| {
            !s.optional && !s.values.is_empty() && s.values.iter().all(|v| !v.trim().is_empty())
        })
        .map(|s| s.name.clone())
        .collect();
    for intent in &nlu.intents {
        if intent.name.trim().is_empty()
            || intent.name != intent.name.trim()
            || matches!(intent.name.as_str(), "other" | "end_session")
            || !names.insert(intent.name.clone())
        {
            bail!(
                "[nlu].intents 名称为空、重复或占用保留标签other/end_session，请改用唯一名称：{}",
                intent.name
            );
        }
        if intent.templates.is_empty() || intent.templates.iter().any(|t| t.trim().is_empty()) {
            bail!(
                "[nlu] 技能{}缺少训练例句/抽槽模板，请填写非空templates",
                intent.name
            );
        }
        let mut declared = BTreeSet::new();
        for template in &intent.templates {
            validate_template(template, &closed_slots)?;
            let fields = slot_names(template);
            for name in &fields {
                if !slot_names_config.contains(name) {
                    bail!(
                        "[nlu] 技能{}引用未声明槽位{name}，请在slots中定义",
                        intent.name
                    );
                }
            }
            declared.extend(fields);
        }
        compile_action(&intent.action)
            .with_context(|| format!("[nlu] 技能{}动作无效", intent.name))?;
        if let Some(plan) = &intent.action.plan {
            for condition in &plan.conditions {
                let slot = match condition {
                    crate::skills::SkillCondition::SlotPresent { slot }
                    | crate::skills::SkillCondition::SlotEquals { slot, .. } => Some(slot),
                    _ => None,
                };
                if let Some(slot) = slot {
                    if !declared.contains(slot) {
                        bail!("plan条件引用未声明槽位{slot}，请修正条件");
                    }
                }
            }
            validate_plan_slots(&serde_json::to_value(plan)?, &declared)?;
        }
    }
    for (label, target) in &nlu.bert_label_map {
        if label.trim().is_empty()
            || target.trim().is_empty()
            || (label == "other" && target != "other")
        {
            bail!("[nlu].bert_label_map 标签不能为空，且other不可映射为动作，请修正映射");
        }
        if target != "other" && target != "end_session" && !names.contains(target) {
            bail!("[nlu].bert_label_map 目标{target}不在技能目录中，请修正映射或启用对应技能");
        }
    }
    Ok(())
}

fn validate_template(template: &str, closed_slots: &BTreeSet<String>) -> Result<()> {
    let mut inside = false;
    let mut previous_slot: Option<String> = None;
    let mut name = String::new();
    for character in template.chars() {
        match character {
            '{' if !inside => {
                if previous_slot
                    .as_ref()
                    .is_some_and(|slot| !closed_slots.contains(slot))
                {
                    bail!("相邻槽位的前槽必须是必填封闭值表，请使用字面分隔或配置有限槽值");
                }
                inside = true;
                name.clear();
            }
            '}' if inside && !name.is_empty() => {
                inside = false;
                previous_slot = Some(name.clone());
            }
            '{' | '}' => bail!("模板槽位花括号无效，请使用{{slot}}"),
            character if inside => {
                if !character.is_ascii_alphanumeric() && character != '_' {
                    bail!("模板槽位名称只允许字母数字下划线，请修正{name}");
                }
                name.push(character);
            }
            _ => previous_slot = None,
        }
    }
    if inside {
        bail!("模板槽位未闭合，请补全花括号");
    }
    Ok(())
}

fn validate_plan_slots(value: &serde_json::Value, declared: &BTreeSet<String>) -> Result<()> {
    match value {
        serde_json::Value::String(text) => {
            let mut rest = text.as_str();
            while let Some((_, suffix)) = rest.split_once("{{slots.") {
                let (name, tail) = suffix
                    .split_once("}}")
                    .context("plan槽位引用未闭合，请使用{{slots.name}}")?;
                if !declared.contains(name) {
                    bail!("plan引用未在同技能模板声明的槽位{name}，请修正引用");
                }
                rest = tail;
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                validate_plan_slots(value, declared)?;
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                validate_plan_slots(value, declared)?;
            }
        }
        _ => {}
    }
    Ok(())
}
