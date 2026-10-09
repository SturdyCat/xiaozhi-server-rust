//! 「灵魂」（人格档案）：把 `[soul]` 的结构化字段渲染成**有序 prompt 段**。
//!
//! ## 为什么单独建段（而不是往 `[llm].system_prompt` 里塞）
//!
//! 一个字符串能写人格，但没法结构化编辑、多套切换、与记忆分层组装、按设备绑定。
//! 更关键的是**可回滚**：`[soul].enabled = false` 时本模块**不产出任何段**，
//! 调用方直接用 `[llm].system_prompt` 原文 —— 输出与启用前**逐字节一致**（有回归单测保证）。
//!
//! ## 语音场景的第一性原则：短
//!
//! 默认 `max_sentences = 2` / `max_chars = 40`，并把它们作为**硬约束写进平台段**
//! （而不是只放在配置里当摆设）。TTS 会把 emoji 念出来，因此 `emoji` 默认关闭。
//!
//! ## 设计取舍（与 `docs/soul-and-graph-memory-plan.md` §4.2 的偏差）
//!
//! - **不做 `profile_path`（Markdown 档案文件）**：它要求每轮（或每会话）读盘并处理
//!   "文件改了但服务没刷新"的一致性问题，收益（版本化/diff）在语音端很低。留作后续阶段。
//! - 字段按"框架哲学"分组（身份/世界观/性格/价值/表达/范例），而不是按 UI 布局分组。
//!
//! 详见 `docs/soul-and-graph-memory-plan.md` §4。

use serde::{Deserialize, Serialize};

use crate::plugins::prompt::{
    self, PromptSection, ORDER_EXTRA, ORDER_PLATFORM, ORDER_SOUL_PREFIX, ORDER_SOUL_SUFFIX,
};

fn default_soul_name() -> String {
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
fn default_true() -> bool {
    true
}

/// 人格档案配置（`[soul]`）。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SoulConfig {
    /// 启用后 LLM 的 `instructions` 由本模块按段组装；关闭时完全退回
    /// `[llm].system_prompt`（可无痛回滚）。
    #[serde(default)]
    pub enabled: bool,
    /// 自称。
    #[serde(default = "default_soul_name")]
    pub name: String,
    /// 自我定位（一段话：你是谁、在做什么）。
    #[serde(default)]
    pub self_intro: String,
    /// 形象/物种（如"一只住在音箱里的小狐狸"）。
    #[serde(default)]
    pub form: String,
    /// 给人的年龄感。
    #[serde(default)]
    pub age_feel: String,
    /// 世界观（世界是什么样的）。
    #[serde(default)]
    pub worldview: String,
    /// 来历/背景故事。
    #[serde(default)]
    pub backstory: String,
    /// 你与用户的关系是怎么开始的（比"关系"更能锁定语气）。
    #[serde(default)]
    pub relationship_origin: String,
    /// 性格：每条写「标签：遇到 X 会怎么做」。只写形容词会失效。
    #[serde(default)]
    pub traits: Vec<String>,
    /// 坚持的价值。
    #[serde(default)]
    pub values: Vec<String>,
    /// 拒绝清单/红线。
    #[serde(default)]
    pub boundaries: Vec<String>,
    /// 语气（如"轻快、温和、带一点好奇"）。
    #[serde(default)]
    pub tone: String,
    /// 口语化（允许省略主语/短句）。
    #[serde(default = "default_true")]
    pub colloquial: bool,
    /// 称呼用户。
    #[serde(default = "default_address_user")]
    pub address_user: String,
    /// 口头禅（自然使用，不必每句都用）。
    #[serde(default)]
    pub catchphrases: Vec<String>,
    /// 是否允许 emoji。**默认 false**：TTS 会把 emoji 念出来，属事故。
    #[serde(default)]
    pub emoji: bool,
    /// 回复句数上限（写进平台段的硬约束）。
    #[serde(default = "default_max_sentences")]
    pub max_sentences: u32,
    /// 回复字数上限（同上）。
    #[serde(default = "default_max_chars")]
    pub max_chars: u32,
    /// 当前情境/场景（如"书房里，晚上"）。
    #[serde(default)]
    pub scenario: String,
    /// 设备相关提示（如"这是放在客厅的音箱"）。
    #[serde(default)]
    pub device_hint: String,
    /// 风格范例：每条一组 Q/A（如 `用户：今天几度？\n你：我这边看不到天气，你手机上看一眼？`）。
    /// 锁风格**最有效**的手段，优先于长篇描述。
    #[serde(default)]
    pub examples: Vec<String>,
}

impl Default for SoulConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            name: default_soul_name(),
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
    /// 签名（热刷新判定用；纯字段，不含运行期值）。
    pub fn signature(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// 硬校验：数值越界、启用时关键字段为空。文案必须含「怎么修」。
    pub fn validate(&self) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if self.name.trim().is_empty() {
            anyhow::bail!("[soul].name 为空：人格需要一个自称（例如 name = \"小智\"），否则 {{name}} 无法取值。");
        }
        if self.address_user.trim().is_empty() {
            anyhow::bail!("[soul].address_user 为空：请填对用户的称呼（例如 \"你\"），否则 {{address_user}} 无法取值。");
        }
        if !(1..=8).contains(&self.max_sentences) {
            anyhow::bail!(
                "[soul].max_sentences = {} 超出范围：语音回复请取 1~8（默认 2）。",
                self.max_sentences
            );
        }
        if !(10..=500).contains(&self.max_chars) {
            anyhow::bail!(
                "[soul].max_chars = {} 超出范围：语音回复请取 10~500（默认 40）。",
                self.max_chars
            );
        }
        // 变量纪律：未知/畸形引用在**保存/启动期**就报出来，不留给运行期
        prompt::validate_sections(&self.sections(""))?;
        Ok(())
    }

    /// 渲染为 prompt 段。
    ///
    /// ⚠️ `enabled == false` 时**只返回 `[llm].system_prompt` 一段**（原文、不插值），
    /// 保证与启用前的输出逐字节一致。
    pub fn sections(&self, system_prompt: &str) -> Vec<PromptSection> {
        if !self.enabled {
            return vec![PromptSection::literal(
                ORDER_EXTRA,
                "llm_system_prompt",
                system_prompt,
            )];
        }
        let mut out = vec![
            PromptSection::literal(
                ORDER_PLATFORM,
                "platform_constraints",
                platform_constraints(self.max_sentences, self.max_chars),
            ),
            PromptSection::new(ORDER_SOUL_PREFIX, "soul_prefix", self.prefix_text()),
            PromptSection::new(ORDER_SOUL_SUFFIX, "soul_suffix", self.suffix_text()),
        ];
        if !system_prompt.trim().is_empty() {
            // 附加约束恒为原文：旧配置里的 `[llm].system_prompt` 可能含花括号
            out.push(PromptSection::literal(
                ORDER_EXTRA,
                "llm_system_prompt",
                system_prompt,
            ));
        }
        out
    }

    /// 人格主体（身份 → 关系 → 来历 → 性格 → 价值）。
    fn prefix_text(&self) -> String {
        let mut s = String::new();
        s.push_str("# 你是谁\n");
        s.push_str(&format!("你的名字是「{}」。", self.name.trim()));
        push_line(&mut s, &self.self_intro);
        let mut attrs = Vec::new();
        if !self.form.trim().is_empty() {
            attrs.push(format!("形象：{}", self.form.trim()));
        }
        if !self.age_feel.trim().is_empty() {
            attrs.push(format!("给人的年龄感：{}", self.age_feel.trim()));
        }
        if !attrs.is_empty() {
            s.push_str(&format!("（{}）", attrs.join("；")));
        }
        s.push('\n');
        push_block(&mut s, "## 你与用户的关系", &self.relationship_origin);
        let mut origin = String::new();
        push_line(&mut origin, &self.worldview);
        push_line(&mut origin, &self.backstory);
        push_block(&mut s, "## 你的来历", &origin);
        push_list(
            &mut s,
            "## 你的性格（照下面的做法来，不要只当形容词）",
            &self.traits,
        );
        push_list(&mut s, "## 你坚持的价值", &self.values);
        s.trim_end().to_string()
    }

    /// 表达风格与边界。
    fn suffix_text(&self) -> String {
        let mut s = String::from("## 怎么说话\n");
        push_line(
            &mut s,
            &format!("- 称呼用户为「{}」。", self.address_user.trim()),
        );
        if !self.tone.trim().is_empty() {
            push_line(&mut s, &format!("- 语气：{}。", self.tone.trim()));
        }
        if self.colloquial {
            s.push_str("- 说人话：短句、可以省略主语，不要书面语和客套话。\n");
        }
        if !self.catchphrases.is_empty() {
            s.push_str(&format!(
                "- 口头禅（自然地用，不要每句都用）：{}。\n",
                self.catchphrases.join("；")
            ));
        }
        if self.emoji {
            s.push_str("- 可以用 emoji。\n");
        } else {
            s.push_str("- 不要用 emoji 或颜文字（会被直接念出来）。\n");
        }
        push_list(&mut s, "## 你不会做的事", &self.boundaries);
        if !self.examples.is_empty() {
            s.push_str("## 风格范例（严格模仿这个说话方式）\n");
            for e in &self.examples {
                let e = e.trim();
                if !e.is_empty() {
                    s.push_str(&format!("{e}\n"));
                }
            }
        }
        let mut ctx = String::new();
        push_line(
            &mut ctx,
            if self.scenario.trim().is_empty() {
                ""
            } else {
                &self.scenario
            },
        );
        push_line(
            &mut ctx,
            if self.device_hint.trim().is_empty() {
                ""
            } else {
                &self.device_hint
            },
        );
        push_block(&mut s, "## 当前情境", &ctx);
        s.trim_end().to_string()
    }
}

/// 平台约束段（代码内置，不可通过 UI 关闭）。
fn platform_constraints(max_sentences: u32, max_chars: u32) -> String {
    format!(
        "【语音对话的基本约束】\n\
         你通过语音与用户对话，你的回复会被直接合成为语音朗读：\n\
         - 只输出要说的内容：不要 Markdown（星号/井号/列表/表格）、不要括号里的动作或情绪说明、不要 emoji。\n\
         - 说短一点：最多 {max_sentences} 句、不超过 {max_chars} 个字；先回答，再补必要的半句。\n\
         - 不要复述用户的问题；不要自我介绍（除非用户问你是谁）。\n\
         - 不确定就直说不确定，不要编造事实、人名、数字或引用不存在的信息。\n\
         - 默认用中文；用户用其他语言时跟随用户。\n\
         【安全边界】\n\
         - 不提供违法、危险或伤害他人的具体做法；简短拒绝，并把话题引回能帮忙的方向。\n\
         - 涉及医疗/法律/金融的关键决定，提醒用户找专业人士，不要下结论。"
    )
}

/// 组装 `instructions`（系统提示词）。
///
/// 关闭人格时输出 = `[llm].system_prompt` **原文**（逐字节一致，见单测）；
/// 启用时 = 平台约束 + 人格主体 + 表达风格 + 附加约束，按 order 拼接。
pub fn instructions(cfg: &crate::config::Config, device_id: &str) -> anyhow::Result<String> {
    let mut vars = prompt::Vars::new();
    vars.set("name", cfg.soul.name.clone());
    vars.set("address_user", cfg.soul.address_user.clone());
    vars.set("device_id", device_id.to_string());
    prompt::assemble(&cfg.soul.sections(&cfg.llm.system_prompt), &vars)
}

/// 追加一行（空串跳过）。
fn push_line(s: &mut String, line: &str) {
    let line = line.trim();
    if !line.is_empty() {
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        s.push_str(line);
        s.push('\n');
    }
}

/// 追加一个带标题的块；正文为空则整块跳过。
fn push_block(s: &mut String, title: &str, body: &str) {
    if body.trim().is_empty() {
        return;
    }
    if !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    }
    s.push_str(title);
    s.push('\n');
    s.push_str(body.trim());
    s.push('\n');
}

/// 追加一个列表块（过滤空项）。
fn push_list(s: &mut String, title: &str, items: &[String]) {
    let items: Vec<&str> = items
        .iter()
        .map(|i| i.trim())
        .filter(|i| !i.is_empty())
        .collect();
    if items.is_empty() {
        return;
    }
    if !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    }
    s.push_str(title);
    s.push('\n');
    for i in items {
        s.push_str(&format!("- {i}\n"));
    }
}

#[cfg(test)]
mod tests;
