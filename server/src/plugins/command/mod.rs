//! 语音指令闸门（`[command]`）：ASR 识别结果 → LLM **之前**的一道拦截。
//!
//! ## 为什么放在这里
//!
//! 「退下」「闭嘴」这类话不是提问，送进 LLM 只会得到一段没用的回复（还要计费、还要合成），
//! 正确的处置是**结束这次会话**。它必须发生在 ASR 之后、LLM 之前——本模块只做匹配判断
//! （纯函数、零依赖、可单测），"说告别语 → 断开会话"的副作用由会话层执行
//! （[`crate::app::session::run_session`] 的指令闸门分支）。
//!
//! ## 匹配口径（安全优先）
//!
//! 识别文本先经 [`normalize_utterance`] 归一化：**只保留字母数字/汉字**（丢掉空白、标点、
//! 符号、emoji），再剥掉句末语气助词（吧/啊/呀/呢/了…）。于是：
//!
//! - `"退下吧。"` → `"退下"` ✅ 命中（语音识别几乎总会带标点/语气词）；
//! - `"关闭闹钟"` → `"关闭闹钟"` ❌ **不**命中 `"关闭"`（默认 `exact` 模式）。
//!
//! 默认 `exact`（整句相等）是刻意的：`contains` 会让「关闭闹钟 / 把灯关闭」这类正常请求
//! 直接断线——那是比"没识别到指令"严重得多的故障。需要宽松匹配的用户可显式改
//! `match_mode = "contains"`。
//!
//! ## 长度闸门（`max_chars`，默认 5）
//!
//! 即使匹配方式放宽，也**只有短话**才做指令判断：归一化后超过 [`DEFAULT_MAX_CHARS`]（默认 5）
//! 个字符的识别文本一律直接放行给 LLM。
//!
//! 理由是"长句子携带信息"：「帮我关闭卧室的灯」「退下之后帮我放首歌」都含指令词，但它们
//! 是正常请求/追问，断线是灾难；真正的指令（退下/闭嘴/关闭）本身只有两三个字。长度闸门与
//! `exact` 是两层独立保险：前者挡"长句误伤"，后者挡"短句误伤"（关闭闹钟）。
//!
//! 计数口径与匹配**完全一致**——归一化之后（标点、空白、句末语气词都不计入），所以
//! 「退下！！！！」仍是 2 个字、照常命中；`max_chars = 0` = 不限制（等于拆掉这道保险）。
//!
//! ## 生效时机
//!
//! `[command]` 是**新会话生效**（`HotReload::NextSession`）：会话开始时按注册表签名取一次
//! 配置快照构造 [`CommandGate`]，会话中途改配置不影响正在进行的会话
//! （与 `[soul]` / `[memory]` / `[vad]` 一致）。

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// 指令匹配方式（`[command].match_mode`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MatchMode {
    /// 整句等于指令词（归一化后比较）。默认，最安全。
    #[default]
    Exact,
    /// 句中**出现**指令词即命中。
    /// ⚠️ 会把「关闭闹钟」「把灯关闭」也当成关机指令，仅在确认语音场景不会说这类话时使用。
    Contains,
}

impl MatchMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            MatchMode::Exact => "exact",
            MatchMode::Contains => "contains",
        }
    }
}

/// 只对**归一化后**不超过这么多字符的识别文本做指令判断的默认上限。
///
/// 5 个字足够覆盖「退下 / 闭嘴 / 关闭 / 退下吧」这类纯指令，又短到「帮我关闭卧室的灯」这种
/// 携带信息的句子不会被误判成指令。
pub const DEFAULT_MAX_CHARS: usize = 5;

/// `[command]` 段：语音指令闸门的配置。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommandConfig {
    /// 总开关。默认 **false**：不启用时行为与加本插件之前完全一致（零开销、零行为变化）。
    #[serde(default)]
    pub enabled: bool,
    /// 指令词表（归一化后比较，重复项在 [`Self::normalize`] 里去掉）。
    #[serde(default = "default_keywords")]
    pub keywords: Vec<String>,
    /// 匹配方式，默认 `exact`（见 [`MatchMode`] 的安全说明）。
    #[serde(default)]
    pub match_mode: MatchMode,
    /// 长度闸门：归一化后超过这么多字符的识别文本**不做指令判断**，直接放行给 LLM。
    ///
    /// 为什么要有它：长句子携带信息（「帮我关闭卧室的灯」含「关闭」），断线是灾难；
    /// 真正的指令只有两三个字。`0` = 不限制（拆掉这道保险，不建议）。
    /// 计数口径与匹配一致——归一化之后，标点/空白/句末语气词不计入。
    #[serde(default = "default_max_chars")]
    pub max_chars: usize,
    /// 命中后先说这句告别语再断开；空串 = 直接断开（不出声）。
    #[serde(default = "default_reply")]
    pub reply: String,
}

impl Default for CommandConfig {
    fn default() -> Self {
        CommandConfig {
            enabled: false,
            keywords: default_keywords(),
            match_mode: MatchMode::Exact,
            max_chars: DEFAULT_MAX_CHARS,
            reply: default_reply(),
        }
    }
}

impl CommandConfig {
    /// 规范化：指令词与告别语都按归一化口径清洗（幂等，只在配置入口调用）。
    ///
    /// 幂等很重要——`normalize()` 会在 `Config::load` / `default` / `merge_client_patch`
    /// 三条路径上被调用，且保存时会落盘。
    pub fn normalize(&mut self) {
        self.keywords = clean_keywords(&self.keywords);
        self.reply = self.reply.trim().to_string();
    }

    /// 闸门是否真的在拦：启用 **且** 指令词表非空。
    ///
    /// 词表为空时（用户清空了）不拦任何话——空词表若按"命中一切"处理会立刻断掉所有会话，
    /// 这是最危险的失败模式。配置问题另由 [`Self::validate`] 上报为 `Degraded`。
    pub fn active(&self) -> bool {
        self.enabled && !self.keywords.is_empty()
    }

    /// 热切换签名（仅内存比较）。本能力无共享实例，签名只用于 `/api/plugins` 与状态上报。
    /// `max_chars` 会改变拦截行为，因此必须在签名里（否则改了它不会被视为配置变化）。
    pub fn signature(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.enabled,
            self.match_mode.as_str(),
            self.max_chars,
            self.reply,
            self.keywords.join(",")
        )
    }

    /// 前置校验：错误文案必须含「怎么修」（对齐注册表 `validate` 约定）。
    pub fn validate(&self) -> Result<()> {
        if self.enabled && self.keywords.is_empty() {
            anyhow::bail!(
                "[command].keywords 为空：启用指令闸门时请至少填一个指令词（如 [\"退下\"]），\
                 或把 [command].enabled 改回 false。"
            );
        }
        // 长度闸门与词表的交互坑：词比 max_chars 还长 → 永远命中不了（静默失效，本仓库最忌讳）。
        if self.enabled && self.max_chars > 0 {
            for kw in &self.keywords {
                let n = normalize_utterance(kw).chars().count();
                if n > self.max_chars {
                    anyhow::bail!(
                        "[command].keywords 里的「{kw}」归一化后有 {n} 个字，超过 [command].max_chars = {}，\
                         永远不会命中；请把 max_chars 调到 ≥ {n}（或设为 0 = 不限制），或换一个更短的指令词。",
                        self.max_chars
                    );
                }
            }
        }
        Ok(())
    }
}

/// 一次命中的结果（会话层据此说告别语并断开）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandHit {
    /// 命中的指令词（用于日志，回答"到底哪句话触发的"）。
    pub keyword: String,
    /// 命中后要说的话（可能为空 = 直接断开）。
    pub reply: String,
}

/// 会话级指令闸门：构造时取配置快照，之后是只读的纯匹配。
#[derive(Debug, Clone)]
pub struct CommandGate {
    active: bool,
    mode: MatchMode,
    /// 长度上限，`0` = 不限制（见 [`CommandConfig::max_chars`]）。
    max_chars: usize,
    keywords: Vec<String>,
    reply: String,
}

impl CommandGate {
    /// 按配置快照构造（会话开始时调用一次 = `next_session` 语义）。
    ///
    /// 这里**再归一化一次**词表（而非假定配置已由 [`CommandConfig::normalize`] 清洗过）：
    /// 归一化是幂等的、每会话只做一次，代价可忽略；换来的是"任何配置入口漏调 normalize
    /// 也不会表现为『词配了却不生效』"——那正是本仓库最忌讳的静默失败。
    pub fn new(cfg: &CommandConfig) -> Self {
        let keywords = clean_keywords(&cfg.keywords);
        CommandGate {
            // 配置层判定（enabled 且原词表非空）**再**与清洗后的词表求交：
            // 词表里全是标点这类"归一化后为空"的项时，闸门也不该拦任何话。
            active: cfg.active() && !keywords.is_empty(),
            mode: cfg.match_mode,
            max_chars: cfg.max_chars,
            keywords,
            reply: cfg.reply.trim().to_string(),
        }
    }

    /// 识别文本是否命中指令；返回命中详情供调用方说告别语/记日志。
    /// 未启用（或词表为空）时恒返回 `None`（调用方无需自己判断开关）。
    pub fn hit(&self, text: &str) -> Option<CommandHit> {
        if !self.active {
            return None;
        }
        let utterance = normalize_utterance(text);
        if utterance.is_empty() {
            return None;
        }
        // 长度闸门：长句是"携带信息的话"，不按指令处理（「帮我关闭卧室的灯」含指令词但不是指令）。
        // 计数在归一化**之后**（标点/空白/句末语气词不计入），与匹配口径一致；
        // `max_chars = 0` = 不限制。
        if self.max_chars > 0 && utterance.chars().count() > self.max_chars {
            return None;
        }
        self.keywords
            .iter()
            .find(|kw| match self.mode {
                MatchMode::Exact => utterance == **kw,
                MatchMode::Contains => utterance.contains(kw.as_str()),
            })
            .map(|kw| CommandHit {
                keyword: kw.clone(),
                reply: self.reply.clone(),
            })
    }
}

/// 句末语气助词：只说「退下」的人往往被识别成「退下吧」「闭嘴啊」，这些应等价处理。
const TRAILING_PARTICLES: &str = "吧啊呀呢哦嘛哈呐哩了呗啦哇哟噢喔唉诶嗯";

/// 识别文本归一化：只保留字母/数字/汉字（丢掉空白、标点、符号、emoji），再剥掉句末语气助词。
///
/// 指令词表走同一函数，因此「命中」= 双方用同一口径比较，不会出现"配置里写对了却不生效"。
pub fn normalize_utterance(raw: &str) -> String {
    let kept: String = raw.chars().filter(|c| c.is_alphanumeric()).collect();
    kept.trim_end_matches(|c| TRAILING_PARTICLES.contains(c))
        .to_string()
}

/// 词表清洗：逐项归一化 → 丢弃空项 → 去重。`normalize()` 与 [`CommandGate::new`] 共用。
fn clean_keywords(raw: &[String]) -> Vec<String> {
    let mut kept: Vec<String> = Vec::with_capacity(raw.len());
    for item in raw {
        let kw = normalize_utterance(item);
        if !kw.is_empty() && !kept.contains(&kw) {
            kept.push(kw);
        }
    }
    kept
}

fn default_keywords() -> Vec<String> {
    // 三个内置指令：退下 / 闭嘴 / 关闭（"关闭"在默认 exact 模式下不会误伤「关闭闹钟」）
    vec!["退下".to_string(), "闭嘴".to_string(), "关闭".to_string()]
}

fn default_max_chars() -> usize {
    DEFAULT_MAX_CHARS
}

fn default_reply() -> String {
    "好的，我先退下了。".to_string()
}

#[cfg(test)]
mod tests;
