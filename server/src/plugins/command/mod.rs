//! `[command]` 仅配置设备域技能 `end_session`（断开对话）的训练词表、启停与告别语。
//! 词表不再自行识别关键词；所有动作必须由统一目录的真实模型接受。
//! 旧match_mode/max_chars可读取，但不影响识别、训练样本或执行授权。

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MatchMode {
    #[default]
    Exact,
    Contains,
}

pub const DEFAULT_MAX_CHARS: usize = 5;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommandConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 保存态词表 = 设备域技能 end_session（断开对话）的训练样本，直接进入统一训练目录，
    /// 不在运行期做 exact/contains 匹配。
    #[serde(default = "default_keywords")]
    pub keywords: Vec<String>,
    /// 弃用兼容字段，不参与识别。
    #[serde(default)]
    pub match_mode: MatchMode,
    /// 弃用兼容字段，不限制模型输入或训练词表。
    #[serde(default = "default_max_chars")]
    pub max_chars: usize,
    #[serde(default = "default_reply")]
    pub reply: String,
}

impl Default for CommandConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            keywords: default_keywords(),
            match_mode: MatchMode::Exact,
            max_chars: DEFAULT_MAX_CHARS,
            reply: default_reply(),
        }
    }
}
impl CommandConfig {
    /// 保留词表原句，只trim/去空/去重；不得把保存语料偷偷改成识别规则。
    pub fn normalize(&mut self) {
        let mut keywords = Vec::new();
        for keyword in &self.keywords {
            let keyword = keyword.trim();
            if !keyword.is_empty() && !keywords.iter().any(|s| s == keyword) {
                keywords.push(keyword.to_string());
            }
        }
        self.keywords = keywords;
        self.reply = self.reply.trim().to_string();
    }

    pub fn active(&self) -> bool {
        self.enabled
            && self
                .keywords
                .iter()
                .any(|keyword| !keyword.trim().is_empty())
    }

    pub fn signature(&self) -> String {
        // 序列化避免词表分隔字符碰撞；弃用阈值不改变行为。
        serde_json::to_string(&(self.enabled, &self.keywords, &self.reply)).unwrap_or_default()
    }

    pub fn validate(&self) -> Result<()> {
        if self.enabled && !self.active() {
            anyhow::bail!(
                "[command].keywords为空：请填写断开对话技能（end_session，设备域）的训练样例，或关闭[command].enabled"
            );
        }
        if self
            .keywords
            .iter()
            .any(|keyword| keyword.contains('{') || keyword.contains('}'))
        {
            anyhow::bail!("[command].keywords是断开对话技能的完整训练句，不允许槽位占位符，请移除花括号");
        }
        Ok(())
    }
}

/// 旧诊断类型兼容；不再提供关键词动作授权。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandHit {
    pub keyword: String,
    pub reply: String,
}

/// 避免旧诊断调用方无模型时仍匹配命令；生产应使用SkillRegistry::recognize。
#[derive(Debug, Clone)]
pub struct CommandGate;
impl CommandGate {
    pub fn new(_config: &CommandConfig) -> Self {
        Self
    }
    pub fn hit(&self, _text: &str) -> Option<CommandHit> {
        None
    }
}

const TRAILING_PARTICLES: &str = "吧啊呀呢哦嘛哈呐哩了呗啦哇哟噢喔唉诶嗯";
/// 仅用于槽位封闭枚举等价校验，不参与意图选择或训练词表改写。
pub fn normalize_utterance(raw: &str) -> String {
    let kept: String = raw.chars().filter(|c| c.is_alphanumeric()).collect();
    kept.trim_end_matches(|c| TRAILING_PARTICLES.contains(c))
        .to_string()
}

#[derive(Deserialize)]
struct InitialCommand {
    keywords: Vec<String>,
    reply: String,
}
fn initial() -> InitialCommand {
    serde_json::from_str(include_str!("defaults.json")).expect("initial command JSON must be valid")
}
fn default_true() -> bool {
    true
}
fn default_keywords() -> Vec<String> {
    initial().keywords
}
fn default_max_chars() -> usize {
    DEFAULT_MAX_CHARS
}
fn default_reply() -> String {
    initial().reply
}

#[cfg(test)]
mod tests;
