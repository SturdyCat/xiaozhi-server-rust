//! 唤醒问候语（`[wakeup]`）：设备唤醒 → LLM **之前**的一道拦截。
//!
//! ## 为什么需要它
//!
//! 固件在唤醒词命中后**立刻**发一条 `{"type":"listen","state":"detect","text":"<唤醒词>"}`
//! （见固件 `Protocol::SendWakeWordDetected`，由 `CONFIG_SEND_WAKE_WORD_DATA=y` 启用）。
//! 这是"用户刚叫醒我"的唯一可靠信号——比任何 ASR 结果都早。
//!
//! 在加本插件之前，服务器收到这条消息只打一行日志（`listen state=detect`），用户唤醒后
//! 面对的是**完全静默**的等待：他要说完一句话，等 VAD 切句、ASR、LLM 首 token，才第一次
//! 听到声音。把这段沉默换成一句「我在」，是语音助手体感上最廉价的一次提升。
//!
//! ## 为什么必须绕开 LLM（而不是"让大模型回一句"）
//!
//! 唤醒是**全系统最高频**的动作，而「我在」的文本是**固定的**——为一句固定的话付一次
//! LLM 调用（延迟 + 计费 + 不可控的措辞/长度）没有道理。这里直接走**预制话术**通道
//! （[`crate::app::session::reply::speak_canned`] 的同一套下行链路），一次 LLM 都不调。
//!
//! ## 两级音频复用（这是"高频也不贵"的关键）
//!
//! | 层级 | 位置 | 命中时 |
//! |---|---|---|
//! | ① 内存 | [`crate::app::tts_cache`]（跨会话，进程内） | 零合成，直接投递已编码帧 |
//! | ② 磁盘 | [`crate::app::hot_audio`]（`[wakeup].persist`） | 读盘 ~2KB → 暖进 ① → 零合成 |
//!
//! 于是稳态下**每次唤醒都是零 LLM、零 TTS**：第 ① 层在进程内一直有效，第 ② 层让
//! "重启后的第一次唤醒"也不必重新合成。两级都不命中（首次部署 / 换了音色）才真正合成一次，
//! 随后自动落盘。
//!
//! ⚠️ **换音色天然重新获取**：缓存键含引擎签名（含音色凭据）、speaker、speed、下行采样率、
//! 帧长、文本。任一变化键即不同 → 不会命中旧音色的音频，而是重新合成并写一份新文件。
//! 这正是需求里"切换音色，重新获取"的实现方式——不需要任何显式的失效逻辑。
//!
//! ## 生效时机
//!
//! `HotReload::NextSession`：会话开始时按配置快照构造 [`WakeGreeting`]（与 `[command]`
//! 同语义）。默认 `enabled = false`，关闭时**零开销、行为与加它之前完全一致**。

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// 默认问候语。选「我在」是因为它短（约 600ms 音频）、语义中性、不假设用户要做什么。
pub const DEFAULT_GREETING: &str = "我在";

/// 问候语长度软上限（**只用于校验提示**，不是硬闸门）。
///
/// 为什么要提醒：问候语是**下行**音频，而它播放期间上行帧在缓冲区排队（会话主循环正在
/// 这个 await 里）。问候语越长，用户说完的第一句话被推迟处理得越久。一句话的体感上限
/// 在 10 个字上下，「我在」「在呢」「你好呀」都远低于它。
pub const GREETING_SOFT_LIMIT: usize = 30;

/// `[wakeup]` 段：唤醒问候语的配置。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WakeConfig {
    /// 总开关。默认 **false**：不启用时行为与加本插件之前完全一致（唤醒后静默等待）。
    #[serde(default)]
    pub enabled: bool,
    /// 唤醒后立刻说的这句话（跳过 LLM 与正常回复）。
    #[serde(default = "default_greeting")]
    pub greeting: String,
    /// 是否把合成结果**持久化**到磁盘（跨进程重启复用）。
    ///
    /// 默认 true。前提是 `[tts].cache_entries > 0`——持久化复用依赖内存缓存承载成品帧，
    /// 缓存被关掉时无从取到帧，配置校验会把这一组合拦下来（见 [`WakeConfig::validate`] 的调用方）。
    #[serde(default = "default_persist")]
    pub persist: bool,
}

fn default_greeting() -> String {
    DEFAULT_GREETING.to_string()
}

fn default_persist() -> bool {
    true
}

impl Default for WakeConfig {
    fn default() -> Self {
        WakeConfig {
            enabled: false,
            greeting: default_greeting(),
            persist: true,
        }
    }
}

impl WakeConfig {
    /// 规范化（幂等；`Config::load` / `default` / `merge_client_patch` 三条路径都会调）。
    pub fn normalize(&mut self) {
        self.greeting = self.greeting.trim().to_string();
    }

    /// 是否真的会问候：启用 **且** 问候语非空。
    ///
    /// 空问候语不问候（而不是"问候一个空字符串"）——留空是"我要静默"的合法表达，
    /// 配置矛盾另由 [`Self::validate`] 上报为 `Degraded`。
    ///
    /// 判定用 `trim()`：配置可能来自任意入口（`merge_client_patch` / 手改文件），
    /// 不能假定 [`Self::normalize`] 一定先跑过；纯空白必须与空串同义，
    /// 否则"填了几个空格"会变成"问候一个空字符串"这种静默怪象。
    pub fn active(&self) -> bool {
        self.enabled && !self.greeting.trim().is_empty()
    }

    /// 热切换签名（仅内存比较）。三项都改变行为，必须全在。
    pub fn signature(&self) -> String {
        format!("{}|{}|{}", self.enabled, self.greeting, self.persist)
    }

    /// 前置校验：错误文案必须含「怎么修」（对齐注册表 `validate` 约定）。
    ///
    /// 这里只做**本段内**的自洽检查；跨段矛盾（`persist` 依赖 `[tts].cache_entries`）
    /// 由注册表的 `validate_wake` 统一判定——它能看到整份配置。
    pub fn validate(&self) -> Result<()> {
        if self.enabled && self.greeting.trim().is_empty() {
            anyhow::bail!(
                "[wakeup].greeting 为空：启用唤醒问候语时请填一句问候语（如 \"我在\"），\
                 或把 [wakeup].enabled 改回 false（唤醒后静默等待，与没有这个插件一致）。"
            );
        }
        let n = self.greeting.trim().chars().count();
        if self.enabled && n > GREETING_SOFT_LIMIT {
            anyhow::bail!(
                "[wakeup].greeting 有 {n} 个字（建议 ≤ {GREETING_SOFT_LIMIT}）：问候语播放期间\
                 上行音频会在缓冲区排队，太长会推迟用户第一句话的处理。建议改成「我在」这类短句。"
            );
        }
        Ok(())
    }
}

/// 会话级唤醒问候快照（会话开始时构造一次 = `HotReload::NextSession` 语义）。
#[derive(Debug, Clone)]
pub struct WakeGreeting {
    active: bool,
    text: String,
    persist: bool,
}

impl WakeGreeting {
    /// 按配置快照构造。**不启用时 `active() == false`**，调用方无需自己判断开关。
    pub fn new(cfg: &WakeConfig) -> Self {
        let text = cfg.greeting.trim().to_string();
        WakeGreeting {
            active: cfg.enabled && !text.is_empty(),
            text,
            persist: cfg.persist,
        }
    }

    pub fn active(&self) -> bool {
        self.active
    }

    /// 要说的话（仅在 [`Self::active`] 为真时有意义）。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 是否允许把成品音频落盘复用。
    pub fn persist(&self) -> bool {
        self.persist
    }
}

#[cfg(test)]
mod tests;
