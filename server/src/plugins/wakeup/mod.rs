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
//! （[`crate::workflow::session::reply::speak_canned`] 的同一套下行链路），一次 LLM 都不调。
//!
//! ## 两级音频复用（这是"高频也不贵"的关键）
//!
//! | 层级 | 位置 | 命中时 |
//! |---|---|---|
//! | ① 内存 | [`crate::workflow::pipeline::tts_cache`]（跨会话，进程内） | 零合成，直接投递已编码帧 |
//! | ② 磁盘 | [`crate::media::hot_audio`]（`[wakeup].persist`） | 读盘 ~2KB → 暖进 ① → 零合成 |
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
//! 同语义）。默认 `enabled = true`——问候语是「开机即用」的标准体验；不需要的用户可
//! 显式关闭，关闭时零开销、行为与加它之前完全一致。

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
    /// 总开关。默认 **true**：问候语是「开机即用」的标准体验（唤醒后立刻回一句「我在」，
    /// 不再静默等待）；不需要的用户可显式改为 false——关闭时零开销、行为与加本插件之前完全一致。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 唤醒后立刻说的话（跳过 LLM 与正常回复）。支持**多句**：换行分隔，
    /// 每次唤醒随机选一句说（「我在 / 在呢 / 你好呀」轮着来，不像录音）；单行 = 单句，
    /// 与旧行为完全一致。留空（或全是空白行）= 不问候。
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

fn default_true() -> bool {
    true
}

fn default_persist() -> bool {
    true
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

impl WakeConfig {
    /// 规范化（幂等；`Config::load` / `default` / `merge_client_patch` 三条路径都会调）。
    pub fn normalize(&mut self) {
        self.greeting = normalize_pool(&self.greeting);
    }

    /// 问候语池：按换行拆分、逐行去首尾空白、丢空行。单行 = 单句（与旧行为一致）。
    pub fn pool(&self) -> Vec<String> {
        split_pool(&self.greeting)
    }

    /// 是否真的会问候：启用 **且** 池里至少有一句。
    ///
    /// 空池不问候（而不是"问候一个空字符串"）——留空是"我要静默"的合法表达，
    /// 配置矛盾另由 [`Self::validate`] 上报为 `Degraded`。
    ///
    /// 判定用逐行 `trim()`：配置可能来自任意入口（`merge_client_patch` / 手改文件），
    /// 不能假定 [`Self::normalize`] 一定先跑过；纯空白必须与空串同义，
    /// 否则"填了几个空格"会变成"问候一个空字符串"这种静默怪象。
    pub fn active(&self) -> bool {
        self.enabled && !self.pool().is_empty()
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
        let lines = self.pool();
        if self.enabled && lines.is_empty() {
            anyhow::bail!(
                "[wakeup].greeting 为空：启用唤醒问候语时请填至少一句问候语（如 \"我在\"），\
                 或把 [wakeup].enabled 改回 false（唤醒后静默等待，与没有这个插件一致）。"
            );
        }
        // 软上限**逐行**检查：多句时任何一句过长都会拖住同一处下行排队。
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

/// 拆问候语池：换行分隔、逐行 trim、丢空行。
fn split_pool(s: &str) -> Vec<String> {
    s.split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// 收敛为规范存储形态：池子逐行 join（幂等的唯一形态，配置对比/签名都稳定）。
fn normalize_pool(s: &str) -> String {
    split_pool(s).join("\n")
}

/// 会话级唤醒问候快照（会话开始时构造一次 = `HotReload::NextSession` 语义）。
#[derive(Debug, Clone)]
pub struct WakeGreeting {
    active: bool,
    lines: Vec<String>,
    persist: bool,
}

impl WakeGreeting {
    /// 按配置快照构造。**不启用时 `active() == false`**，调用方无需自己判断开关。
    pub fn new(cfg: &WakeConfig) -> Self {
        WakeGreeting {
            active: cfg.active(),
            lines: cfg.pool(),
            persist: cfg.persist,
        }
    }

    pub fn active(&self) -> bool {
        self.active
    }

    /// 本轮要说的一句：多句池中**随机**选一（每次唤醒各选各的，不固定顺序）；
    /// 单句池恒返回它。仅在 [`Self::active`] 为真时有意义，空池返回空串（防御）。
    pub fn text(&self) -> &str {
        let Some(first) = self.lines.first() else {
            return "";
        };
        if self.lines.len() == 1 {
            return first;
        }
        // 进程内伪随机：`RandomState` 每次新实例都换种子（OS 熵 + 计数器），
        // 对"多句问候换着说"足够，刻意不为此引入 rand 依赖。
        use std::hash::{BuildHasher, Hasher};
        let idx = (std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish()
            % self.lines.len() as u64) as usize;
        &self.lines[idx]
    }

    /// 是否允许把成品音频落盘复用。
    pub fn persist(&self) -> bool {
        self.persist
    }
}

#[cfg(test)]
mod tests;
