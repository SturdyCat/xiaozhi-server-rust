//! pure-base 下的唤醒问候兼容接口。
//!
//! 唤醒问候是业务级预制话术通道（三级音频复用 + 磁盘缓存），不属于纯底座
//! 核心 ASR/VAD/TTS/LLM 数据面；pure-base 不编译问候快照与会话问候路径，
//! 设备唤醒后直接静默等待第一句话（与 `[wakeup]` 插件不存在时一致）。
//!
//! 兼容承诺：`[wakeup]` 配置段保持原字段与纯配置校验可解析（旧配置库无损读取，
//! 注册表描述符与节点测试预览继续工作），但不再产生任何问候行为。

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// 默认问候语（配置解析兼容；pure-base 不会合成或下发它）。
pub const DEFAULT_GREETING: &str = "我在";
/// 单句问候语软上限（配置校验兼容）。
pub const GREETING_SOFT_LIMIT: usize = 30;

fn default_greeting() -> String {
    DEFAULT_GREETING.to_string()
}
fn default_true() -> bool {
    true
}

/// `[wakeup]` 段：唤醒问候语的配置（pure-base 仅保留解析兼容形状）。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WakeConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_greeting")]
    pub greeting: String,
    #[serde(default = "default_true")]
    pub persist: bool,
}

impl Default for WakeConfig {
    fn default() -> Self {
        WakeConfig {
            enabled: true,
            greeting: default_greeting(),
            persist: true,
        }
    }
}

/// 拆问候语池：换行分隔、逐行 trim、丢空行（纯配置解析，无行为）。
fn split_pool(s: &str) -> Vec<String> {
    s.split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

impl WakeConfig {
    /// 规范化（幂等；`Config::normalize` 三条路径都会调）。
    pub fn normalize(&mut self) {
        self.greeting = split_pool(&self.greeting).join("\n");
    }

    /// 问候语池：按换行拆分、逐行去首尾空白、丢空行。
    pub fn pool(&self) -> Vec<String> {
        split_pool(&self.greeting)
    }

    /// 是否会问候（pure-base 恒不产生问候行为，但配置语义保持真实）。
    pub fn active(&self) -> bool {
        self.enabled && !self.pool().is_empty()
    }

    /// 热切换签名（注册表状态上报用）。
    pub fn signature(&self) -> String {
        format!("{}|{}|{}", self.enabled, self.greeting, self.persist)
    }

    /// 段内自洽校验（纯配置检查；跨段矛盾由注册表 `validate_wake` 判定）。
    pub fn validate(&self) -> Result<()> {
        let lines = self.pool();
        if self.enabled && lines.is_empty() {
            anyhow::bail!(
                "[wakeup].greeting 为空：启用唤醒问候语时请填至少一句问候语（如 \"我在\"），\
                 或把 [wakeup].enabled 改回 false（唤醒后静默等待，与没有这个插件一致）。"
            );
        }
        for line in &lines {
            let n = line.chars().count();
            if self.enabled && n > GREETING_SOFT_LIMIT {
                anyhow::bail!(
                    "[wakeup].greeting 有一句 {n} 个字（建议 ≤ {GREETING_SOFT_LIMIT}）：\
                     {line:?}。问候语播放期间上行音频会在缓冲区排队，太长会推迟用户第一句话的\
                     处理。建议改成「我在」这类短句。"
                );
            }
        }
        Ok(())
    }
}
