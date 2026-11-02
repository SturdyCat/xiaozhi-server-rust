//! 模型唯一意图门禁。目录/槽位/动作与训练共用，副作用仅由会话执行。
//! 无规则、启发式或 LLM 意图提名回退；模板只对已接受的同一技能抽槽。

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use crate::plugins::command::CommandConfig;
use crate::plugins::llm::{ToolCall, ToolSpec};
use crate::plugins::nlu::{
    matcher::match_template, norm, parse_template, template::Tok, BertClassify, NluConfig,
};
use anyhow::{bail, Result};

mod catalog;
pub use catalog::{effective_catalog, validate};
#[cfg(test)]
mod tests;

/// 仅为旧诊断调用方保留的标识；生产不再暴露此 LLM 工具。
pub const TOOL_NAME: &str = "execute_skill";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillStage {
    Model,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillAction {
    EndSession,
    LocalCommand { kind: String },
    Plan { plan: crate::skills::SkillPlan },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInvocation {
    pub name: String,
    pub slots: HashMap<String, String>,
    pub action: SkillAction,
    /// 仅在动作真实成功后应答；不作为任务提交成功的替代。
    pub reply: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Recognition {
    Accepted {
        inv: SkillInvocation,
        confidence: f32,
    },
    NeedsSlots {
        name: String,
        missing: Vec<String>,
        confidence: f32,
    },
    Disabled,
    Rejected,
    Unavailable {
        reason: String,
    },
}

#[derive(Debug, Clone)]
struct Definition {
    name: String,
    templates: Vec<String>,
    action: SkillAction,
    reply: String,
}

#[derive(Clone)]
pub struct SkillRegistry {
    bert: Option<Arc<dyn BertClassify>>,
    model_enabled: bool,
    threshold: f32,
    label_map: HashMap<String, String>,
    definitions: Vec<Definition>,
    known_names: BTreeSet<String>,
    slots: HashMap<String, (Vec<String>, bool)>,
    catalog_error: Option<String>,
}

impl SkillRegistry {
    /// 引擎跨会话共享，目录按保存态快照固定；启用却缺引擎绝不改走规则。
    pub fn new(
        command: &CommandConfig,
        nlu: &NluConfig,
        bert: Option<Arc<dyn BertClassify>>,
    ) -> Self {
        let (catalog, catalog_error) = match effective_catalog(command, nlu) {
            Ok(catalog) => (Some(catalog), None),
            Err(error) => (None, Some(format!("技能目录不可用：{error:#}"))),
        };
        let definitions = catalog
            .as_ref()
            .map(|c| {
                c.intents
                    .iter()
                    .filter_map(|intent| {
                        Some(Definition {
                            name: intent.name.clone(),
                            templates: intent.templates.clone(),
                            action: catalog::compile_action(&intent.action).ok()?,
                            reply: intent.action.reply.clone(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let slots = catalog
            .as_ref()
            .map(|c| {
                c.slots
                    .iter()
                    .map(|s| (s.name.clone(), (s.values.clone(), s.optional)))
                    .collect()
            })
            .unwrap_or_default();
        let mut known_names: BTreeSet<_> = nlu
            .intents
            .iter()
            .map(|intent| intent.name.clone())
            .collect();
        known_names.insert("end_session".into());
        Self {
            bert,
            model_enabled: nlu.bert_enabled,
            threshold: nlu.bert_threshold,
            label_map: nlu.bert_label_map.clone(),
            definitions,
            known_names,
            slots,
            catalog_error,
        }
    }

    pub fn active(&self) -> bool {
        self.model_enabled && (!self.definitions.is_empty() || self.catalog_error.is_some())
    }

    /// 每轮唯一分类入口。Rejected/Disabled 允许纯聊天，但没有任何 LLM 技能工具。
    /// Unavailable 必须按严格错误处理；NeedsSlots 只澄清，不重新识别意图。
    pub fn recognize(&self, text: &str) -> Recognition {
        if !self.model_enabled {
            return Recognition::Disabled;
        }
        if let Some(reason) = &self.catalog_error {
            return Recognition::Unavailable {
                reason: reason.clone(),
            };
        }
        if self.definitions.is_empty() {
            return Recognition::Disabled;
        }
        if !self.threshold.is_finite() || !(0.0..=1.0).contains(&self.threshold) {
            return Recognition::Unavailable {
                reason: "模型阈值无效，请设为0~1的有限数".into(),
            };
        }
        let Some(bert) = &self.bert else {
            return Recognition::Unavailable {
                reason: "模型已启用但未加载，请检查模型文件与运行状态".into(),
            };
        };
        let hit = match bert.classify_timed(text) {
            Ok(hit) => hit,
            Err(error) => {
                return Recognition::Unavailable {
                    reason: format!("模型分类失败：{error:#}"),
                }
            }
        };
        if !hit.confidence.is_finite() || !(0.0..=1.0).contains(&hit.confidence) {
            return Recognition::Unavailable {
                reason: "模型返回无效置信度".into(),
            };
        }
        let name = self.label_map.get(&hit.label).unwrap_or(&hit.label);
        if hit.label == "other" || name == "other" || hit.confidence < self.threshold {
            return Recognition::Rejected;
        }
        let Some(def) = self.definitions.iter().find(|d| d.name == *name) else {
            if self.known_names.contains(name) {
                return Recognition::Rejected;
            }
            return Recognition::Unavailable {
                reason: format!("模型返回未知标签：{name}"),
            };
        };
        let slot_values = self
            .slots
            .iter()
            .map(|(k, (v, _))| (k.clone(), v.iter().map(|v| norm(v)).collect()))
            .collect();
        let slot_optional = self
            .slots
            .iter()
            .map(|(k, (_, optional))| (k.clone(), *optional))
            .collect();
        let mut candidates = Vec::new();
        // 无槽技能无需字面匹配，模型就是唯一意图选择者。
        if def.templates.iter().any(|t| slot_names(t).is_empty()) {
            if let Ok(inv) = self.resolve(name, HashMap::new()) {
                candidates.push(inv);
            }
        } else {
            let mut most_literal = 0;
            for source in &def.templates {
                let template = parse_template(source);
                if let Some(slots) = match_template(text, &template, &slot_values, &slot_optional) {
                    if let Ok(inv) = self.resolve(name, slots) {
                        // 同技能显式字面边界优先，避免宽模板把“的”等锚点吞进开放槽。
                        // 这不选择意图；同等边界仍有冲突时必须澄清。
                        let literal: usize = template
                            .tokens
                            .iter()
                            .map(|token| match token {
                                Tok::Lit(text) => text.chars().count(),
                                Tok::Slot(_) => 0,
                            })
                            .sum();
                        if literal > most_literal {
                            candidates.clear();
                            most_literal = literal;
                        }
                        if literal == most_literal && !candidates.contains(&inv) {
                            candidates.push(inv);
                        }
                    }
                }
            }
        }
        if candidates.len() == 1 {
            return Recognition::Accepted {
                inv: candidates.remove(0),
                confidence: hit.confidence,
            };
        }
        let missing = def
            .templates
            .iter()
            .map(|t| {
                slot_names(t)
                    .into_iter()
                    .filter(|key| !self.slots.get(key).is_some_and(|(_, optional)| *optional))
                    .collect::<Vec<_>>()
            })
            .min_by_key(|keys| keys.len())
            .unwrap_or_default();
        Recognition::NeedsSlots {
            name: name.clone(),
            missing,
            confidence: hit.confidence,
        }
    }

    /// 旧诊断入口兼容，但同样仅模型识别；不再保留任何暗回退。
    #[deprecated(
        note = "Use recognize and handle Unavailable/NeedsSlots explicitly; diagnostic compatibility only"
    )]
    pub fn parse_staged(&self, text: &str) -> Option<(SkillStage, SkillInvocation)> {
        self.try_parse_staged(text).ok().flatten()
    }

    pub fn try_parse_staged(&self, text: &str) -> Result<Option<(SkillStage, SkillInvocation)>> {
        match self.recognize(text) {
            Recognition::Accepted { inv, .. } => Ok(Some((SkillStage::Model, inv))),
            Recognition::Unavailable { reason } => bail!("{reason}"),
            _ => Ok(None),
        }
    }

    /// 同技能槽位/目录验证，不执行任何动作。LLM 回提不能进入这里。
    pub fn resolve(
        &self,
        name: &str,
        mut slots: HashMap<String, String>,
    ) -> Result<SkillInvocation> {
        let Some(def) = self.definitions.iter().find(|d| d.name == name) else {
            bail!("未知或未启用的技能：{name}");
        };
        let supplied: BTreeSet<String> = slots.keys().cloned().collect();
        let schemas: Vec<_> = def.templates.iter().map(|t| slot_names(t)).collect();
        let schema = schemas
            .iter()
            .find(|names| {
                supplied.is_subset(names)
                    && names.iter().all(|key| {
                        supplied.contains(key)
                            || self.slots.get(key).is_some_and(|(_, optional)| *optional)
                    })
            })
            .ok_or_else(|| anyhow::anyhow!("技能 {name} 的槽位缺失或包含未声明字段"))?;
        for key in schema {
            let value = slots.entry(key.clone()).or_default();
            let (values, optional) = self.slots.get(key).cloned().unwrap_or_default();
            if value.trim().is_empty() {
                if !optional {
                    bail!("技能 {name} 的必填槽位 {key} 为空");
                }
                continue;
            }
            if value.chars().count() > 256 {
                bail!("技能 {name} 的槽位 {key} 过长（>256字符）");
            }
            if !values.is_empty() {
                *value = values
                    .iter()
                    .find(|v| norm(v) == norm(value))
                    .ok_or_else(|| anyhow::anyhow!("技能 {name} 的槽位 {key} 不在允许值表中"))?
                    .clone();
            }
        }
        Ok(SkillInvocation {
            name: name.into(),
            slots,
            action: def.action.clone(),
            reply: def.reply.clone(),
        })
    }

    /// 安全兼容：LLM 不拥有意图识别或动作回提入口。
    pub fn tools(&self) -> Vec<ToolSpec> {
        Vec::new()
    }
    pub fn from_tool_call(&self, _call: &ToolCall) -> Result<SkillInvocation> {
        bail!("LLM技能回提已停用；技能只能由本轮真实模型接受")
    }
}

fn slot_names(template: &str) -> BTreeSet<String> {
    parse_template(template)
        .tokens
        .into_iter()
        .filter_map(|t| match t {
            Tok::Slot(name) => Some(name),
            _ => None,
        })
        .collect()
}
