use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{atomic::AtomicBool, Arc};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SkillPlan {
    #[serde(default)]
    pub conditions: Vec<SkillCondition>,
    pub steps: Vec<SkillStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillCondition {
    SlotPresent { slot: String },
    SlotEquals { slot: String, value: String },
    DeviceKind { value: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SkillStep {
    pub api: String,
    #[serde(default = "empty_object")]
    pub args: Value,
}
fn empty_object() -> Value {
    serde_json::json!({})
}

#[derive(Debug, Clone)]
pub struct SkillExecutionContext {
    /// Authenticated/bound transport device; never copied from model slots.
    pub device_id: String,
    pub session_id: String,
    /// Stable across retries, unique for each logical step (e.g. turn UUID:index).
    pub request_id: String,
    pub device_kind: Option<String>,
    pub now_ms: i64,
    pub cancelled: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeliveryClaim {
    pub id: String,
    pub task_id: Option<String>,
    pub device_id: String,
    pub session_id: String,
    pub text: String,
    pub lease_token: String,
    pub lease_until_ms: i64,
}

/// Safe clarification speech. Errors abort the plan rather than masquerading as a successful step.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{speech}")]
pub struct SkillActionError {
    pub speech: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityMetadata {
    pub api: &'static str,
    pub description: &'static str,
    pub execution: &'static str,
    pub args_schema: Value,
}
