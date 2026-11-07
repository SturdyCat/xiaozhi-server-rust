//! Bounded policy bridges for wakeup, soul, and Tuya.

use serde::Deserialize;
use serde_json::{json, Value};

use super::super::registry::WasmHost;
use super::Resolution;

// P7a：wakeup strategy（native greeting catalog remains authoritative）
// ============================================================

const MAX_WAKEUP_CATALOG: usize = 64;
const MAX_WAKEUP_TEXT_BYTES: usize = 256;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct WakeupPrepareResult {
    pub greeting_index: usize,
    #[serde(default)]
    pub route: Option<String>,
}

/// Ask a WASM policy to select one entry from the native wakeup catalog.
/// No arbitrary text, audio, cache key, or TTS operation crosses the boundary.
pub fn wakeup_prepare(
    host: &WasmHost,
    greetings: &[String],
    persist: bool,
) -> std::result::Result<usize, String> {
    if greetings.is_empty() {
        return Err("唤醒问候目录为空".to_string());
    }
    if greetings.len() > MAX_WAKEUP_CATALOG {
        return Err(format!(
            "唤醒问候目录超限（{} > {MAX_WAKEUP_CATALOG}）",
            greetings.len()
        ));
    }
    if greetings
        .iter()
        .any(|text| text.len() > MAX_WAKEUP_TEXT_BYTES)
    {
        return Err("唤醒问候目录包含超长文本".to_string());
    }
    let plugin_id = match host.resolve("wakeup.strategy") {
        Resolution::Resolved(id) => id,
        other => return Err(format!("唤醒策略扩展点无可用实现：{other:?}")),
    };
    let req_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({
        "req_id": req_id,
        "greetings": greetings,
        "persist": persist,
    })
    .to_string();
    let errors = host
        .handle_event(&plugin_id, "wakeup.prepare", &payload)
        .map_err(|e| format!("唤醒策略插件事件失败：{e:#}"))?;
    if !errors.is_empty() {
        return Err(format!("唤醒策略效果执行失败：{errors:?}"));
    }
    let bytes = host
        .kv()
        .get(&plugin_id, &format!("wakeup-result:{req_id}"))
        .ok_or_else(|| "唤醒策略插件未返回结果（需要 kv 键 wakeup-result:<req_id>）".to_string())?;
    let result: WakeupPrepareResult = serde_json::from_slice(&bytes)
        .map_err(|e| format!("唤醒策略结果损坏（需要 greeting_index）：{e}"))?;
    if result.greeting_index >= greetings.len() {
        return Err(format!(
            "唤醒策略 greeting_index 越界：{}（候选数 {}）",
            result.greeting_index,
            greetings.len()
        ));
    }
    if let Some(route) = result.route.as_deref() {
        if route != "native" {
            return Err(format!("唤醒策略返回不支持的 route={route:?}"));
        }
    }
    Ok(result.greeting_index)
}

// ============================================================

// P7：prompt.soul 有界控制策略（人格渲染与提示词数据面留 native）
// ============================================================

const MAX_SOUL_DEVICE_ID_BYTES: usize = 128;
const MAX_SOUL_NAME_BYTES: usize = 128;
const MAX_SOUL_ADDRESS_BYTES: usize = 128;
const MAX_SOUL_TONE_BYTES: usize = 256;

/// WASM 人格策略只能覆盖受限风格/回复预算，不得返回完整 instructions、变量或 prompt 片段。
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SoulPrepareResult {
    #[serde(default)]
    pub tone: Option<String>,
    #[serde(default)]
    pub max_sentences: Option<u32>,
    #[serde(default)]
    pub max_chars: Option<u32>,
    #[serde(default)]
    pub route: Option<String>,
}

/// 向 `prompt.soul` 派发一次有界人格策略事件。
///
/// 请求只携带设备标识与已规范化的人格元数据；结果不能携带完整 prompt，且所有
/// 覆盖项都经过宿主范围校验。插件缺失、事件失败、KV 缺失、坏 JSON 或越界结果
/// 均显式失败，绝不回退到未经过策略的文本。
pub fn soul_prepare(
    host: &WasmHost,
    device_id: &str,
    name: &str,
    address_user: &str,
    tone: &str,
    max_sentences: u32,
    max_chars: u32,
) -> std::result::Result<SoulPrepareResult, String> {
    if device_id.trim().is_empty() || device_id.len() > MAX_SOUL_DEVICE_ID_BYTES {
        return Err("Soul 请求 device_id 为空或超长".to_string());
    }
    if name.len() > MAX_SOUL_NAME_BYTES || address_user.len() > MAX_SOUL_ADDRESS_BYTES {
        return Err("Soul 请求人格元数据超长".to_string());
    }
    if tone.len() > MAX_SOUL_TONE_BYTES {
        return Err("Soul 请求 tone 超长".to_string());
    }
    if !(1..=8).contains(&max_sentences) || !(10..=500).contains(&max_chars) {
        return Err("Soul 请求回复预算非法".to_string());
    }
    let plugin_id = match host.resolve("prompt.soul") {
        Resolution::Resolved(id) => id,
        other => return Err(format!("Soul 策略扩展点无可用实现：{other:?}")),
    };
    let req_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({
        "req_id": req_id,
        "device_id": device_id.trim(),
        "name": name,
        "address_user": address_user,
        "tone": tone,
        "max_sentences": max_sentences,
        "max_chars": max_chars,
    })
    .to_string();
    let errors = host
        .handle_event(&plugin_id, "prompt.soul.prepare", &payload)
        .map_err(|e| format!("Soul 策略插件事件失败：{e:#}"))?;
    if !errors.is_empty() {
        return Err(format!("Soul 策略效果执行失败：{errors:?}"));
    }
    let bytes = host
        .kv()
        .get(&plugin_id, &format!("soul-result:{req_id}"))
        .ok_or_else(|| "Soul 策略插件未返回结果".to_string())?;
    let result: SoulPrepareResult =
        serde_json::from_slice(&bytes).map_err(|e| format!("Soul 策略结果损坏：{e}"))?;
    if let Some(route) = result.route.as_deref() {
        if route != "native" {
            return Err(format!("Soul 策略返回不支持的 route={route:?}"));
        }
    }
    if let Some(tone) = result.tone.as_deref() {
        if tone.len() > MAX_SOUL_TONE_BYTES {
            return Err("Soul 策略返回 tone 超长".to_string());
        }
    }
    if let Some(value) = result.max_sentences {
        if !(1..=8).contains(&value) {
            return Err(format!("Soul 策略 max_sentences 非法：{value}"));
        }
    }
    if let Some(value) = result.max_chars {
        if !(10..=500).contains(&value) {
            return Err(format!("Soul 策略 max_chars 非法：{value}"));
        }
    }
    Ok(result)
}

// ============================================================

// P5：connector.tuya 控制策略桥（目录选择过 wasm，LAN 数据面留 native）
// ============================================================

const MAX_TUYA_NAME_BYTES: usize = 256;
const MAX_TUYA_DEVICES: usize = 128;

/// WASM Tuya 策略返回的候选索引；local_key、IP、原始 DP 和 socket 句柄永不跨边界。
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct TuyaPrepareResult {
    pub candidate_index: usize,
    pub action: String,
    #[serde(default)]
    pub route: Option<String>,
}

/// 向 `connector.tuya` 派发有限设备目录元数据，读取 native 控制决策。
/// wrapper 随后校验索引和 action，再由 `TuyaEngine` 执行 rustuya LAN 操作。
pub fn tuya_prepare(
    host: &WasmHost,
    name: &str,
    action: &str,
    candidates: &[Value],
) -> std::result::Result<TuyaPrepareResult, String> {
    if name.trim().is_empty() || name.len() > MAX_TUYA_NAME_BYTES {
        return Err("Tuya 请求设备名为空或超长".to_string());
    }
    if !matches!(action, "on" | "off") {
        return Err("Tuya 请求 action 只允许 on/off".to_string());
    }
    if candidates.len() > MAX_TUYA_DEVICES {
        return Err(format!(
            "Tuya 候选目录超限（{} > {MAX_TUYA_DEVICES}）",
            candidates.len()
        ));
    }
    let plugin_id = match host.resolve("connector.tuya") {
        Resolution::Resolved(id) => id,
        other => return Err(format!("Tuya 策略扩展点无可用实现：{other:?}")),
    };
    let req_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({
        "req_id": req_id,
        "name": name.trim(),
        "action": action,
        "candidates": candidates,
    })
    .to_string();
    let errors = host
        .handle_event(&plugin_id, "connector.tuya.prepare", &payload)
        .map_err(|e| format!("Tuya 策略插件事件失败：{e:#}"))?;
    if !errors.is_empty() {
        return Err(format!("Tuya 策略效果执行失败：{errors:?}"));
    }
    let bytes = host
        .kv()
        .get(&plugin_id, &format!("tuya-result:{req_id}"))
        .ok_or_else(|| "Tuya 策略插件未返回结果（需要 kv 键 tuya-result:<req_id>）".to_string())?;
    let result: TuyaPrepareResult = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Tuya 策略结果损坏（需要 candidate_index/action）：{e}"))?;
    if result.candidate_index >= candidates.len() {
        return Err(format!(
            "Tuya 策略 candidate_index 越界：{}（候选数 {}）",
            result.candidate_index,
            candidates.len()
        ));
    }
    if result.action != action || !matches!(result.action.as_str(), "on" | "off") {
        return Err("Tuya 策略返回的 action 非本轮请求的 on/off".to_string());
    }
    if let Some(route) = result.route.as_deref() {
        if route != "native" {
            return Err(format!("Tuya 策略返回不支持的 route={route:?}"));
        }
    }
    Ok(result)
}

// ============================================================
// P7d：debug domain bounded policy (node tests/log stream/bench stay native)
// ============================================================

const MAX_DEBUG_TARGET_BYTES: usize = 128;
const MAX_DEBUG_SESSION_BYTES: usize = 128;
const MAX_DEBUG_ITEMS: u32 = 256;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct DebugPrepareResult {
    #[serde(default = "default_debug_items")]
    pub max_items: u32,
    #[serde(default = "default_debug_redact")]
    pub redact: bool,
    #[serde(default)]
    pub route: Option<String>,
}

fn default_debug_items() -> u32 {
    64
}

fn default_debug_redact() -> bool {
    true
}

/// Ask a WASM policy for bounded debug presentation controls. Raw logs,
/// node-test inputs/results, benchmark samples, credentials, and sockets remain
/// native; the policy only chooses safe item/redaction limits.
pub fn debug_prepare(
    host: &WasmHost,
    kind: &str,
    target: &str,
    session_id: Option<&str>,
    requested_items: u32,
) -> std::result::Result<DebugPrepareResult, String> {
    if !matches!(kind, "node-test" | "log-stream" | "test-bench") {
        return Err("debug 策略 kind 不受支持".to_string());
    }
    if target.trim().is_empty() || target.len() > MAX_DEBUG_TARGET_BYTES {
        return Err("debug 策略 target 为空或超长".to_string());
    }
    if session_id.is_some_and(|id| id.len() > MAX_DEBUG_SESSION_BYTES) {
        return Err("debug 策略 session_id 超长".to_string());
    }
    if !(1..=MAX_DEBUG_ITEMS).contains(&requested_items) {
        return Err(format!(
            "debug 策略 requested_items 非法（1..={MAX_DEBUG_ITEMS}）"
        ));
    }
    let plugin_id = match host.resolve("debug.policy") {
        Resolution::Resolved(id) => id,
        other => return Err(format!("debug 策略扩展点无可用实现：{other:?}")),
    };
    let req_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({
        "req_id": req_id,
        "kind": kind,
        "target": target.trim(),
        "session_id": session_id,
        "requested_items": requested_items,
    })
    .to_string();
    let errors = host
        .handle_event(&plugin_id, &format!("debug.{kind}.prepare"), &payload)
        .map_err(|e| format!("debug 策略插件事件失败：{e:#}"))?;
    if !errors.is_empty() {
        return Err(format!("debug 策略效果执行失败：{errors:?}"));
    }
    let bytes = host
        .kv()
        .get(&plugin_id, &format!("debug-result:{req_id}"))
        .ok_or_else(|| "debug 策略插件未返回结果".to_string())?;
    let result: DebugPrepareResult =
        serde_json::from_slice(&bytes).map_err(|e| format!("debug 策略结果损坏：{e}"))?;
    if !(1..=MAX_DEBUG_ITEMS).contains(&result.max_items) || result.max_items > requested_items {
        return Err(format!(
            "debug 策略 max_items 非法：{}（请求上限 {requested_items}）",
            result.max_items
        ));
    }
    if result
        .route
        .as_deref()
        .is_some_and(|route| route != "native")
    {
        return Err("debug 策略返回不支持的 route".to_string());
    }
    Ok(result)
}

// ============================================================
// P7e：skills bounded policy (native plan validation/execution stays native)
// ============================================================

const MAX_SKILL_ID_BYTES: usize = 128;
const MAX_SKILL_SLOTS: usize = 16;
const MAX_SKILL_SLOT_KEY_BYTES: usize = 64;
const MAX_SKILL_SLOT_VALUE_BYTES: usize = 128;
const MAX_SKILL_STEPS: u32 = 32;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SkillPrepareResult {
    #[serde(default = "default_skill_allow")]
    pub allow: bool,
    #[serde(default = "default_skill_steps")]
    pub max_steps: u32,
    #[serde(default)]
    pub route: Option<String>,
}

fn default_skill_allow() -> bool {
    false
}

fn default_skill_steps() -> u32 {
    1
}

/// Ask a WASM policy for bounded skill-plan controls. Skill definitions,
/// capability validation, interpolation, MCP calls, task persistence, and
/// execution remain native; only a small admission/step budget is delegated.
pub fn skill_prepare(
    host: &WasmHost,
    skill_id: &str,
    slots: &[(String, String)],
    requested_steps: u32,
) -> std::result::Result<SkillPrepareResult, String> {
    if skill_id.trim().is_empty() || skill_id.len() > MAX_SKILL_ID_BYTES {
        return Err("skill 策略 skill_id 为空或超长".to_string());
    }
    if slots.len() > MAX_SKILL_SLOTS {
        return Err(format!(
            "skill 策略 slots 超限（{} > {MAX_SKILL_SLOTS}）",
            slots.len()
        ));
    }
    for (key, value) in slots {
        if key.is_empty()
            || key.len() > MAX_SKILL_SLOT_KEY_BYTES
            || value.len() > MAX_SKILL_SLOT_VALUE_BYTES
            || key.chars().any(char::is_control)
            || value.chars().any(char::is_control)
        {
            return Err("skill 策略 slot 超长、为空或含控制字符".to_string());
        }
    }
    if !(1..=MAX_SKILL_STEPS).contains(&requested_steps) {
        return Err(format!(
            "skill 策略 requested_steps 非法（1..={MAX_SKILL_STEPS}）"
        ));
    }
    let plugin_id = match host.resolve("skill.policy") {
        Resolution::Resolved(id) => id,
        other => return Err(format!("skill 策略扩展点无可用实现：{other:?}")),
    };
    let req_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({
        "req_id": req_id,
        "skill_id": skill_id.trim(),
        "slots": slots.iter().map(|(key, value)| json!({ "key": key, "value": value })).collect::<Vec<_>>(),
        "requested_steps": requested_steps,
    })
    .to_string();
    let errors = host
        .handle_event(&plugin_id, "skill.policy.prepare", &payload)
        .map_err(|e| format!("skill 策略插件事件失败：{e:#}"))?;
    if !errors.is_empty() {
        return Err(format!("skill 策略效果执行失败：{errors:?}"));
    }
    let bytes = host
        .kv()
        .get(&plugin_id, &format!("skill-result:{req_id}"))
        .ok_or_else(|| "skill 策略插件未返回结果".to_string())?;
    let result: SkillPrepareResult =
        serde_json::from_slice(&bytes).map_err(|e| format!("skill 策略结果损坏：{e}"))?;
    if !(1..=MAX_SKILL_STEPS).contains(&result.max_steps) || result.max_steps > requested_steps {
        return Err(format!(
            "skill 策略 max_steps 非法：{}（请求上限 {requested_steps}）",
            result.max_steps
        ));
    }
    if result
        .route
        .as_deref()
        .is_some_and(|route| route != "native")
    {
        return Err("skill 策略返回不支持的 route".to_string());
    }
    Ok(result)
}

// ============================================================
// P7f：memory.policy bounded recall controls (storage/extraction stay native)
// ============================================================

const MAX_MEMORY_DEVICE_BYTES: usize = 128;
const MAX_MEMORY_QUERY_BYTES: usize = 512;
const MAX_MEMORY_ITEMS: u32 = 32;
const MAX_MEMORY_CHARS: u32 = 4096;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct MemoryPrepareResult {
    #[serde(default = "default_memory_allow")]
    pub allow: bool,
    #[serde(default = "default_memory_items")]
    pub max_items: u32,
    #[serde(default = "default_memory_chars")]
    pub max_chars: u32,
    #[serde(default)]
    pub route: Option<String>,
}

fn default_memory_allow() -> bool {
    false
}

fn default_memory_items() -> u32 {
    8
}

fn default_memory_chars() -> u32 {
    1024
}

/// Ask a WASM policy for bounded memory recall controls. SQLite facts, graph/
/// store/rag indexing, extraction, retention, and final prompt injection remain
/// native; the policy only gates recall and clamps its budget.
pub fn memory_prepare(
    host: &WasmHost,
    device_id: &str,
    query: &str,
    requested_items: u32,
    requested_chars: u32,
) -> std::result::Result<MemoryPrepareResult, String> {
    if device_id.trim().is_empty() || device_id.len() > MAX_MEMORY_DEVICE_BYTES {
        return Err("memory 策略 device_id 为空或超长".to_string());
    }
    if query.trim().is_empty() || query.len() > MAX_MEMORY_QUERY_BYTES {
        return Err("memory 策略 query 为空或超长".to_string());
    }
    if !(1..=MAX_MEMORY_ITEMS).contains(&requested_items)
        || !(1..=MAX_MEMORY_CHARS).contains(&requested_chars)
    {
        return Err("memory 策略请求预算超限".to_string());
    }
    let plugin_id = match host.resolve("memory.policy") {
        Resolution::Resolved(id) => id,
        other => return Err(format!("memory 策略扩展点无可用实现：{other:?}")),
    };
    let req_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({
        "req_id": req_id,
        "device_id": device_id.trim(),
        "query": query.trim(),
        "requested_items": requested_items,
        "requested_chars": requested_chars,
    })
    .to_string();
    let errors = host
        .handle_event(&plugin_id, "memory.policy.prepare", &payload)
        .map_err(|e| format!("memory 策略插件事件失败：{e:#}"))?;
    if !errors.is_empty() {
        return Err(format!("memory 策略效果执行失败：{errors:?}"));
    }
    let bytes = host
        .kv()
        .get(&plugin_id, &format!("memory-result:{req_id}"))
        .ok_or_else(|| "memory 策略插件未返回结果".to_string())?;
    let result: MemoryPrepareResult =
        serde_json::from_slice(&bytes).map_err(|e| format!("memory 策略结果损坏：{e}"))?;
    if !(1..=MAX_MEMORY_ITEMS).contains(&result.max_items)
        || result.max_items > requested_items
        || !(1..=MAX_MEMORY_CHARS).contains(&result.max_chars)
        || result.max_chars > requested_chars
    {
        return Err("memory 策略返回预算非法".to_string());
    }
    if result
        .route
        .as_deref()
        .is_some_and(|route| route != "native")
    {
        return Err("memory 策略返回不支持的 route".to_string());
    }
    Ok(result)
}
