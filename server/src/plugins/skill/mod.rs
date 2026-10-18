//! 统一技能目录（skill）：把 `[command]` 指令闸门与 `[nlu]` 自定义意图合并成**一个**
//! 技能表，规则、轻量模型与云 LLM 三级识别源都只在这里**提名**，副作用由会话层
//! 按目录统一执行。
//!
//! ## 三级级联中的位置
//!
//! 终端 → ASR → 规则 NLU（[`NluEngine`]，命中即执行）→ 轻量模型（配置了
//! `[nlu].bert_*` 时优先 [`crate::plugins::nlu::BertClassify`] TinyBERT 语义分类，
//! 否则/失败回落内置 [`LightModel`] 启发式打分，达阈值即执行）→ 云 LLM（本目录以
//! `execute_skill` 工具接入）→ TTS。
//!
//! ## 兼容性
//!
//! - `[command]` 配置原样生效：活跃时编译为内置技能 `end_session`（模板为空，
//!   走 [`NluEngine`] 的 exact/长度闸门，不进模糊层）；
//! - `[nlu]` 配置原样生效：意图 = 带 `{slot}` 模板的技能，`action.kind` 语义不变
//!  （`reply` / `local_command` / `end_session`），另支持 `mcp:<工具名>` 真正派发
//!   设备 MCP 工具（白名单来自服务端配置，LLM 只能提名技能名，不能指定工具）；
//! - LLM 侧只暴露**一个** `execute_skill` 工具（技能目录进 description），
//!   返回的工具调用经 [`SkillRegistry::from_tool_call`] 反序列化后走与本地完全相同的
//!   `resolve` 校验——模型输出不可信，目录是唯一安全边界。
//!
//! ## 为什么执行不留在识别层
//!
//! 三级识别源对"执行什么"各自只应有**提名权**（技能名 + 槽位），执行统一收敛到
//! [`SkillInvocation`]：槽位值表校验、动作白名单、长度上限都在 `resolve` 一处，
//! 规则/模型/LLM 任何一个源出 bug 都不可能绕过校验直接产生副作用。

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::{bail, Result};
use serde::Deserialize;
use serde_json::json;

use crate::plugins::command::CommandConfig;
use crate::plugins::llm::{ToolCall, ToolSpec};
use crate::plugins::nlu::{
    norm, parse_template, template::Tok, BertClassify, NluConfig, NluEngine,
};

mod model;
pub use model::LightModel;
#[cfg(test)]
mod tests;

/// LLM 侧唯一工具名：技能执行入口（目录详情进 description）。
pub const TOOL_NAME: &str = "execute_skill";

/// 提名来自哪一级识别源（日志/观测用；执行路径完全一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillStage {
    /// 规则层：模板锚点确定性命中（零误判）。
    Rules,
    /// 轻量模型层：模糊打分达阈值（见 [`LightModel`]）。
    Model,
}

impl SkillStage {
    pub fn as_str(self) -> &'static str {
        match self {
            SkillStage::Rules => "rules",
            SkillStage::Model => "model",
        }
    }
}

/// 技能命中后的副作用（`reply` / `local_command` 为兼容应答技能；
/// `mcp:<tool>` 调用设备白名单工具）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillAction {
    /// 结束聆听（原 `[command]` 闸门语义，设备 MCP `self.audio_listener.stop`）。
    EndSession,
    /// 其他本地动作：`kind = "reply"` 只应答；`kind = "mcp:<tool>"` 派发设备工具。
    LocalCommand { kind: String },
}

/// 一次**通过校验**的技能调用：识别层的唯一合法产物，会话层据此执行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInvocation {
    pub name: String,
    pub slots: HashMap<String, String>,
    pub action: SkillAction,
    /// 命中后要说的话（可能为空 = 直接执行副作用、不出声）。
    pub reply: String,
}

#[derive(Debug, Clone)]
struct Definition {
    name: String,
    templates: Vec<String>,
    action: SkillAction,
    reply: String,
}

/// 技能目录：规则引擎 + TinyBERT/轻量模型 + 校验器 + LLM 工具目录，会话级持有。
#[derive(Clone)]
pub struct SkillRegistry {
    rules: NluEngine,
    model: LightModel,
    /// TinyBERT 语义分类器（`[nlu].bert_enabled` 且引擎构建成功时才有；先于启发式）。
    bert: Option<Arc<dyn BertClassify>>,
    /// TinyBERT 接受阈值（取自 `[nlu].bert_threshold`；无 bert 时无意义）。
    bert_threshold: f32,
    /// TinyBERT 标签 → 技能名映射（`[nlu].bert_label_map`，缺省同名直配）。
    bert_label_map: HashMap<String, String>,
    definitions: Vec<Definition>,
    /// 槽位名 → (原始值表, 是否可选)；`resolve` 校验与 LLM 目录共用。
    slots: HashMap<String, (Vec<String>, bool)>,
}

impl SkillRegistry {
    /// 按两处配置快照构造：`command`（end_session 来源，向后兼容）+ `nlu`（自定义技能）。
    /// `bert` 由调用方注入（跨会话共享的已加载引擎，见 `Engines::bert_engine`；None = 未配置或不可用）。
    pub fn new(
        command: &CommandConfig,
        nlu: &NluConfig,
        bert: Option<Arc<dyn BertClassify>>,
    ) -> Self {
        // 内置提醒技能合并（见 [`NluConfig::with_builtins`]）：规则/模型/LLM 目录三处同源。
        let merged = nlu.with_builtins();
        let nlu = &merged;
        let mut definitions = Vec::new();
        if command.active() {
            definitions.push(Definition {
                name: "end_session".into(),
                templates: Vec::new(),
                action: SkillAction::EndSession,
                reply: command.reply.clone(),
            });
        }
        if nlu.enabled {
            for intent in &nlu.intents {
                // 重名/未知动作拒绝注册（validate 会明确报告）；坏配置不产生副作用。
                if intent.name.is_empty() || definitions.iter().any(|d| d.name == intent.name) {
                    continue;
                }
                let Some(action) = compile_action(&intent.action.kind) else {
                    continue;
                };
                definitions.push(Definition {
                    name: intent.name.clone(),
                    templates: intent.templates.clone(),
                    action,
                    reply: intent.action.reply.clone(),
                });
            }
        }
        let skills: Vec<(String, Vec<String>)> = definitions
            .iter()
            .map(|d| (d.name.clone(), d.templates.clone()))
            .collect();
        let slot_map: HashMap<String, Vec<String>> = nlu
            .slots
            .iter()
            .map(|s| (s.name.clone(), s.values.clone()))
            .collect();
        let slot_meta: HashMap<String, (Vec<String>, bool)> = nlu
            .slots
            .iter()
            .map(|s| (s.name.clone(), (s.values.clone(), s.optional)))
            .collect();
        Self {
            rules: NluEngine::new(command, nlu),
            model: LightModel::build(&skills, &slot_map, nlu.model_threshold, nlu.model_margin),
            bert,
            bert_threshold: nlu.bert_threshold.clamp(0.0, 1.0),
            bert_label_map: nlu.bert_label_map.clone(),
            definitions,
            slots: slot_meta,
        }
    }

    /// 是否有技能可被提名（全关时三级级联直接从 ASR 进 LLM）。
    pub fn active(&self) -> bool {
        !self.definitions.is_empty()
    }

    /// 三级中的本地两级：规则 → TinyBERT（配置时）→ 轻量模型；都未命中返回 `None`
    /// （调用方放行 LLM）。命中层级见 [`SkillStage`]（规则与模型两级，TinyBERT 与
    /// 启发式打分同属 Model——执行路径完全一致，仅提名来源不同）。
    pub fn parse_staged(&self, text: &str) -> Option<(SkillStage, SkillInvocation)> {
        if let Some(hit) = self.rules.parse(text) {
            return self
                .resolve(&hit.intent, hit.slots)
                .ok()
                .map(|inv| (SkillStage::Rules, inv));
        }
        // TinyBERT 语义分类（真实模型）：达阈值且标签映射到目录中的技能才提名；
        // 引擎出错只记日志并回落启发式（模型故障不能拖垮级联）。
        if let Some(bert) = &self.bert {
            match bert.classify_timed(text) {
                Ok(hit) if hit.confidence >= self.bert_threshold => {
                    let skill = self
                        .bert_label_map
                        .get(&hit.label)
                        .cloned()
                        .unwrap_or(hit.label.clone());
                    if let Ok(inv) = self.resolve(&skill, HashMap::new()) {
                        return Some((SkillStage::Model, inv));
                    }
                }
                Ok(_) => {} // 低于阈值：放行后续级（漏触发优于误触发）
                Err(e) => tracing::warn!("TinyBERT 分类失败（回落启发式/LLM）: {e:#}"),
            }
        }
        if !self.model.is_active() {
            return None;
        }
        let hit = self.model.parse(text)?;
        // 模型只提名：动作合法性、开放槽位长度等仍由 resolve 复核；拒绝则放行 LLM。
        self.resolve(&hit.skill, hit.slots)
            .ok()
            .map(|inv| (SkillStage::Model, inv))
    }

    /// 识别结果统一校验入口：**不信任任何识别源**（尤其 LLM 返回的 JSON）。
    ///
    /// - 技能必须存在于目录且已启用；
    /// - 槽位集合必须恰好匹配该技能**某一条模板**声明的槽位（缺失的必填槽位按
    ///   槽位表 optional 判定，多余字段一律拒绝——LLM 不能增造槽位）；
    /// - 封闭槽位值必须命中值表，并**改写为配置原文**（模型提的是归一化值）；
    /// - 开放槽位限长（防 LLM 塞超长内容进 MCP 参数）。
    pub fn resolve(
        &self,
        name: &str,
        mut slots: HashMap<String, String>,
    ) -> Result<SkillInvocation> {
        let Some(def) = self.definitions.iter().find(|d| d.name == name) else {
            bail!("未知或未启用的技能：{name}");
        };
        let supplied: BTreeSet<String> = slots.keys().cloned().collect();
        let schemas: Vec<BTreeSet<String>> = if def.templates.is_empty() {
            vec![BTreeSet::new()]
        } else {
            def.templates.iter().map(|t| slot_names(t)).collect()
        };
        let schema = schemas
            .iter()
            .find(|names| {
                supplied.is_subset(names)
                    && names.iter().all(|n| {
                        supplied.contains(n)
                            || self.slots.get(n).is_some_and(|(_, optional)| *optional)
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
                continue; // 可选槽位留空：保留空串占位，语义与规则层一致
            }
            if !values.is_empty() {
                let canonical = values
                    .iter()
                    .find(|v| norm(v) == norm(value))
                    .ok_or_else(|| anyhow::anyhow!("技能 {name} 的槽位 {key} 不在允许值表中"))?;
                *value = canonical.clone();
            }
            if value.chars().count() > 64 {
                bail!("技能 {name} 的槽位 {key} 过长（>64 字符）");
            }
        }
        Ok(SkillInvocation {
            name: name.into(),
            slots,
            action: def.action.clone(),
            reply: def.reply.clone(),
        })
    }

    /// LLM 工具目录：技能全关时返回空（会话层不传 tools，LLM 纯聊天）。
    pub fn tools(&self) -> Vec<ToolSpec> {
        if self.definitions.is_empty() {
            return Vec::new();
        }
        let catalog: Vec<_> = self
            .definitions
            .iter()
            .map(|d| {
                json!({
                    "name": d.name,
                    "templates": d.templates,
                    "slots": d.templates.iter().flat_map(|t| slot_names(t))
                        .collect::<BTreeSet<_>>().into_iter().map(|n| {
                            let (values, optional) = self.slots.get(&n).cloned().unwrap_or_default();
                            json!({"name": n, "values": values, "optional": optional})
                        }).collect::<Vec<_>>(),
                })
            })
            .collect();
        let catalog = json!(catalog).to_string();
        vec![ToolSpec {
            name: TOOL_NAME.into(),
            description: format!(
                "仅在用户明确要求执行技能时调用；否定、引用、假设、询问含义不是执行指令。\
                 一次只执行一个技能，不要声称已执行。技能目录：{catalog}"
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "skill": {"type": "string",
                              "enum": self.definitions.iter().map(|d| &d.name).collect::<Vec<_>>()},
                    "slots": {"type": "object",
                              "additionalProperties": {"type": "string"}}
                },
                "required": ["skill", "slots"],
                "additionalProperties": false
            }),
        }]
    }

    /// LLM 工具调用 → 技能调用（白名单外 / JSON 不合规 / 校验失败都拒绝）。
    pub fn from_tool_call(&self, call: &ToolCall) -> Result<SkillInvocation> {
        if call.name != TOOL_NAME {
            bail!("未授权的工具：{}", call.name);
        }
        if call.arguments.len() > 16 * 1024 {
            bail!("技能参数过长");
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Arguments {
            skill: String,
            #[serde(default)]
            slots: HashMap<String, String>,
        }
        let args: Arguments = serde_json::from_str(&call.arguments)?;
        self.resolve(&args.skill, args.slots)
    }
}

/// `action.kind` → 副作用；未知 kind 返回 `None`（拒绝注册）。
fn compile_action(kind: &str) -> Option<SkillAction> {
    match kind {
        "end_session" => Some(SkillAction::EndSession),
        "" | "reply" | "local_command" => Some(SkillAction::LocalCommand {
            kind: "reply".into(),
        }),
        k if k.starts_with("mcp:") && k.len() > 4 => {
            Some(SkillAction::LocalCommand { kind: k.into() })
        }
        _ => None,
    }
}

/// 模板声明的槽位名集合（校验与 LLM 目录共用）。
fn slot_names(template: &str) -> BTreeSet<String> {
    parse_template(template)
        .tokens
        .into_iter()
        .filter_map(|t| match t {
            Tok::Slot(s) => Some(s),
            _ => None,
        })
        .collect()
}

/// 保存时校验（注册表 `validate_nlu` 调用）；运行时仍以 [`SkillRegistry::resolve`]
/// 作最后一道安全边界。
pub fn validate(command: &CommandConfig, nlu: &NluConfig) -> Result<()> {
    if !nlu.model_threshold.is_finite()
        || !(0.0..=1.0).contains(&nlu.model_threshold)
        || !nlu.model_margin.is_finite()
        || !(0.0..=1.0).contains(&nlu.model_margin)
    {
        bail!("[nlu].model_threshold / model_margin 必须是 [0,1] 内有限数，请修正阈值");
    }
    if !nlu.enabled {
        return Ok(());
    }
    // 与运行时同源：校验对象 = 配置意图 ⊕ 内置提醒技能（重名配置覆盖内置后仍唯一）。
    let merged = nlu.with_builtins();
    let mut names = BTreeSet::new();
    if command.active() {
        names.insert("end_session".to_string());
    }
    for def in &merged.intents {
        if def.name.trim().is_empty() || !names.insert(def.name.clone()) {
            bail!(
                "[nlu].intents 技能名称为空或重复：{}，请使用唯一名称",
                def.name
            );
        }
        if compile_action(&def.action.kind).is_none() {
            bail!(
                "[nlu] 技能 {} 的 action.kind 未知，请使用 reply / end_session / mcp:<设备工具名>",
                def.name
            );
        }
    }
    Ok(())
}
