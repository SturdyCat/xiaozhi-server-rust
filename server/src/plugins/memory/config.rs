//! 记忆插件的配置层：`[memory]` / `[memory.retention]` / `[memory.extractor]`
//! 与引擎枚举 [`MemoryBackend`]。
//!
//! 本文件由 `plugins/memory/mod.rs` 拆分而来（纯移动，不改任何逻辑与字段语义）；
//! 对外路径经 `mod.rs` 的 `pub use` 保持不变（`crate::plugins::memory::MemoryConfig` 等）。

use serde::{Deserialize, Serialize};

/// 记忆引擎（`[memory].engine`；P4 前叫 `backend`）。
///
/// P1-7 起支持多后端注册表（见 `docs/p1-round1-design.md` §1.2），P1-9 加 rag：
/// - `graph`（默认）：内置图记忆，抽取 + 摘要召回；
/// - `store`：纯存储——只落对话原文（`extraction_state='stored'`），零 LLM、零召回，
///   语义定位是"先积累再决定"；
/// - `rag`（P1-9）：本地文档 RAG——扫描 `docs_dir`（txt/md）建词法倒排索引，
///   `recall` 检索文档分块注入上下文；对话原文照常落库（与 store 同表语义）。
///   零 LLM、零外网；
/// - `none`：显式停用（保留配置但不起作用）。
///
/// ⚠️ `Deserialize` 是**手写**的（[下面][impl]）：serde derive 对未知变体的报错是英文
/// （`unknown variant ... expected one of ...`），不满足「怎么修」中文文案口径（NF-3）；
/// `Serialize` 保持 derive（写出侧无此问题）。宽容度对齐 `canonical_engine`：trim + 小写。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryBackend {
    /// 内置图记忆（SQLite，本地词法召回，零外网依赖）。
    Graph,
    /// 纯存储（P1-7）：只把对话原文落 SQLite，不抽取（零辅助 LLM 调用）、`recall` 恒空。
    /// 与 graph 的 `extraction_enabled` 开关**分层**：backend 选引擎，开关管 graph 内部行为。
    Store,
    /// 本地文档 RAG（P1-9）：扫描目录建词法索引，召回文档分块；零 LLM 零外网。
    Rag,
    /// 显式关闭（保留配置但不起作用，便于临时停用而不丢参数）。
    None,
}

impl Default for MemoryBackend {
    fn default() -> Self {
        MemoryBackend::Graph
    }
}

/// `[memory].engine` 合法取值（中文报错文案与 schema options 的单一出处）。
pub const MEMORY_ENGINE_VALUES: &[&str] = &["graph", "store", "rag", "none"];

impl MemoryBackend {
    /// 字符串 → 变体（trim + 小写归一；未知值返回 `None` 由调用方报错）。
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "graph" => Some(MemoryBackend::Graph),
            "store" => Some(MemoryBackend::Store),
            "rag" => Some(MemoryBackend::Rag),
            "none" => Some(MemoryBackend::None),
            _ => None,
        }
    }
}

/// 手写反序列化：接受字符串（含 TOML 的裸字面量），未知值给出中文「怎么修」报错。
///
/// 错误范本（对齐 `registry::unknown_engine_hint` 的口径）：
/// `[memory].engine = "store2" 不是已知后端：可选 graph / store / rag / none。…`
/// `Config::load`（启动报错）与 `PUT /api/config`（400）两条路径都在这一层统一拦截。
impl<'de> Deserialize<'de> for MemoryBackend {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = MemoryBackend;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("[memory].engine 的取值（graph / store / rag / none）")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<MemoryBackend, E> {
                MemoryBackend::parse(v).ok_or_else(|| {
                    E::custom(format!(
                        "[memory].engine = \"{v}\" 不是已知后端：可选 graph / store / rag / none。\
                         请改成一个可选值，或删掉该键使用默认 graph（旧键 backend 仍可读）。"
                    ))
                })
            }
        }
        deserializer.deserialize_str(V)
    }
}

fn default_db_path() -> String {
    "/data/memory.db".into()
}
fn default_busy_timeout() -> u64 {
    5000
}
fn default_true() -> bool {
    true
}
fn default_fresh_turns() -> u32 {
    5
}
fn default_recall_nodes() -> usize {
    6
}
fn default_recall_tokens() -> u32 {
    400
}
fn default_recall_budget_ms() -> u64 {
    300
}
fn default_maintenance_interval() -> u32 {
    6
}
fn default_quarantine_attempts() -> u32 {
    3
}
fn default_scope_prefix() -> String {
    "xiaozhi".into()
}
fn default_inject_position() -> String {
    "after_history".into()
}
fn default_keep() -> String {
    "all".into()
}
fn default_extract_temperature() -> f32 {
    0.1
}
fn default_rag_docs_dir() -> String {
    "/data/rag-docs".into()
}
fn default_rag_chunk_size() -> u32 {
    512
}
fn default_rag_chunk_overlap() -> u32 {
    64
}
fn default_rag_top_k() -> u32 {
    4
}

/// 记忆配置（`[memory]`）。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryConfig {
    /// 总开关。`false`（默认）= [`NoopMemory`]，零依赖零成本。
    #[serde(default)]
    pub enabled: bool,
    /// 引擎 id（P4 约定，见 `docs/plugin-architecture-unification.md` §3.3）：
    /// `graph`（默认，内置图记忆）| `store`（纯存储：只落原文，零 LLM 零召回，P1-7）|
    /// `none`（显式停用，等价 `enabled=false`）。
    /// 旧键 `backend` 经 serde `alias` 仍可读（保存后写 `engine`）；
    /// 未知值由 [`MemoryBackend`] 的自定义 `Deserialize` 报中文错误。
    #[serde(default, alias = "backend")]
    pub engine: MemoryBackend,
    /// SQLite 库路径。容器部署请放在挂载卷内（如 /data/memory.db），否则重建容器即丢。
    #[serde(default = "default_db_path")]
    pub db_path: String,
    /// SQLite 写锁等待（与 DSH 进程共用同一 db 文件时尤其重要）。
    #[serde(default = "default_busy_timeout")]
    pub db_busy_timeout_ms: u64,
    /// 是否启用召回（关掉 = 只写不读，便于先积累数据）。
    #[serde(default = "default_true")]
    pub recall_enabled: bool,
    /// 是否启用抽取（关掉 = 只存原文，不做辅助 LLM 调用、不产生摘要）。
    #[serde(default = "default_true")]
    pub extraction_enabled: bool,
    /// 近 N 轮原文保留在会话上下文里；这 N 轮**不参与召回**（避免重复注入）。
    #[serde(default = "default_fresh_turns")]
    pub fresh_turn_count: u32,
    /// 每次最多召回几条轮次记忆。
    #[serde(default = "default_recall_nodes")]
    pub recall_max_nodes: usize,
    /// 注入正文的 token 预算（0 = 不限；中文按 ≈1 token/字估算）。
    #[serde(default = "default_recall_tokens")]
    pub recall_max_tokens: u32,
    /// 关键路径召回预算（毫秒）。超时立即按"无记忆"继续，不拖慢 TTFA。
    #[serde(default = "default_recall_budget_ms")]
    pub recall_budget_ms: u64,
    /// 每 N 轮做一次维护（保留策略 + 隔离重试）。
    #[serde(default = "default_maintenance_interval")]
    pub maintenance_interval: u32,
    /// 记忆块注入位置：`after_history`（默认，护住前缀缓存）/ `before_history`（recency 更强）。
    #[serde(default = "default_inject_position")]
    pub inject_position: String,
    /// 抽取失败的最大尝试次数，超过即永久隔离（等 `maintain` 或人工处理）。
    #[serde(default = "default_quarantine_attempts")]
    pub quarantine_max_attempts: u32,
    /// 作用域前缀：`scope = <prefix>:<device_id>`（跨会话稳定，是"记忆"成立的前提）。
    #[serde(default = "default_scope_prefix")]
    pub scope_prefix: String,
    #[serde(default)]
    pub retention: RetentionConfig,
    /// 抽取专用模型路由（字段全空 = 复用 `[llm]`）。
    #[serde(default)]
    pub extractor: ExtractorConfig,
    /// 本地文档 RAG 参数组（P1-9，`[memory.rag]`；仅 `engine = "rag"` 时生效）。
    #[serde(default)]
    pub rag: RagConfig,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            engine: MemoryBackend::Graph,
            db_path: default_db_path(),
            db_busy_timeout_ms: default_busy_timeout(),
            recall_enabled: true,
            extraction_enabled: true,
            fresh_turn_count: default_fresh_turns(),
            recall_max_nodes: default_recall_nodes(),
            recall_max_tokens: default_recall_tokens(),
            recall_budget_ms: default_recall_budget_ms(),
            maintenance_interval: default_maintenance_interval(),
            inject_position: default_inject_position(),
            quarantine_max_attempts: default_quarantine_attempts(),
            scope_prefix: default_scope_prefix(),
            retention: RetentionConfig::default(),
            extractor: ExtractorConfig::default(),
            rag: RagConfig::default(),
        }
    }
}

/// 原文保留策略（`[memory.retention]`）。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RetentionConfig {
    /// `all`（默认，全部保留）/ `referenced`（只留被记忆引用的原文）/ `recent`（最近 N 轮之外的未引用原文）。
    #[serde(default = "default_keep")]
    pub keep: String,
    /// `keep = "recent"` 时保留的轮数。
    #[serde(default)]
    pub recent_turns: u32,
    /// 超过 N 天的**未引用**原文可删（0 = 不限）。
    #[serde(default)]
    pub retention_days: u32,
    /// 只演练不删除（先看会删多少条）。
    #[serde(default)]
    pub dry_run: bool,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            keep: default_keep(),
            recent_turns: 0,
            retention_days: 0,
            dry_run: false,
        }
    }
}

/// 抽取模型路由（`[memory.extractor]`）；字段全空表示复用 `[llm]`。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExtractorConfig {
    #[serde(default)]
    pub api_base: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model: String,
    /// 抽取温度（默认 0.1：要稳定，不要发挥）。
    #[serde(default = "default_extract_temperature")]
    pub temperature: f32,
}

/// ⚠️ 必须手写 `Default`：`#[derive(Default)]` 会让 `temperature` 取 `0.0`，
/// 而 serde 的字段级 `default = "default_extract_temperature"` 只在**表存在但字段缺失**时
/// 生效 —— 两者不一致会导致"[memory.extractor] 整段不写"与"写了表但省略 temperature"
/// 得到不同行为（抽取温度 0.0 vs 0.1），且单测很难覆盖到。
impl Default for ExtractorConfig {
    fn default() -> Self {
        Self {
            api_base: String::new(),
            api_key: String::new(),
            model: String::new(),
            temperature: default_extract_temperature(),
        }
    }
}

/// 本地文档 RAG 参数组（P1-9，`[memory.rag]`）；仅 `engine = "rag"` 时生效。
///
/// ⚠️ 与 [`ExtractorConfig`] 同理必须手写 `Default`：字段级 serde `default_*` 只在
/// "表存在但字段缺失"时生效，整段不写走 `impl Default`——两者必须一致。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RagConfig {
    /// 文档目录（服务器本地，容器放挂载卷内）。目录不存在/为空 = 零知识、零影响
    /// （对话完全正常，这是**正常态**不是降级）。仅扫描 `*.txt` / `*.md`。
    #[serde(default = "default_rag_docs_dir")]
    pub docs_dir: String,
    /// 分块目标长度（字符；按空行分段后超长段再按此滑窗）。
    #[serde(default = "default_rag_chunk_size")]
    pub chunk_size: u32,
    /// 相邻分块的重叠字符数（保住跨块边界的语义）。
    #[serde(default = "default_rag_chunk_overlap")]
    pub chunk_overlap: u32,
    /// 每次召回最多注入几个分块。
    #[serde(default = "default_rag_top_k")]
    pub top_k: u32,
}

impl Default for RagConfig {
    fn default() -> Self {
        Self {
            docs_dir: default_rag_docs_dir(),
            chunk_size: default_rag_chunk_size(),
            chunk_overlap: default_rag_chunk_overlap(),
            top_k: default_rag_top_k(),
        }
    }
}

impl MemoryConfig {
    /// 签名（热刷新判定用；含密钥，**绝不打印**）。
    pub fn signature(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// 记忆是否真正生效（启用 + 后端非 none）。
    ///
    /// P1-7 语义扩展：原为 `enabled && engine == Graph`，多后端后改为**任何非 none 后端
    /// 都算生效**——store 也要通过 [`crate::app::session::reply`] 的 `spawn_memory_write`
    /// 门落原文（这正是"纯存储仍要记录"的本意）；召回门是 `recall_enabled`，
    /// store 后端由 `StoreOnlyMemory::recall` 恒返空兜底，不依赖该开关。
    pub fn active(&self) -> bool {
        self.enabled && self.engine != MemoryBackend::None
    }

    /// 规范化后的引擎 id（`graph` / `store` / `rag` / `none`）。
    /// 取值由 [`MemoryBackend`] 枚举闭死：未知值在**反序列化**阶段即报错（不需运行期校验）。
    pub fn engine_id(&self) -> &'static str {
        match self.engine {
            MemoryBackend::Graph => "graph",
            MemoryBackend::Store => "store",
            MemoryBackend::Rag => "rag",
            MemoryBackend::None => "none",
        }
    }

    /// 规范化（P4）：memory 的 engine 是闭枚举，serde 已完成取值校验，此处仅占位以统一调用点。
    pub fn normalize(&mut self) {}

    /// 作用域键：`<prefix>:<device_id>`。
    pub fn scope_for(&self, device_id: &str) -> String {
        let id = device_id.trim();
        let id = if id.is_empty() { "unknown" } else { id };
        format!("{}:{}", self.scope_prefix.trim(), id)
    }

    /// 注入位置。
    pub fn inject_position(&self) -> crate::plugins::prompt::MemoryPosition {
        crate::plugins::prompt::MemoryPosition::parse(&self.inject_position)
    }

    /// 校验（错误文案含「怎么修」）。启用但配置不全时**只降级**，不阻止启动。
    pub fn validate(&self) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if self.db_path.trim().is_empty() {
            anyhow::bail!("[memory].db_path 为空：请填可写路径（容器建议 /data/memory.db，并在 compose 挂载卷）。");
        }
        if self.engine == MemoryBackend::None {
            anyhow::bail!("[memory].enabled = true 但 engine = \"none\"：要么改成 \"graph\"，要么把 enabled 改为 false。");
        }
        if self.scope_prefix.trim().is_empty() {
            anyhow::bail!(
                "[memory].scope_prefix 为空：它决定记忆按哪台设备隔离，请填如 \"xiaozhi\"。"
            );
        }
        if !(1..=20).contains(&self.recall_max_nodes) {
            anyhow::bail!(
                "[memory].recall_max_nodes = {} 超出范围：请取 1~20（默认 6）。",
                self.recall_max_nodes
            );
        }
        // rag 参数组校验（仅 engine = rag 时生效；docs_dir 不存在/为空是正常态，不校验）
        if self.engine == MemoryBackend::Rag {
            if self.rag.docs_dir.trim().is_empty() {
                anyhow::bail!(
                    "[memory.rag].docs_dir 为空：请填服务器上放文档的目录\
                    （如 /data/rag-docs，容器请在 compose 挂载卷内）。"
                );
            }
            if self.rag.chunk_size < 64 {
                anyhow::bail!(
                    "[memory.rag].chunk_size = {} 太小：请取 ≥ 64（默认 512）。",
                    self.rag.chunk_size
                );
            }
            if self.rag.chunk_overlap >= self.rag.chunk_size {
                anyhow::bail!(
                    "[memory.rag].chunk_overlap = {} 必须小于 chunk_size = {}：\
                     请调小 overlap 或调大 chunk_size。",
                    self.rag.chunk_overlap,
                    self.rag.chunk_size
                );
            }
            if !(1..=20).contains(&self.rag.top_k) {
                anyhow::bail!(
                    "[memory.rag].top_k = {} 超出范围：请取 1~20（默认 4）。",
                    self.rag.top_k
                );
            }
        }
        Ok(())
    }

    /// 面向 UI 的软性提示（`Phase::Degraded` 的原因；`None` = 一切正常）。
    ///
    /// ⚠️ 本方法只被 **graph 描述符**的 `warn_if` 调用（store 描述符 `warn_if: None`——
    /// "不召回"是 store 的设计定位而非能力受限，不构成降级理由）。
    pub fn warning(&self, llm: &crate::config::LlmConfig) -> Option<String> {
        // 只对 graph 后端给出软性提示（store 的定位说明走配置页文案，不进状态）
        if self.enabled && self.engine != MemoryBackend::Graph {
            return None;
        }
        if !self.active() {
            return None;
        }
        let mut notes = Vec::new();
        if self.extraction_enabled && self.extractor.resolve_missing(llm) {
            notes.push(
                "抽取未配置模型路由且 [llm] 无凭据：原文仍会记录，但不会生成摘要（召回命中率低）。\
                 请在 [memory.extractor] 填 api_base/api_key/model，或配好 [llm]。"
                    .to_string(),
            );
        }
        if !self.extraction_enabled {
            notes.push(
                "抽取已关闭（extraction_enabled=false）：只写原文、不生成摘要，召回依赖不到摘要。"
                    .into(),
            );
        }
        notes.push("无 embedding：当前为本地词法召回（摘要汉字二元组索引），零外网调用".into());
        Some(notes.join("；"))
    }
}

impl ExtractorConfig {
    /// 是否所有字段都空（→ 复用 `[llm]`）。
    /// 当前调用方经 `resolve_missing` 间接覆盖同一判定；保留为公共原语。
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.api_base.trim().is_empty()
            && self.model.trim().is_empty()
            && self.api_key.trim().is_empty()
    }

    /// 解析出实际使用的 LLM 配置：本段非空则**逐字段**覆盖 `[llm]`（允许只换模型）。
    pub fn resolve(&self, llm: &crate::config::LlmConfig) -> crate::config::LlmConfig {
        let mut c = llm.clone();
        if !self.api_base.trim().is_empty() {
            c.api_base = self.api_base.clone();
        }
        if !self.api_key.trim().is_empty() {
            c.api_key = self.api_key.clone();
        }
        if !self.model.trim().is_empty() {
            c.model = self.model.clone();
        }
        c.temperature = self.temperature;
        // 抽取不需要流式（要的是完整 JSON），关掉省掉 SSE 解析与首包延迟
        c.stream = false;
        c
    }

    /// 抽取路由是否缺凭据（复用 `[llm]` 时看 `[llm]`）。
    pub fn resolve_missing(&self, llm: &crate::config::LlmConfig) -> bool {
        let c = self.resolve(llm);
        c.api_base.trim().is_empty() || c.api_key.trim().is_empty() || c.model.trim().is_empty()
    }
}
