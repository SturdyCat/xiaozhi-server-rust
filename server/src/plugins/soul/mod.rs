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
//! ## 内置默认灵魂（`preset`）：开箱可用 + 可改可覆盖
//!
//! 21 个字段全留空时人格是空壳，因此 `[soul].preset` 默认指向内置预设
//! [`presets::DEFAULT_ID`]，它提供一份**默认人格**（小智，见 `presets.rs`）。
//! 合并规则（[`SoulConfig::effective`]）：**留空**的字段用预设补，**填了**的字段逐字段覆盖；
//! `preset = "none"` 完全不用预设。因此"用默认"和"改默认"是同一个机制，没有额外开关。
//!
//! 配套的可观测性（人格是"看不见的"，不给反馈就没人敢改）：
//! `GET /api/soul/presets` 给出预设清单与内容，`POST /api/soul/preview` 给出**最终提示词**
//! 以及"哪些字段来自预设、哪些被你覆盖"。
//!
//! ## 设计取舍（与 `docs/soul-and-graph-memory-plan.md` §4.2 的偏差）
//!
//! - **不做 `profile_path`（Markdown 档案文件）**：它要求每轮（或每会话）读盘并处理
//!   "文件改了但服务没刷新"的一致性问题，收益（版本化/diff）在语音端很低。留作后续阶段。
//! - 字段按"框架哲学"分组（身份/世界观/性格/价值/表达/范例），而不是按 UI 布局分组。
//! - `preset` **不是** P4 的 `engine`：`engine` 选实现（闭集），`preset` 是可覆盖的**内容基线**。
//!
//! 详见 `docs/soul-and-graph-memory-plan.md` §4。

use serde::{Deserialize, Serialize};

use crate::plugins::prompt::{
    self, PromptSection, ORDER_EXTRA, ORDER_PLATFORM, ORDER_SOUL_PREFIX, ORDER_SOUL_SUFFIX,
};

pub mod presets;

/// 预览（最终提示词 + 字段来源）：`preview::build`。
///
/// ⚠️ 不要在这里 `pub use` 重导出：本 crate 是二进制，**没有任何外部消费方**，
/// 未被使用的重导出会触发 `unused_imports`（实测）。模块公开即可。
pub mod preview;

/// 默认灵魂预设（`[soul]` 不写 `preset` 时用它）。
fn default_soul_preset() -> String {
    presets::DEFAULT_ID.into()
}

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
    /// 内置人格预设：提供**默认灵魂**的基线内容（`"none"` = 不用预设，完全自定义）。
    ///
    /// 留空的字段由预设补、填了的字段覆盖它（[`Self::effective`]）；
    /// 未知取值在启用时**报错**（不静默回退，与 `engine` 同一纪律）。
    #[serde(default = "default_soul_preset")]
    pub preset: String,
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
            preset: default_soul_preset(),
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

    /// 规范化后的预设 id（空串 = 用默认预设，避免"手写空串"变成静默不用预设）。
    pub fn preset_id(&self) -> &str {
        let id = self.preset.trim();
        if id.is_empty() {
            presets::DEFAULT_ID
        } else {
            id
        }
    }

    /// 规范化（P4 约定的配置入口调用）：`preset` 写入规范值。
    pub fn normalize(&mut self) {
        self.preset = self.preset_id().to_string();
    }

    /// **有效档案** = 内置预设基线 ⊕ 用户填写的字段（留空字段由预设补）。
    ///
    /// - `enabled == false`：直接返回自身（人格不参与，不解析 preset——老配置里写错的
    ///   preset 不该让服务起不来，保持"关掉就回到今天"）；
    /// - `preset = "none"`：不合并（完全自定义）；
    /// - 未知 preset：报错（**绝不静默回退**，否则用户以为换个预设生效了）；
    /// - 只合并文本/列表字段（[`presets::provided_fields`]）：布尔与数值恒有具体值，
    ///   不存在"留空"，一律以用户配置为准。
    pub fn effective(&self) -> anyhow::Result<SoulConfig> {
        if !self.enabled || presets::is_none(self.preset_id()) {
            return Ok(self.clone());
        }
        let id = self.preset_id();
        let Some(p) = presets::get(id) else {
            anyhow::bail!(
                "[soul].preset = \"{id}\" 不是内置灵魂预设：可选 {}。请改成可选值，\
                 或删掉该键使用默认（\"{}\"）。",
                presets::ids_hint(),
                presets::DEFAULT_ID
            );
        };
        let base = (p.profile)();
        let mut json = serde_json::to_value(self)?;
        let obj = json
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("[soul] 序列化结果不是对象"))?;
        for (k, base_v) in presets::provided_fields(&base) {
            let empty = match obj.get(&k) {
                Some(serde_json::Value::String(s)) => s.trim().is_empty(),
                Some(serde_json::Value::Array(a)) => a.is_empty(),
                None => true,
                Some(_) => false,
            };
            if empty {
                obj.insert(k, base_v);
            }
        }
        Ok(serde_json::from_value(json)?)
    }

    /// 硬校验：数值越界、启用时关键字段为空。文案必须含「怎么修」。
    ///
    /// 校验的是[`Self::effective`]后的档案：留空字段由预设补，因此**只在
    /// `preset = "none"`（或预设没提供该字段）时**才会因空值报错。
    pub fn validate(&self) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let eff = self.effective()?;
        if eff.name.trim().is_empty() {
            anyhow::bail!(
                "[soul].name 为空，且当前 preset（\"{}\"）没有提供名字：请填自称（例如 name = \"小智\"），\
                 否则 {{name}} 无法取值；也可以把 preset 改回 \"{}\" 用内置默认灵魂。",
                self.preset_id(),
                presets::DEFAULT_ID
            );
        }
        if eff.address_user.trim().is_empty() {
            anyhow::bail!(
                "[soul].address_user 为空，且当前 preset（\"{}\"）没有提供称呼：请填对用户的称呼\
                 （例如 \"你\"），否则 {{address_user}} 无法取值。",
                self.preset_id()
            );
        }
        if !(1..=8).contains(&eff.max_sentences) {
            anyhow::bail!(
                "[soul].max_sentences = {} 超出范围：语音回复请取 1~8（默认 2）。",
                eff.max_sentences
            );
        }
        if !(10..=500).contains(&eff.max_chars) {
            anyhow::bail!(
                "[soul].max_chars = {} 超出范围：语音回复请取 10~500（默认 40）。",
                eff.max_chars
            );
        }
        // 变量纪律：未知/畸形引用在**保存/启动期**就报出来，不留给运行期
        //（预设文本也在这里一起校验，拼错的变量在测试期就会被 presets/tests.rs 拦住）
        prompt::validate_sections(&eff.sections(""))?;
        Ok(())
    }

    /// 渲染为 prompt 段。
    ///
    /// ⚠️ `enabled == false` 时**只返回 `[llm].system_prompt` 一段**（原文、不插值），
    /// 保证与启用前的输出逐字节一致。
    ///
    /// ⚠️ 调用方应传 [`Self::effective`] 的结果（`instructions`/`preview` 已如此）：
    /// 直接传 `cfg.soul` 会丢掉内置预设的基线内容。
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
/// 人格内容取 [`SoulConfig::effective`]（预设基线 ⊕ 用户字段）。
pub fn instructions(cfg: &crate::config::Config, device_id: &str) -> anyhow::Result<String> {
    let soul = cfg.soul.effective()?;
    prompt::assemble(&soul.sections(&cfg.llm.system_prompt), &vars_for(&soul, device_id))
}

/// 插值变量（人格渲染与预览共用同一口径，避免"预览的和实际发的不一样"）。
fn vars_for(soul: &SoulConfig, device_id: &str) -> prompt::Vars {
    let mut vars = prompt::Vars::new();
    vars.set("name", soul.name.clone());
    vars.set("address_user", soul.address_user.clone());
    vars.set("device_id", device_id.to_string());
    vars
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
