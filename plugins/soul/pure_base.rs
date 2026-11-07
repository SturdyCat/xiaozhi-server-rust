//! pure-base 下的人格档案兼容接口。
//!
//! 人格（soul）是业务级上下文生产者：预设表、逐字段覆盖与 prompt 段渲染
//! 属于 legacy-business；纯底座不编译它们，业务人格只能由运行期 WASM 插件提供。
//!
//! 兼容承诺：`[soul]` 配置段保持原字段可解析（旧配置库无损读取）；
//! `instructions` 恒等于原生实现 `enabled=false` 的回滚语义 ——
//! 逐字节返回 `[llm].system_prompt` 原文（人格不参与组装）。

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}
fn default_name() -> String {
    "小智".into()
}
fn default_address_user() -> String {
    "你".into()
}
fn default_max_sentences() -> u32 {
    2
}
fn default_max_chars() -> u32 {
    40
}

/// 人格档案配置（`[soul]`）：pure-base 仅保留解析兼容形状，不执行人格业务。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SoulConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub preset: String,
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default)]
    pub self_intro: String,
    #[serde(default)]
    pub form: String,
    #[serde(default)]
    pub age_feel: String,
    #[serde(default)]
    pub worldview: String,
    #[serde(default)]
    pub backstory: String,
    #[serde(default)]
    pub relationship_origin: String,
    #[serde(default)]
    pub traits: Vec<String>,
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub boundaries: Vec<String>,
    #[serde(default)]
    pub tone: String,
    #[serde(default = "default_true")]
    pub colloquial: bool,
    #[serde(default = "default_address_user")]
    pub address_user: String,
    #[serde(default)]
    pub catchphrases: Vec<String>,
    #[serde(default)]
    pub emoji: bool,
    #[serde(default = "default_max_sentences")]
    pub max_sentences: u32,
    #[serde(default = "default_max_chars")]
    pub max_chars: u32,
    #[serde(default)]
    pub scenario: String,
    #[serde(default)]
    pub device_hint: String,
    #[serde(default)]
    pub examples: Vec<String>,
}

impl Default for SoulConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            preset: String::new(),
            name: default_name(),
            self_intro: String::new(),
            form: String::new(),
            age_feel: String::new(),
            worldview: String::new(),
            backstory: String::new(),
            relationship_origin: String::new(),
            traits: Vec::new(),
            values: Vec::new(),
            boundaries: Vec::new(),
            tone: String::new(),
            colloquial: true,
            address_user: default_address_user(),
            catchphrases: Vec::new(),
            emoji: false,
            max_sentences: default_max_sentences(),
            max_chars: default_max_chars(),
            scenario: String::new(),
            device_hint: String::new(),
            examples: Vec::new(),
        }
    }
}

impl SoulConfig {
    /// 签名（注册表热刷新判定用；pure-base 无业务字段语义，仅形状签名）。
    pub fn signature(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// pure-base 无预设表：保持字段原样（幂等）。
    pub fn normalize(&mut self) {}

    /// pure-base 不执行人格业务：配置恒合法。
    pub fn validate(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// 组装系统提示词：人格业务未编译，恒回滚到 `[llm].system_prompt` 原文
/// （与原生实现 `[soul].enabled=false` 的输出逐字节一致）。
pub fn instructions(cfg: &crate::config::Config, _device_id: &str) -> anyhow::Result<String> {
    Ok(cfg.llm.system_prompt.clone())
}
