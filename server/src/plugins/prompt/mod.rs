//! 有序 prompt 段注册表：把"模型可见的系统提示词"从**一个字符串**变成**按 order 排序的段**。
//!
//! 借自 DSH `dsh-system-prompt`（见 `docs/plugin-architecture-unification.md` §4.4）的三条纪律：
//!
//! 1. **稀疏 order**：段号留空档（间隔 100~1000），以后往任意位置插一段不必重编号；
//! 2. **确定性的拼接顺序**：升序，同 order 按 `name` 的 code-unit 序兜底（避免顺序随机）；
//! 3. **宁可吵闹地失败，也不要畸形的 prompt**：未知/畸形变量在**启动期**就报错，
//!    而不是把 `{{name}}` 原样塞给模型；替换结果**不再二次扫描**；要保留字面花括号
//!    必须用 [`PromptSection::literal`]（记忆原文里会出现花括号，因此记忆段恒为 literal）。
//!
//! 段号表（本项目实际使用）：
//!
//! ```text
//! -1000  平台约束（能力/安全/格式；代码内置，随 [soul] 一起启用）
//!     0  人格主体（[soul] 前缀：身份/世界观/性格/价值）
//!   100  表达风格与边界（[soul] 后缀）
//!   200  [llm].system_prompt（附加约束，保持向后兼容）
//!   900  工具说明（预留）
//!  1000  历史（由 input[] 承载，此处仅占位说明顺序）
//!  1100  记忆召回（**易变**，默认插到历史之后 → 保住前面整段的前缀缓存）
//! ```
//!
//! ⚠️ 顺序不代表"全都在同一个字符串里"：[`TurnPrompt`] 里 `instructions` 承载**稳定前缀**
//! （平台 + 人格 + 附加约束），而记忆段按 [`MemoryPosition`] 决定插入 `input[]` 的位置。

use anyhow::{anyhow, Result};

/// 平台约束（代码内置，不可通过 UI 关闭）。
pub const ORDER_PLATFORM: i32 = -1000;
/// 人格主体。
pub const ORDER_SOUL_PREFIX: i32 = 0;
/// 表达风格与边界。
pub const ORDER_SOUL_SUFFIX: i32 = 100;
/// `[llm].system_prompt` 作为附加约束。
pub const ORDER_EXTRA: i32 = 200;
/// 工具说明（预留，MCP 闭环使用）。
#[allow(dead_code)]
pub const ORDER_TOOLS: i32 = 900;
/// 历史（仅用于说明顺序；实际由 `input[]` 承载）。
#[allow(dead_code)]
pub const ORDER_HISTORY: i32 = 1000;
/// 记忆召回。
pub const ORDER_MEMORY: i32 = 1100;

/// 已声明的运行时变量（`{{name}}`）：**唯一权威**，未在此列的引用一律报错。
///
/// 少即是好：语音端需要的是"人格自洽地称呼自己与用户"，其余动态事实（时间/天气）
/// 应当走记忆或工具，而不是每条 prompt 里塞一段易变文本（那会破坏前缀缓存）。
pub const KNOWN_VARS: &[&str] = &["name", "address_user", "device_id"];

/// 运行时变量表。
#[derive(Debug, Clone, Default)]
pub struct Vars {
    values: Vec<(&'static str, String)>,
}

impl Vars {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: &'static str, value: impl Into<String>) -> &mut Self {
        self.values.push((key, value.into()));
        self
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_str())
    }

    /// 供**启动期校验**用的样例值：所有已声明变量都有值 →
    /// 此时插值失败只可能是"未知变量名"，正是我们要在启动期抓的错误。
    pub fn sample() -> Self {
        let mut v = Self::new();
        for k in KNOWN_VARS {
            v.values.push((k, format!("<{k}>")));
        }
        v
    }
}

/// 一个 prompt 段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSection {
    /// 段号（升序拼接；同号按 `name` 兜底排序）。
    pub order: i32,
    /// 段名（同 order 的确定性 tie-break；也用于日志/测试定位）。
    pub name: &'static str,
    /// 段文本。
    pub text: String,
    /// 是否做 `{{var}}` 插值。`false` = 原文照发（记忆召回内容必须如此：
    /// 用户历史原话里可能出现花括号）。
    pub interpolate: bool,
}

impl PromptSection {
    /// 会做变量插值的段。
    pub fn new(order: i32, name: &'static str, text: impl Into<String>) -> Self {
        Self {
            order,
            name,
            text: text.into(),
            interpolate: true,
        }
    }

    /// **不做**变量插值的段（原文照发）。
    pub fn literal(order: i32, name: &'static str, text: impl Into<String>) -> Self {
        Self {
            order,
            name,
            text: text.into(),
            interpolate: false,
        }
    }
}

/// 单段插值：`{{var}}` → 变量值。
///
/// 规则（逐条对齐 DSH 的插值纪律）：
/// - 未知变量、已知但无值、畸形 `{{a}b}}`（名字里含花括号）、空名 → **报错**；
/// - 孤立的 `{{`（后面没有 `}}`）按普通文本处理；
/// - 替换结果**不再二次扫描**（变量值里出现 `{{x}}` 不会被继续展开）；
/// - 报错文案指向"改哪个字段怎么改"，而不是只报变量名。
pub fn interpolate(section: &str, text: &str, vars: &Vars) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            // 孤立 `{{`：当普通文本，且不再寻找后续（与 DSH 一致）
            out.push_str(&rest[start..]);
            return Ok(out);
        };
        let raw = &after[..end];
        if raw.contains('{') || raw.contains('}') {
            return Err(anyhow!(
                "prompt 段 `{section}` 里出现畸形变量 {{{{…}}}}（名字内含花括号）：{raw:?}。\
                 请改成 {{{{name}}}} 形式，或去掉花括号。"
            ));
        }
        let name = raw.trim();
        if name.is_empty() {
            return Err(anyhow!(
                "prompt 段 `{section}` 里有空变量 {{{{}}}}：请填入变量名（可用：{}）。",
                KNOWN_VARS.join(" / ")
            ));
        }
        match vars.get(name) {
            Some(v) => out.push_str(v),
            // 区分"名字根本不存在"与"已声明但本次没给值"：两者的修法完全不同
            None if KNOWN_VARS.contains(&name) => {
                return Err(anyhow!(
                    "prompt 段 `{section}` 引用了变量 {{{{{name}}}}}，但当前上下文没有它的值。\
                     该变量已声明，请检查取值处（会话是否带上了设备标识）后重试。"
                ))
            }
            None => {
                return Err(anyhow!(
                    "prompt 段 `{section}` 引用了未声明的变量 {{{{{name}}}}}：\
                     可用变量为 {}（新增变量需同时改 plugins/prompt/mod.rs 的 KNOWN_VARS 与取值处）。",
                    KNOWN_VARS.join(" / ")
                ))
            }
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// 启动期/保存期校验：段文本里的变量引用必须合法（**不留到运行期**）。
pub fn validate_sections(sections: &[PromptSection]) -> Result<()> {
    let v = Vars::sample();
    for s in sections {
        if s.interpolate {
            interpolate(s.name, &s.text, &v)?;
        }
    }
    Ok(())
}

/// 组装：升序（同 order 按 name）→ 丢空段 → `\n\n` 连接。
///
/// 空段丢弃是"配置项留空 = 不产生噪音"的关键：人格字段全空时不会拼出一串空行。
pub fn assemble(sections: &[PromptSection], vars: &Vars) -> Result<String> {
    let mut ordered: Vec<&PromptSection> = sections
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .collect();
    ordered.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.name.cmp(b.name)));
    let mut parts = Vec::with_capacity(ordered.len());
    for s in ordered {
        let text = if s.interpolate {
            interpolate(s.name, &s.text, vars)?
        } else {
            s.text.clone()
        };
        if text.trim().is_empty() {
            continue; // 插值后变空（如 `{{name}}` 值为空）也丢弃
        }
        parts.push(text.trim().to_string());
    }
    Ok(parts.join("\n\n"))
}

/// 记忆段相对历史的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryPosition {
    /// 历史**之后**、当前用户消息之前（默认）：保住 `instructions + history` 这段
    /// append-only 前缀的 prompt cache 命中。
    AfterHistory,
    /// 历史**之前**（紧跟 instructions）：召回内容离当前问题更近（recency 更强），
    /// 但每次召回变化都会让后面的历史前缀缓存失效。
    BeforeHistory,
}

impl MemoryPosition {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryPosition::AfterHistory => "after_history",
            MemoryPosition::BeforeHistory => "before_history",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "before_history" => MemoryPosition::BeforeHistory,
            _ => MemoryPosition::AfterHistory,
        }
    }
}

/// 本轮请求的 prompt 计划：稳定前缀（`instructions`）+ 易变记忆块（`memory`）。
#[derive(Debug, Clone, Default)]
pub struct TurnPrompt {
    /// 系统提示词：平台约束 + 人格 + 附加约束 + 工具说明（**稳定前缀**）。
    pub instructions: String,
    /// 记忆召回块（已渲染的 XML 文本）；`None` = 无命中或记忆关闭。
    pub memory: Option<String>,
    pub position: MemoryPosition,
}

impl Default for MemoryPosition {
    fn default() -> Self {
        MemoryPosition::AfterHistory
    }
}

#[cfg(test)]
mod tests;
