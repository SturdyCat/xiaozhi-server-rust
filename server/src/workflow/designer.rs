//! Declarative plugin workflow schema, compiler, and stable API projection.
//! The compiler is intentionally independent of runtime engines: it validates
//! topology and produces an ordered plan without changing the legacy pipeline.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

pub const POSITIONS: &[(&str, &str, &str)] = &[
    ("start", "开始", "core"),
    ("vad", "语音活动检测", "core"),
    ("link.vad.asr", "VAD→识别", "attachment"),
    ("asr", "语音识别", "core"),
    ("link.asr.nlu", "识别→意图", "attachment"),
    ("nlu", "意图识别", "core"),
    ("link.nlu.llm", "意图→对话", "attachment"),
    ("llm", "大语言模型", "core"),
    ("link.llm.tts", "对话→语音", "attachment"),
    ("tts", "语音合成", "core"),
    ("end", "结束", "core"),
];

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkflowConfig {
    pub version: u32,
    pub bindings: Vec<Binding>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Binding {
    pub id: String,
    pub plugin_id: String,
    pub position_id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub order: i32,
    #[serde(default)]
    pub condition: Condition,
    #[serde(default = "default_continue")]
    pub on_error: String,
}
fn default_true() -> bool {
    true
}
fn default_continue() -> String {
    "continue".into()
}
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Condition {
    #[default]
    Always,
    TextContains {
        text: String,
        #[serde(default)]
        case_sensitive: bool,
    },
    DeviceKind {
        value: String,
    },
    Confidence {
        min: f32,
        max: Option<f32>,
    },
}

#[derive(Debug, Clone)]
pub struct CompiledBinding {
    pub plugin_id: String,
    pub position_id: String,
    pub order: i32,
    pub condition: Condition,
    pub on_error: String,
}
#[derive(Debug, Clone)]
pub struct CompiledWorkflow {
    pub config: WorkflowConfig,
    pub plan: Vec<CompiledBinding>,
}

const PLUGINS: &[(&str, &str, &str, &[&str])] = &[
    ("skill.gate", "技能闸门", "nlu", &["nlu"]),
    (
        "soul.profile",
        "人格档案",
        "link.nlu.llm",
        &["link.nlu.llm"],
    ),
    (
        "memory.recall",
        "记忆召回",
        "link.nlu.llm",
        &["link.nlu.llm"],
    ),
    ("memory.writeback", "记忆写回", "end", &["end"]),
    ("wakeup.greeting", "唤醒问候", "start", &["start"]),
    ("nlu.rules", "规则意图", "nlu", &["nlu"]),
    (
        "command.gate",
        "指令闸门",
        "link.asr.nlu",
        &["link.asr.nlu"],
    ),
];

pub fn default_config() -> WorkflowConfig {
    let ids = [
        ("wakeup.greeting", "start"),
        ("soul.profile", "link.nlu.llm"),
        ("memory.recall", "link.nlu.llm"),
        ("command.gate", "link.asr.nlu"),
        ("nlu.rules", "nlu"),
        ("memory.writeback", "end"),
    ];
    WorkflowConfig {
        version: 1,
        bindings: ids
            .into_iter()
            .enumerate()
            .map(|(i, (p, pos))| Binding {
                id: format!("default-{i}"),
                plugin_id: p.into(),
                position_id: pos.into(),
                enabled: true,
                order: i as i32,
                condition: Condition::Always,
                on_error: error_options(p)[0].into(),
            })
            .collect(),
    }
}

pub fn parse_or_default(value: &Value) -> Result<WorkflowConfig, String> {
    if value.is_null() {
        return Ok(default_config());
    }
    serde_json::from_value(value.clone()).map_err(|e| format!("workflow 格式错误: {e}"))
}
pub fn compile(value: &Value) -> Result<CompiledWorkflow, String> {
    let cfg = parse_or_default(value)?;
    if cfg.version != 1 {
        return Err(format!("不支持 workflow version={}，仅支持 1", cfg.version));
    }
    if cfg.bindings.len() > 64 {
        return Err("workflow bindings 最多 64 项".into());
    }
    let positions: HashSet<&str> = POSITIONS.iter().map(|x| x.0).collect();
    let mut ids = HashSet::new();
    let mut seen_plugins = HashSet::new();
    let mut plan = Vec::new();
    let mut skill_gate = false;
    let mut command_gate = false;
    let mut nlu_rules = false;
    let mut bindings: Vec<_> = cfg.bindings.iter().collect();
    bindings.sort_by(|a, b| a.id.cmp(&b.id));
    for b in bindings {
        if b.id.trim().is_empty() {
            return Err("binding id 不能为空".into());
        }
        if !ids.insert(b.id.as_str()) {
            return Err(format!("重复 binding id: {}", b.id));
        }
        let Some((_, _, owner, allowed)) = PLUGINS.iter().find(|x| x.0 == b.plugin_id) else {
            if crate::plugins::registry::find(&b.plugin_id).is_some() {
                return Err(format!(
                    "插件 {} 是能力/核心引擎描述符，不可作为 workflow binding",
                    b.plugin_id
                ));
            }
            return Err(format!("未知 plugin_id: {}", b.plugin_id));
        };
        if !positions.contains(b.position_id.as_str()) {
            return Err(format!("非法 position_id: {}", b.position_id));
        }
        if allowed.is_empty() {
            return Err(format!(
                "插件 {} 是核心引擎描述符，不可作为 workflow binding",
                b.plugin_id
            ));
        }
        if !allowed.contains(&b.position_id.as_str()) && b.position_id != *owner {
            return Err(format!(
                "插件 {} 不允许挂载到 {}",
                b.plugin_id, b.position_id
            ));
        }
        if !error_options(&b.plugin_id).contains(&b.on_error.as_str()) {
            return Err(format!(
                "插件 {} 的 on_error 仅支持 {}",
                b.plugin_id,
                error_options(&b.plugin_id).join(" / ")
            ));
        }
        let kind = match b.condition {
            Condition::Always => "always",
            Condition::TextContains { .. } => "text_contains",
            Condition::DeviceKind { .. } => "device_kind",
            Condition::Confidence { .. } => "confidence",
        };
        if !condition_kinds(&b.plugin_id).contains(&kind) {
            return Err(format!("{} 不支持该 condition kind", b.plugin_id));
        }
        match &b.condition {
            Condition::TextContains { text, .. } if text.trim().is_empty() => {
                return Err("text_contains 文本不能为空".into());
            }
            Condition::DeviceKind { value }
                if !matches!(value.as_str(), "xiaozhi_esp" | "echomuse") =>
            {
                return Err("device_kind 仅支持 xiaozhi_esp / echomuse".into());
            }
            _ => {}
        }
        if let Condition::Confidence { min, max } = b.condition {
            if !min.is_finite()
                || min < 0.0
                || min > 1.0
                || max.is_some_and(|x| !x.is_finite() || x < 0.0 || x > 1.0)
                || max.is_some_and(|x| min > x)
            {
                return Err("confidence 必须在 0..1 且 min 不大于 max".into());
            }
        }
        if !seen_plugins.insert(b.plugin_id.as_str()) {
            return Err(format!("插件不可重复声明: {}", b.plugin_id));
        }
        if !b.enabled {
            continue;
        }
        match b.plugin_id.as_str() {
            "skill.gate" => skill_gate = true,
            "command.gate" => command_gate = true,
            "nlu.rules" => nlu_rules = true,
            _ => {}
        }
        if skill_gate && (command_gate || nlu_rules) {
            return Err("skill.gate 不可与 command.gate/nlu.rules 同时启用，避免重复执行".into());
        }
        plan.push(CompiledBinding {
            plugin_id: b.plugin_id.clone(),
            position_id: b.position_id.clone(),
            order: b.order,
            condition: b.condition.clone(),
            on_error: b.on_error.clone(),
        });
    }
    plan.sort_by_key(|x| {
        (
            POSITIONS
                .iter()
                .position(|p| p.0 == x.position_id)
                .unwrap_or(usize::MAX),
            x.order,
        )
    });
    Ok(CompiledWorkflow { config: cfg, plan })
}

fn safety_gate(id: &str) -> bool {
    matches!(id, "command.gate" | "nlu.rules" | "skill.gate")
}

fn condition_kinds(id: &str) -> &'static [&'static str] {
    if safety_gate(id) {
        &["always", "text_contains", "confidence"]
    } else {
        &["always", "device_kind"]
    }
}

fn error_options(id: &str) -> &'static [&'static str] {
    if safety_gate(id) {
        &["stop"]
    } else {
        &["continue", "stop"]
    }
}

fn condition_json() -> Value {
    json!([
        {"kind":"always","label":"始终","fields":[]},
        {"kind":"text_contains","label":"文本包含","fields":[{"key":"text","label":"文本","type":"string","required":true},{"key":"case_sensitive","label":"区分大小写","type":"boolean","required":false}]},
        {"kind":"device_kind","label":"设备类型","fields":[{"key":"value","label":"设备类型值","type":"string","required":true,"options":["xiaozhi_esp","echomuse"]}]},
        {"kind":"confidence","label":"置信度","fields":[{"key":"min","label":"最低值","type":"number","required":true},{"key":"max","label":"最高值","type":"number","required":false}]}
    ])
}
fn plugin_json((id, label, owner, allowed): &(&str, &str, &str, &[&str])) -> Value {
    let runtime = !allowed.is_empty();
    let config_paths = match *id {
        "soul.profile" => vec!["soul"],
        "memory.recall" | "memory.writeback" => vec!["memory"],
        "wakeup.greeting" => vec!["wakeup"],
        "command.gate" => vec!["command"],
        "nlu.rules" => vec!["nlu"],
        _ => vec![],
    };
    json!({"id":id,"label":label,"owner_position":owner,"allowed_positions":if runtime { *allowed } else { &[] as &[&str] },"config_paths":config_paths,"input_type":"event","output_type":"event","repeatable":false,"order_constraints":[],"tests":[],"actions":[],"condition_kinds":if runtime { condition_kinds(id) } else { &[] as &[&str] },"on_error_options":if runtime { error_options(id) } else { &[] as &[&str] }})
}
fn core_plugins_json() -> Vec<Value> {
    use crate::plugins::registry::{Capability, REGISTRY};
    REGISTRY.iter().filter_map(|plugin| {
        let (owner, input, output) = match plugin.capability {
            Capability::Vad => ("vad", "audio_frames", "speech_segment"),
            Capability::Asr => ("asr", "speech_segment", "text"),
            Capability::Llm => ("llm", "turn_context", "response"),
            Capability::Tts => ("tts", "text", "audio"),
            Capability::FullChain => ("start", "audio_frames", "audio"),
            _ => return None,
        };
        Some(json!({"id":plugin.id,"label":plugin.display,"owner_position":owner,"allowed_positions":[],"config_paths":[plugin.fields_path.join(".")],"input_type":input,"output_type":output,"repeatable":false,"hot":plugin.hot.as_str(),"condition_kinds":[],"on_error_options":[],"order_constraints":[],"tests":[],"actions":[]}))
    }).collect()
}

pub fn projection(value: &Value, revision: String) -> Value {
    projection_with_mode(value, revision, "local")
}

pub fn projection_with_mode(value: &Value, revision: String, mode: &str) -> Value {
    let compiled = match compile(value) {
        Ok(compiled) => compiled,
        Err(error) => {
            return json!({"version":1,"valid":false,"revision":revision,"mode":mode,"error":error})
        }
    };
    let plan: Vec<Value> = compiled.plan.iter().map(|b| json!({"plugin_id":b.plugin_id,"position_id":b.position_id,"order":b.order,"on_error":b.on_error,"condition":b.condition})).collect();
    let mut plugins: Vec<Value> = PLUGINS.iter().map(plugin_json).collect();
    plugins.extend(core_plugins_json());
    json!({"version":1,"revision":revision,"mode":mode,"hot":"next_session","condition_kinds":condition_json(),"positions":POSITIONS.iter().map(|(id,label,kind)| json!({"id":id,"label":label,"kind":kind})).collect::<Vec<_>>(),"plugins":plugins,"config":compiled.config,"plan":plan})
}

#[cfg(test)]
mod tests;
