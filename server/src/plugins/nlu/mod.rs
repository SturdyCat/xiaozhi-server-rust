//! 技能（统一概念）目录配置与同技能槽位模板。
//!
//! 技能只能由真实分类器选择；模板不识别技能、不提供规则/启发式回退。
//! 每个技能 = 名称（训练标签）+ 控制域 + 例句模板 + 动作；保存的同名自定义配置完整覆盖初始项。
//! 控制域只描述分组与执行者（device 设备固件 / server 服务端 / iot 第三方），不进训练输入。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::plugins::command::normalize_utterance;

pub(crate) mod matcher;
pub(crate) mod template;
#[cfg(test)]
mod tests;

pub(crate) mod bert;
#[cfg(feature = "nlu")]
#[allow(unused_imports)]
pub use bert::TinyBertNlu;
#[allow(unused_imports)]
pub use bert::{bert_feature_enabled, bert_signature, build_bert_checked, BertClassify, BertHit};
pub use template::parse_template;

/// 槽位约束：空 values 表示开放文本；只针对模型已经接受的同一技能抽取。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SlotList {
    pub name: String,
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NluConfig {
    /// 自定义与初始技能目录开关；command 的目录开关独立，但都必须通过真实模型。
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_intents")]
    pub intents: Vec<IntentDef>,
    #[serde(default = "default_slots")]
    pub slots: Vec<SlotList>,
    /// 旧配置兼容字段，不再参与识别或任何回退。
    #[serde(default = "default_model_threshold")]
    pub model_threshold: f32,
    /// 旧配置兼容字段，不再参与识别或任何回退。
    #[serde(default = "default_model_margin")]
    pub model_margin: f32,
    /// 唯一意图分类器开关。关闭时不识别技能；启用却不可用时失败关闭。
    #[serde(default)]
    pub bert_enabled: bool,
    #[serde(default)]
    pub bert_model: String,
    #[serde(default)]
    pub bert_vocab: String,
    #[serde(default)]
    pub bert_labels: String,
    #[serde(default = "default_bert_threshold")]
    pub bert_threshold: f32,
    #[serde(default = "default_bert_num_threads")]
    pub bert_num_threads: usize,
    #[serde(default = "default_bert_max_len")]
    pub bert_max_len: usize,
    #[serde(default)]
    pub bert_label_map: HashMap<String, String>,
}

impl Default for NluConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            intents: default_intents(),
            slots: default_slots(),
            model_threshold: default_model_threshold(),
            model_margin: default_model_margin(),
            bert_enabled: false,
            bert_model: String::new(),
            bert_vocab: String::new(),
            bert_labels: String::new(),
            bert_threshold: default_bert_threshold(),
            bert_num_threads: default_bert_num_threads(),
            bert_max_len: default_bert_max_len(),
            bert_label_map: HashMap::new(),
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_model_threshold() -> f32 {
    0.9
}
fn default_model_margin() -> f32 {
    0.1
}
fn default_bert_threshold() -> f32 {
    0.75
}
fn default_bert_num_threads() -> usize {
    1
}
fn default_bert_max_len() -> usize {
    64
}

impl NluConfig {
    pub fn bert_configured(&self) -> bool {
        self.bert_enabled
            && !self.bert_model.trim().is_empty()
            && !self.bert_vocab.trim().is_empty()
            && !self.bert_labels.trim().is_empty()
    }

    pub fn bert_map_label<'a>(&'a self, label: &'a str) -> &'a str {
        self.bert_label_map
            .get(label)
            .map(String::as_str)
            .unwrap_or(label)
    }

    /// 旧调用兼容；默认技能只在首次缺省配置时播种，不向保存态隐藏注入。
    /// 生产/训练/候选使用 skill::effective_catalog，以同时包含 command 词表。
    pub fn with_builtins(&self) -> NluConfig {
        self.clone()
    }
}

/// 技能控制域：技能的一级分组 = 控制目标类别。
///
/// 按**执行者**分域：设备固件执行的归 [`Domain::Device`]（含 end_session 断开对话、
/// 音量/播放控制），服务端执行的归 [`Domain::Server`]（提醒/天气类 plan 技能），
/// 第三方智能设备（tuya 等）归 [`Domain::Iot`]（iot_device_on/off 语音开关）。
/// 枚举即白名单：非法值直接反序列化失败，进入既有"保存态无效"链路。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Domain {
    /// 服务端执行的技能（缺省：现网保存态只有 reminder_* 等服务端技能）。
    #[default]
    Server,
    /// 设备固件执行的技能（MCP 工具直调、end_session 断开对话）。
    Device,
    /// 第三方智能设备域（tuya 等）：语音开关已落地（iot_device_on/off）。
    Iot,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct IntentDef {
    pub name: String,
    #[serde(default)]
    pub templates: Vec<String>,
    #[serde(default)]
    pub action: ActionDef,
    /// 技能控制域。旧保存态/旧 manifest 缺该字段时缺省 [`Domain::Server`]，
    /// 与"现网保存态只有服务端技能"的事实一致；不迁移、不改写已保存 JSON。
    #[serde(default)]
    pub domain: Domain,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ActionDef {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub reply: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<crate::skills::SkillPlan>,
}

pub(crate) fn norm(s: &str) -> String {
    normalize_utterance(s)
        .split_whitespace()
        .collect::<String>()
}

/// 纯初始数据；不能在这里硬编码提醒动作或偷偷迁移保存态 action。
pub(crate) fn builtin_defs() -> (Vec<IntentDef>, Vec<SlotList>) {
    #[derive(Deserialize)]
    struct Defaults {
        intents: Vec<IntentDef>,
        slots: Vec<SlotList>,
    }
    let defaults: Defaults = serde_json::from_str(include_str!("defaults.json"))
        .expect("bundled initial skill JSON must be valid");
    (defaults.intents, defaults.slots)
}

fn default_intents() -> Vec<IntentDef> {
    builtin_defs().0
}
fn default_slots() -> Vec<SlotList> {
    builtin_defs().1
}
