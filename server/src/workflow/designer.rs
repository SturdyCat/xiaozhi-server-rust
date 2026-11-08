//! Declarative plugin workflow schema, compiler, and stable API projection.
//! The compiler is intentionally independent of runtime engines: it validates
//! topology and produces an ordered plan; nlu.model is the sole model gate.
//! Legacy intent IDs remain serialization aliases, never rule recognizers.
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

#[cfg(feature = "native-plugins")]
const PLUGINS: &[(&str, &str, &str, &[&str])] = &[
    ("nlu.model", "模型意图", "nlu", &["nlu"]),
    ("skill.gate", "模型门控（兼容）", "nlu", &["nlu"]),
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
    ("nlu.rules", "模型意图（兼容）", "nlu", &["nlu"]),
    (
        "command.gate",
        "模型门控（旧指令兼容）",
        "link.asr.nlu",
        &["link.asr.nlu"],
    ),
];

/// wasm **观察者挂钩**（P2d）可绑定的位置：v1 = 会话已接线的三个 hook 位。
/// 语义 = 旁路观察：收到 `{session_id, device_id, text?}` 上下文事件，返回的效果
/// **异步执行**（fire-and-forget），不阻塞对话轮、不可改写对话流、永不打断轮次。
pub const OBSERVER_POSITIONS: &[&str] = &["start", "link.nlu.llm", "end"];

/// 是否静态已接线插件（观察者预过滤用：会话侧剔除"静态不可识别且未装载"的绑定）。
pub fn is_static_plugin(id: &str) -> bool {
    static_plugin(id).is_some()
}

fn static_plugin(
    id: &str,
) -> Option<&'static (
    &'static str,
    &'static str,
    &'static str,
    &'static [&'static str],
)> {
    #[cfg(not(feature = "native-plugins"))]
    {
        let _ = id;
        None
    }
    #[cfg(feature = "native-plugins")]
    {
        PLUGINS.iter().find(|x| x.0 == id)
    }
}

pub fn default_config() -> WorkflowConfig {
    #[cfg(not(feature = "native-plugins"))]
    return WorkflowConfig {
        version: 1,
        bindings: Vec::new(),
    };

    #[cfg(feature = "native-plugins")]
    {
        let ids = [
            ("wakeup.greeting", "start"),
            ("soul.profile", "link.nlu.llm"),
            ("memory.recall", "link.nlu.llm"),
            ("nlu.model", "nlu"),
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
}

pub fn parse_or_default(value: &Value) -> Result<WorkflowConfig, String> {
    if value.is_null() {
        return Ok(default_config());
    }
    serde_json::from_value(value.clone()).map_err(|e| format!("workflow 格式错误: {e}"))
}
pub fn compile(value: &Value) -> Result<CompiledWorkflow, String> {
    compile_with_observers(value, &[])
}

/// 带观察者目录的编译：`observers` = 已装载 wasm 插件 `(id, 标题)`（feature 由调用方
/// 决定来源，未编入 = 空表 → 只认静态表，行为与 `compile` 一致）。
/// 观察者绑定规则：可绑 [`OBSERVER_POSITIONS`] 任一位；`on_error` 仅 `continue`
/// （旁路派发永不打断对话轮）；condition 同普通 hook（always/device_kind）。
pub fn compile_with_observers(
    value: &Value,
    observers: &[(String, String)],
) -> Result<CompiledWorkflow, String> {
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
    let mut bindings: Vec<_> = cfg.bindings.iter().collect();
    bindings.sort_by(|a, b| a.id.cmp(&b.id));
    for b in bindings {
        if b.id.trim().is_empty() {
            return Err("binding id 不能为空".into());
        }
        if !ids.insert(b.id.as_str()) {
            return Err(format!("重复 binding id: {}", b.id));
        }
        // 静态插件 → 原 position/on_error 校验；wasm 观察者 → 钉死 hook 位 + continue；
        // 其余 → 能力描述符/未知 id 报错（保存门不变）
        let static_entry = static_plugin(&b.plugin_id);
        let is_observer =
            static_entry.is_none() && observers.iter().any(|(id, _)| *id == b.plugin_id);
        if let Some((_, _, owner, allowed)) = static_entry {
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
        } else if is_observer {
            if !OBSERVER_POSITIONS.contains(&b.position_id.as_str()) {
                return Err(format!(
                    "wasm 观察者插件 {} 仅可绑定 hook 位 {:?}（当前 {}）",
                    b.plugin_id, OBSERVER_POSITIONS, b.position_id
                ));
            }
            if b.on_error != "continue" {
                return Err(format!(
                    "观察者插件 {} 的 on_error 仅支持 continue（旁路派发永不打断对话轮）",
                    b.plugin_id
                ));
            }
        } else if crate::plugins::registry::find(&b.plugin_id).is_some() {
            return Err(format!(
                "插件 {} 是能力/核心引擎描述符，不可作为 workflow binding",
                b.plugin_id
            ));
        } else {
            return Err(format!("未知 plugin_id: {}", b.plugin_id));
        }
        if !positions.contains(b.position_id.as_str()) {
            return Err(format!("非法 position_id: {}", b.position_id));
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
    matches!(
        id,
        "nlu.model" | "command.gate" | "nlu.rules" | "skill.gate"
    )
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
#[cfg(feature = "native-plugins")]
fn plugin_json((id, label, owner, allowed): &(&str, &str, &str, &[&str])) -> Value {
    let runtime = !allowed.is_empty();
    let config_paths = match *id {
        "soul.profile" => vec!["soul"],
        "memory.recall" | "memory.writeback" => vec!["memory"],
        "wakeup.greeting" => vec!["wakeup"],
        "nlu.model" | "command.gate" | "nlu.rules" | "skill.gate" => vec!["nlu", "command"],
        _ => vec![],
    };
    json!({"id":id,"label":label,"alias_of":if safety_gate(id) && *id != "nlu.model" { Some("nlu.model") } else { None },"owner_position":owner,"allowed_positions":if runtime { *allowed } else { &[] as &[&str] },"config_paths":config_paths,"input_type":"event","output_type":"event","repeatable":false,"order_constraints":[],"tests":[],"actions":[],"condition_kinds":if runtime { condition_kinds(id) } else { &[] as &[&str] },"on_error_options":if runtime { error_options(id) } else { &[] as &[&str] }})
}
fn static_plugins_json() -> Vec<Value> {
    #[cfg(not(feature = "native-plugins"))]
    {
        Vec::new()
    }
    #[cfg(feature = "native-plugins")]
    {
        PLUGINS.iter().map(plugin_json).collect()
    }
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
    projection_with_observers(value, revision, mode, &[])
}

/// 带观察者目录的投影：流程编辑器可把 wasm 观察者插件拖到 hook 位（编辑器语义
/// 与编译器一致——同一个目录源）。observer 条目 `observer: true`，客户端可区分渲染。
pub fn projection_with_observers(
    value: &Value,
    revision: String,
    mode: &str,
    observers: &[(String, String)],
) -> Value {
    let compiled = match compile_with_observers(value, observers) {
        Ok(compiled) => compiled,
        Err(error) => {
            return json!({"version":1,"valid":false,"revision":revision,"mode":mode,"error":error})
        }
    };
    let plan: Vec<Value> = compiled.plan.iter().map(|b| json!({"plugin_id":b.plugin_id,"position_id":b.position_id,"order":b.order,"on_error":b.on_error,"condition":b.condition})).collect();
    let mut plugins = static_plugins_json();
    plugins.extend(core_plugins_json());
    for (id, title) in observers {
        plugins.push(json!({
            "id": id, "label": title,
            "owner_position": OBSERVER_POSITIONS[0],
            "allowed_positions": OBSERVER_POSITIONS,
            "config_paths": [],
            "input_type": "event", "output_type": "effects",
            "repeatable": false,
            "observer": true,
            "condition_kinds": ["always", "device_kind"],
            "on_error_options": ["continue"],
            "order_constraints": [], "tests": [], "actions": [],
        }));
    }
    json!({"version":1,"revision":revision,"mode":mode,"hot":"next_session","condition_kinds":condition_json(),"positions":POSITIONS.iter().map(|(id,label,kind)| json!({"id":id,"label":label,"kind":kind})).collect::<Vec<_>>(),"plugins":plugins,"config":compiled.config,"plan":plan})
}

#[cfg(all(test, feature = "native-plugins"))]
mod tests;

#[cfg(all(test, not(feature = "native-plugins")))]
mod pure_base_tests;
