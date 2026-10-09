//! 插件注册表：能力与实现的**可枚举**描述（`docs/plugin-architecture-unification.md` §3）。
//!
//! ## 为什么需要它
//!
//! `plugins/mod.rs` 承诺过「加实现文件 + 工厂一行注册即可」，但实测新增一个 TTS 供应商要改
//! **19 处**（枚举变体 / 签名 / 工厂 / 客户端 observable / 注册表 / save / fill / 卡片……）。
//! 根因不是代码写错了，而是**「能力」只存在于代码结构里，没有任何地方能枚举它**：
//! 没有清单、没有字段元数据、没有状态上报，于是每一处调用方都得自己"知道"有哪些实现。
//!
//! 本文件把这三件事变成 `'static` 常量表：
//!
//! 1. [`REGISTRY`] → `GET /api/plugins` 能列出能力 + 实现 + 状态（UI 不必再猜后端）；
//! 2. [`FieldSchema`] → `GET /api/config/schema` 返回字段元数据（校验/默认值/热生效语义）；
//! 3. **新增实现 = 表里加 1 条**，枚举/校验/上报自动获得。
//!
//! ## 边界（刻意的）
//!
//! - 本文件只有**描述与构建入口**，不含业务逻辑；各实现仍在各自 `plugins/<cap>/` 下，
//!   五个引擎 trait（`AsrEngine`/`TtsEngine`/`LlmProvider`/`VadEngine`/`FullChainEngine`）
//!   **一行未改**——这是本设计能低风险落地的根本原因。
//! - [`Built`] 是**闭枚举**而不是 `Box<dyn Any>`：类型擦除后向下转型会把编译期错误变成
//!   运行时 panic；枚举 + 按能力 `match` 保持 100% 编译期检查。
//! - `signature` 只用于**内存内比较**（热切换判定），**绝不打日志**（含密钥字段）。

//! ## 文件布局（AGENTS.md §5.10 文件规模约定）
//!
//! - `mod.rs`（本文件）：类型定义 + 全量注册表 + 查找 API + JSON 投影；
//! - `fields.rs`：字段元数据表；
//! - `impls.rs`：各实现的选择/签名/校验/构建 + 描述符常量；
//! - `tests.rs`：回归测试。

use anyhow::Result;
use std::sync::Arc;

use crate::config::Config;
use crate::plugins::aiui::FullChainEngine;
use crate::plugins::asr::AsrEngine;
use crate::plugins::llm::Llm;
use crate::plugins::tts::TtsEngine;

mod fields;
mod fields_command;
mod fields_context;
mod impls;
use fields::CAPABILITY_COMMON_FIELDS;
use fields_context::CONTEXT_COMMON_FIELDS;
pub use impls::*;

/// 某能力的**全部公共字段**（`fields.rs` 的表 + `fields_context.rs` 的上下文生产者表）。
///
/// 同一个能力可以有多条记录（不同配置段，如 `[memory]` / `[memory.retention]`），
/// 因此这里返回**合并后的迭代器**，而不是只取第一条。
pub fn capability_common_fields(
    cap: Capability,
) -> impl Iterator<Item = (&'static [&'static str], &'static [FieldSchema])> {
    CAPABILITY_COMMON_FIELDS
        .iter()
        .chain(CONTEXT_COMMON_FIELDS.iter())
        .filter(move |(c, _, _)| *c == cap)
        .map(|(_, path, fields)| (*path, *fields))
}

// ============================================================
// 能力
// ============================================================

/// 一个**能力**（capability）：一类可替换的引擎/服务。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum Capability {
    Asr,
    Vad,
    Tts,
    Llm,
    /// AIUI 全链路（ASR+LLM+TTS 云端闭环）——每会话有状态长连接，不进共享引擎池。
    FullChain,
    /// 发音人目录探测（不是引擎，是 app 层服务；P4 纳入统一动作模型）。
    Voices,
    /// 固件托管（同上）。
    Firmware,
    /// 图记忆（规划中，见 `docs/soul-and-graph-memory-plan.md`）。
    Memory,
    /// 灵魂/人格档案（规划中）。
    Soul,
    /// 语音指令闸门（`[command]`）：ASR → LLM 之间拦截「退下/闭嘴/关闭」并断开会话。
    /// 不是引擎（不产出共享实例、无 `engine` 选择器），只按 `[command].enabled` 开关。
    Command,
}

/// 全部能力（顺序即 UI 展示顺序）。
pub const ALL_CAPABILITIES: &[Capability] = &[
    Capability::Asr,
    Capability::Vad,
    Capability::Tts,
    Capability::Llm,
    Capability::FullChain,
    Capability::Voices,
    Capability::Firmware,
    Capability::Memory,
    Capability::Soul,
    Capability::Command,
];

impl Capability {
    /// 稳定 id（API/JSON 用，勿改）。
    pub const fn id(self) -> &'static str {
        match self {
            Capability::Asr => "asr",
            Capability::Vad => "vad",
            Capability::Tts => "tts",
            Capability::Llm => "llm",
            Capability::FullChain => "fullchain",
            Capability::Voices => "voices",
            Capability::Firmware => "firmware",
            Capability::Memory => "memory",
            Capability::Soul => "soul",
            Capability::Command => "command",
        }
    }

    pub const fn display(self) -> &'static str {
        match self {
            Capability::Asr => "语音识别",
            Capability::Vad => "语音活动检测",
            Capability::Tts => "语音合成",
            Capability::Llm => "大语言模型",
            Capability::FullChain => "AIUI 全链路",
            Capability::Voices => "发音人目录",
            Capability::Firmware => "固件托管",
            Capability::Memory => "记忆（图记忆）",
            Capability::Soul => "灵魂（人格档案）",
            Capability::Command => "指令闸门（ASR→LLM）",
        }
    }

    /// **必需能力**：缺失即启动失败（对齐现状 `Engines::new` 失败即退出）。
    /// 其余为可选能力：坏掉只应降级（`Phase::Degraded`），不能拖垮语音服务。
    pub const fn required(self) -> bool {
        matches!(
            self,
            Capability::Asr | Capability::Vad | Capability::Tts | Capability::Llm
        )
    }

    /// 是否已有实现（注册表覆盖度测试据此断言"声明了就必须有实现"）。
    ///
    /// P6 起 `Memory`/`Soul` 均已落地，因此恒为 `true`；保留该 API 是因为
    /// 新增能力时"先声明、后实现"仍是合法的工作顺序。
    pub const fn implemented(self) -> bool {
        true
    }

    /// 校验失败是否**致命**（启动即失败）。
    ///
    /// - 必需能力恒致命；
    /// - [`Capability::FullChain`] 虽不是"必备能力"（可关掉走本地流水线），但**一旦启用**
    ///   就接管整条链路：配置不全时每个会话都会失败，此时快速失败远好过"能启动但没法用"
    ///   ——保持 `Engines::new` 既有行为（`aiui.enabled` 三要素缺失即退出）。
    /// - 其余可选能力（`Voices`/`Firmware`/`Memory`/`Soul`）坏掉只降级，不拖垮语音服务。
    pub const fn fatal_if_enabled(self) -> bool {
        self.required() || matches!(self, Capability::FullChain)
    }
}

// ============================================================
// 热生效语义 / 依赖 / 字段元数据
// ============================================================

/// 热生效语义（= DSH 的 `.volatile()`，但区分三种粒度）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HotReload {
    /// 保存即对**下一次请求**生效。
    Live,
    /// 保存后对**新会话**生效（当前会话保持旧值）。
    NextSession,
    /// 必须重启进程。
    RestartOnly,
}

impl HotReload {
    pub const fn as_str(self) -> &'static str {
        match self {
            HotReload::Live => "live",
            HotReload::NextSession => "next_session",
            HotReload::RestartOnly => "restart_only",
        }
    }
    /// 给 UI 的一句话文案（管理页状态/字段提示直接用）。
    pub const fn hint(self) -> &'static str {
        match self {
            HotReload::Live => "保存即生效",
            HotReload::NextSession => "保存后新会话生效",
            HotReload::RestartOnly => "需重启 server 生效",
        }
    }
    /// 严格程度序（Live < NextSession < RestartOnly）：一次保存改了多个字段时，
    /// 对外承诺必须取**最严格**的那个，否则会出现"说立即生效、其实要重启"的误导。
    pub const fn rank(self) -> u8 {
        match self {
            HotReload::Live => 0,
            HotReload::NextSession => 1,
            HotReload::RestartOnly => 2,
        }
    }
    /// 两者取更严格的一个（`strictest(Live, x) == x`）。
    pub const fn strictest(self, other: Self) -> Self {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }
}

/// **非插件**配置段（应用级，不在注册表里）的热生效语义。
///
/// `[server]`（端口 / worker 线程 / 静态目录 / token）与 `[audio]`（下行采样率、帧长、
/// 二进制协议版本、抖动提前量）都是**进程级**参数：握手与 OTA 必须与启动值一致，
/// 因此只能重启生效（对齐 `docs/plugin-architecture-unification.md` §0.5 偏差 6）。
const APP_SECTION_HOT: &[(&str, HotReload)] = &[
    ("server", HotReload::RestartOnly),
    ("audio", HotReload::RestartOnly),
];

/// 按**字段完整路径**查热生效语义：路径 = `fields_path ++ [key]`
///（如 `["tts", "kokoro", "model"]`、`["memory", "retention", "keep"]`）。
/// 未登记返回 `None`（调用方逐级回退到 [`section_hot`]）。
pub fn field_hot(path: &[&str]) -> Option<HotReload> {
    let (key, parents) = path.split_last()?;
    let matches_path = |fp: &[&str]| fp.len() == parents.len() && fp.iter().zip(parents).all(|(a, b)| a == b);
    for (_, fp, fields) in CAPABILITY_COMMON_FIELDS.iter().chain(CONTEXT_COMMON_FIELDS) {
        if matches_path(fp) {
            if let Some(f) = fields.iter().find(|f| f.key == *key) {
                return Some(f.hot);
            }
        }
    }
    for d in REGISTRY {
        if matches_path(d.fields_path) {
            if let Some(f) = d.fields.iter().find(|f| f.key == *key) {
                return Some(f.hot);
            }
        }
    }
    None
}

/// 配置段（顶层 key）→ 该段**所有已登记字段中最严格**的热生效语义。
///
/// 用途：`POST /api/config` 回报"这次保存实际多久生效"。字段级 [`field_hot`] 优先，
/// 查不到时才回退到这里（如新增了字段但忘了登记元数据）。
/// 未登记且不在 [`APP_SECTION_HOT`] 里 → **保守返回 `RestartOnly`**：
/// 宁少承诺（让用户以为要重启），也不要多承诺（说生效了其实没有）。
pub fn section_hot(section: &str) -> HotReload {
    section_hot_known(section).unwrap_or(HotReload::RestartOnly)
}

/// 同 [`section_hot`]，但**未登记时返回 `None`**（测试用它保证每个配置段都有明确出处）。
pub fn section_hot_known(section: &str) -> Option<HotReload> {
    let mut acc: Option<HotReload> = None;
    let mut fold = |h: HotReload| {
        acc = Some(match acc {
            Some(a) => a.strictest(h),
            None => h,
        });
    };
    for (_, fp, fields) in CAPABILITY_COMMON_FIELDS.iter().chain(CONTEXT_COMMON_FIELDS) {
        if fp.first().copied() == Some(section) {
            fields.iter().for_each(|f| fold(f.hot));
        }
    }
    for d in REGISTRY {
        if d.fields_path.first().copied() == Some(section) {
            d.fields.iter().for_each(|f| fold(f.hot));
        }
    }
    acc.or_else(|| {
        APP_SECTION_HOT
            .iter()
            .find(|(s, _)| *s == section)
            .map(|(_, h)| *h)
    })
}

/// 构造成立所需的外部条件（**声明式**，用于 UI 提前提示；真正的判定在 `validate`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Requirement {
    /// 需要 `--features sherpa` 编译的二进制。
    SherpaFeature,
    /// 需要本地模型文件（缺文件时容器入口脚本会自动下载）。
    ModelFiles,
    /// 需要凭据（密钥/三要素）。
    Credentials,
    /// 需要可用的 LLM 上游。
    LlmUpstream,
    /// 需要可用的 embedding 上游（记忆的可选增强；记忆接入后使用）。
    #[allow(dead_code)]
    EmbeddingUpstream,
}

impl Requirement {
    pub const fn as_str(self) -> &'static str {
        match self {
            Requirement::SherpaFeature => "sherpa_feature",
            Requirement::ModelFiles => "model_files",
            Requirement::Credentials => "credentials",
            Requirement::LlmUpstream => "llm_upstream",
            Requirement::EmbeddingUpstream => "embedding_upstream",
        }
    }
}

/// 字段控件类型（前端据此选输入控件；**不做自动布局**，只做元数据）。
#[derive(Clone, Copy, Debug)]
pub enum FieldKind {
    Str,
    /// 密钥：`GET /api/config` 只回 presence 标记，不回明文。
    Password,
    Int,
    Float,
    Bool,
    /// 枚举（第一个值为默认/推荐值）。
    Enum(&'static [&'static str]),
    Path,
    Dir,
    /// 多行散文（系统提示词/人设）。
    Text,
}

impl FieldKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            FieldKind::Str => "str",
            FieldKind::Password => "password",
            FieldKind::Int => "int",
            FieldKind::Float => "float",
            FieldKind::Bool => "bool",
            FieldKind::Enum(_) => "enum",
            FieldKind::Path => "path",
            FieldKind::Dir => "dir",
            FieldKind::Text => "text",
        }
    }
    pub const fn is_secret(&self) -> bool {
        matches!(self, FieldKind::Password)
    }
}

/// 一个配置字段的元数据。
///
/// `default_hint` 只是**展示用**的默认值文案——真实默认值仍在各 serde `default_*()` 函数里
/// （单一权威，避免两处默认值漂移；`tests::field_keys_exist_in_config` 保证 key 不漂）。
#[derive(Clone, Copy, Debug)]
pub struct FieldSchema {
    /// 字段 key（相对 `fields_path` 的 TOML/JSON 路径片段）。
    pub key: &'static str,
    /// UI 标签（中文）。
    pub label: &'static str,
    pub kind: FieldKind,
    pub default_hint: &'static str,
    /// 一句话说明 + 坑（讲**后果**，不是复述字段名）。
    pub help: &'static str,
    pub hot: HotReload,
    pub required: bool,
}

// ============================================================
// 构建结果与描述符
// ============================================================

/// 构建产物：类型擦除但**保持各 trait 不变**的闭枚举。
pub enum Built {
    Asr(Arc<dyn AsrEngine>),
    Tts(Arc<dyn TtsEngine>),
    Llm(Llm),
    /// 图记忆（可选能力：坏掉只降级，不影响语音）。
    Memory(Arc<dyn crate::plugins::memory::MemoryProvider>),
    /// 每会话构造（AIUI 全链路）：登记能力与依赖，但不产出共享实例。
    /// 当前没有描述符构造它（`AiuiSession::new` 需要每会话的 sn），保留为类型契约。
    #[allow(dead_code)]
    FullChain(Arc<dyn FullChainEngine>),
}

/// 一个实现（implementation）的完整描述。
pub struct PluginDescriptor {
    /// 命名空间唯一 id：`<capability>.<impl>`，如 `tts.kokoro`。
    pub id: &'static str,
    pub capability: Capability,
    pub display: &'static str,
    /// 本地实现（离线）还是远程服务（在线 API）——UI 两级下拉的依据。
    pub local: bool,
    /// 该实现整体的热生效语义（字段级可覆盖，[`FieldSchema::hot`]）。
    pub hot: HotReload,
    pub requires: &'static [Requirement],
    /// 字段所在配置路径（如 `["tts","xfyun"]`）；`[]` 表示该实现无独立配置段。
    pub fields_path: &'static [&'static str],
    pub fields: &'static [FieldSchema],
    /// 热切换判定：由**实现自己**声明哪些字段影响成败（取代 `engine.rs` 手写的 `tts_sig`）。
    pub signature: fn(&Config) -> String,
    /// 构建前的前置校验：错误文案必须含「怎么修」（对齐 DSH 的 inject 校验前移）。
    pub validate: fn(&Config) -> Result<()>,
    /// 构建入口；`None` = 无共享实例（每会话构造 / 纯服务型能力）。
    pub build: Option<fn(&Config) -> Result<Built>>,
    /// **软性问题**（能构建、但能力受限）：`Some(reason)` → 状态报 `Phase::Degraded` + 原因。
    /// 例：`[llm].api_key` 为空（服务能起来，但每次对话都会在上游 401）。
    /// 与 `validate` 的分工：`validate` = 硬条件（不满足无法构建）；`warn_if` = 降级提示。
    pub warn_if: Option<fn(&Config) -> Option<String>>,
    /// 该实现当前是否被配置选中（同一能力下多实现互斥，依据 `[<cap>].engine`；
    /// 没有 `engine` 键的能力用各自的开关，如 `[aiui].enabled` / `[soul].enabled`）。
    pub selected: fn(&Config) -> bool,
}

impl PluginDescriptor {
    /// 该实现当前是否启用（未选中 = `Phase::Disabled`）。
    pub fn is_selected(&self, cfg: &Config) -> bool {
        (self.selected)(cfg)
    }
}
/// 全量注册表（顺序即 UI 展示顺序）。
pub static REGISTRY: &[PluginDescriptor] = &[
    ASR_SENSEVOICE,
    VAD_SILERO,
    TTS_KOKORO,
    TTS_XFYUN,
    LLM_OPENAI,
    AIUI_FULLCHAIN,
    VOICES_CATALOG,
    FIRMWARE_HOST,
    MEMORY_GRAPH,
    SOUL_PROFILE,
    COMMAND_GATE,
];

/// 按 id 查描述符。
pub fn find(id: &str) -> Option<&'static PluginDescriptor> {
    REGISTRY.iter().find(|d| d.id == id)
}

/// 某能力下的全部实现。
pub fn impls_of(cap: Capability) -> impl Iterator<Item = &'static PluginDescriptor> {
    REGISTRY.iter().filter(move |d| d.capability == cap)
}

/// 某能力当前**被选中**的实现 id（无实现/未启用时返回 `None`）。
pub fn active_impl(cfg: &Config, cap: Capability) -> Option<&'static str> {
    impls_of(cap).find(|d| d.is_selected(cfg)).map(|d| d.id)
}

/// 描述符 id 的**实现段**：`tts.kokoro` → `kokoro`（= `engine` 键的取值）。
pub fn impl_id(descriptor_id: &str) -> &str {
    descriptor_id
        .split_once('.')
        .map(|(_, impl_part)| impl_part)
        .unwrap_or(descriptor_id)
}

/// 某能力配置里的 `engine` 值（规范化后；`None` = 该能力没有 engine 选择器）。仅用于诊断文案。
pub fn configured_engine(cfg: &Config, cap: Capability) -> Option<String> {
    match cap {
        Capability::Asr => Some(cfg.asr.engine_id()),
        Capability::Vad => Some(cfg.vad.engine_id()),
        Capability::Tts => Some(cfg.tts.engine_id()),
        Capability::Llm => Some(cfg.llm.engine_id()),
        Capability::Memory => Some(cfg.memory.engine_id().to_string()),
        Capability::FullChain | Capability::Voices | Capability::Firmware | Capability::Soul
        | Capability::Command => None,
    }
}

/// 诊断：配置的 `engine` 值**不在**该能力的实现表里（= 没有任何实现会被选中）。
///
/// 存在的意义：P4 的 `engine` 是**显式选择**，写错时若静默回退默认，用户会以为配置生效了。
/// 这里让 `/api/plugins` 与启动错误都能直接说出"你写了什么、可选什么"。
pub fn unknown_engine_of(cfg: &Config, cap: Capability) -> Option<String> {
    let raw = configured_engine(cfg, cap)?;
    // `[memory].engine = "none"` 是**合法的显式停用**（闭枚举），不是"未知实现"。
    if cap == Capability::Memory && raw == "none" {
        return None;
    }
    let hit = impls_of(cap).any(|d| impl_id(d.id) == raw);
    (!hit).then_some(raw)
}

/// 「怎么写才对」的完整提示（含可选值）。
pub fn unknown_engine_hint(cfg: &Config, cap: Capability) -> String {
    let raw = configured_engine(cfg, cap).unwrap_or_default();
    let known: Vec<&str> = impls_of(cap).map(|d| impl_id(d.id)).collect();
    format!(
        "[{}].engine = \"{raw}\" 不是已知实现：可选 {}。请改成一个可选值，或删掉该键使用默认实现。",
        cap.id(),
        known.join(" / ")
    )
}

/// 某能力当前选中实现的**热切换签名**（`PluginHost`/会话缓存键共用同一口径）。
pub fn signature_of(cfg: &Config, cap: Capability) -> String {
    active_impl(cfg, cap)
        .and_then(find)
        .map(|d| (d.signature)(cfg))
        .unwrap_or_default()
}

// ============================================================
// JSON 投影（手写 wire format，保证 API 契约稳定、可单测）
// ============================================================

/// 字段元数据 → JSON（`GET /api/config/schema` 用）。
pub fn field_json(f: &FieldSchema) -> serde_json::Value {
    let mut v = serde_json::json!({
        "key": f.key,
        "label": f.label,
        "kind": f.kind.as_str(),
        // 密钥字段只回类型标记：管理页据此**不回显明文**、留空即保留原值
        "secret": f.kind.is_secret(),
        "default_hint": f.default_hint,
        "help": f.help,
        "hot": f.hot.as_str(),
        "hot_hint": f.hot.hint(),
        "required": f.required,
    });
    if let FieldKind::Enum(options) = f.kind {
        v["options"] = serde_json::json!(options);
    }
    v
}

fn fields_json(fields: &[FieldSchema]) -> serde_json::Value {
    serde_json::Value::Array(fields.iter().map(field_json).collect())
}

/// `GET /api/config/schema` 响应体：按能力聚合（公共字段 + 各实现的私有字段）。
///
/// ⚠️ 这是**元数据**，不是自动表单：前端 tab 仍手写（DSH 的 `autoGenerate` 至今无客户端使用，
/// 见 `docs/plugin-architecture-unification.md` §11.1）。这里解决的是"字段默认值/热生效语义/
/// 密钥标记/校验提示"在前后端各写一遍导致的漂移。
pub fn schema_json() -> serde_json::Value {
    let capabilities: Vec<serde_json::Value> = ALL_CAPABILITIES
        .iter()
        .map(|&cap| {
            let common: Vec<serde_json::Value> = capability_common_fields(cap)
                .map(|(path, fields)| {
                    serde_json::json!({ "fields_path": path, "fields": fields_json(fields) })
                })
                .collect();
            let impls: Vec<serde_json::Value> = impls_of(cap)
                .map(|d| {
                    serde_json::json!({
                        "id": d.id,
                        "display": d.display,
                        "local": d.local,
                        "hot": d.hot.as_str(),
                        "hot_hint": d.hot.hint(),
                        "requires": d.requires.iter().map(|r| r.as_str()).collect::<Vec<_>>(),
                        "fields_path": d.fields_path,
                        "fields": fields_json(d.fields),
                    })
                })
                .collect();
            serde_json::json!({
                "id": cap.id(),
                "display": cap.display(),
                "required": cap.required(),
                "implemented": cap.implemented(),
                "common_fields": common,
                "implementations": impls,
            })
        })
        .collect();
    serde_json::json!({
        "capabilities": capabilities,
        // **非插件**配置段（`[server]` / `[audio]`）的档位：它们不在能力注册表里，
        // 但管理页同样需要"改这段多久生效"的权威答案（此前只能靠卡片文案口口相传）。
        "app_sections": APP_SECTION_HOT
            .iter()
            .map(|(section, hot)| serde_json::json!({
                "id": section,
                "hot": hot.as_str(),
                "hot_hint": hot.hint(),
            }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests;
