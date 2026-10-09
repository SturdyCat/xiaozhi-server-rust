//! 记忆插件：`MemoryProvider` trait + `[memory]` 配置 + 图记忆实现（SQLite）。
//!
//! 这是"把 LLM 之前的一层可插拔上下文生产者"落到本仓库的第二个生产者
//! （第一个是 [`crate::plugins::soul`]）。设计见 `docs/soul-and-graph-memory-plan.md`。
//!
//! ## 分层（对齐 graph-memory 的架构，不是照抄代码）
//!
//! - **事实源**：`gm_messages`（一轮 = user + assistant 两条，不可变）；
//! - **导航层**：`gm_turn_memories`（每轮一条自包含摘要）+ `gm_summary_terms`（我们的倒排索引）；
//! - **证据层**：注入正文来自**原始 Q/A**（`gm_turn_memory_sources` 绑定），不是摘要本身；
//! - **关键路径**：`recall()` 有硬预算，超时/失败**静默降级为空**（绝不让记忆拖慢语音）；
//! - **后台路径**：`record()` 只落原文（快），`extract_pending()` 才做那一次辅助 LLM 调用，
//!   失败入隔离表、有限重试、**永不阻塞对话**。
//!
//! ## 默认零成本
//!
//! `[memory].enabled = false`（默认）时装配为 [`NoopMemory`]：不建库、不读盘、不做任何
//! LLM 调用，行为与今天完全一致 —— 这是记忆功能能被放心打开的底线。
//!
//! ## 与设计文档的偏差（详见 `docs/plugin-architecture-unification.md` §0.9）
//!
//! - **未做** embedding / 语义 Top-K / PPR / LPA 社区 / 三元组图（P2）：v1 用
//!   「摘要倒排索引（汉字二元组）」做词法召回，中文场景可用且零外网调用；
//! - **未做** `[memory.embedding]` 配置段：不暴露"配了不生效"的旋钮；
//! - `retention` 已实现，且修掉了上游"误删被引用的证据"的缺陷。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub mod assemble;
mod graph;
mod llm_extract;
mod schema;
mod store;
mod terms;
pub use graph::GraphMemory;
// 作为插件对外 API 面保留（bin crate 内暂无直接引用者；抽取链路的单测经 `super::*` 使用）
#[allow(unused_imports)]
pub use llm_extract::{Extraction, EXTRACT_INSTRUCTIONS, OUTCOMES};

/// 记忆引擎（`[memory].engine`；P4 前叫 `backend`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryBackend {
    /// 内置图记忆（SQLite，本地词法召回，零外网依赖）。
    Graph,
    /// 显式关闭（保留配置但不起作用，便于临时停用而不丢参数）。
    None,
}

impl Default for MemoryBackend {
    fn default() -> Self {
        MemoryBackend::Graph
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

/// 记忆配置（`[memory]`）。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MemoryConfig {
    /// 总开关。`false`（默认）= [`NoopMemory`]，零依赖零成本。
    #[serde(default)]
    pub enabled: bool,
    /// 引擎 id（P4 约定，见 `docs/plugin-architecture-unification.md` §3.3）：
    /// `graph`（默认，内置图记忆）| `none`（显式停用，等价 `enabled=false`）。
    /// 旧键 `backend` 经 serde `alias` 仍可读（保存后写 `engine`）。
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

impl MemoryConfig {
    /// 签名（热刷新判定用；含密钥，**绝不打印**）。
    pub fn signature(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// 记忆是否真正生效（启用 + 后端非 none）。
    pub fn active(&self) -> bool {
        self.enabled && self.engine == MemoryBackend::Graph
    }

    /// 规范化后的引擎 id（`graph` / `none`）。
    /// 取值由 [`MemoryBackend`] 枚举闭死：未知值在**反序列化**阶段即报错（不需运行期校验）。
    pub fn engine_id(&self) -> &'static str {
        match self.engine {
            MemoryBackend::Graph => "graph",
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
        Ok(())
    }

    /// 面向 UI 的软性提示（`Phase::Degraded` 的原因；`None` = 一切正常）。
    pub fn warning(&self, llm: &crate::config::LlmConfig) -> Option<String> {
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

/// 召回结果（**从不返回 `Err`**：记忆失败不该影响对话，错误只用于日志/状态页）。
#[derive(Debug, Clone, Default)]
pub struct RecallResult {
    /// 已渲染的注入正文；`None` = 无命中/关闭/失败。
    pub block: Option<String>,
    pub hits: usize,
    pub matched_terms: usize,
    pub elapsed_ms: u128,
    /// 失败原因（仅用于日志与状态页；会话侧忽略）。
    pub error: Option<String>,
}

impl RecallResult {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "hits": self.hits,
            "matched_terms": self.matched_terms,
            "elapsed_ms": self.elapsed_ms,
            "injected": self.block.is_some(),
            "preview": self.block,
            "error": self.error,
        })
    }
}

/// 一轮已完成的对话（写入记忆的输入）。
#[derive(Debug, Clone)]
pub struct CompletedTurn {
    pub scope: String,
    pub session_id: String,
    pub user_text: String,
    pub assistant_text: String,
}

/// 抽取批次的统计。
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq)]
pub struct ExtractOutcome {
    pub attempted: u32,
    pub succeeded: u32,
    pub failed: u32,
}

/// 维护结果（`GET /api/memory/status` 与「立即维护」动作都要能看到"到底做了什么"）。
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq)]
pub struct MaintainOutcome {
    /// 是否真的执行了（`false` = 未到维护间隔，被跳过）。
    pub ran: bool,
    /// 从隔离放回队列的轮次数。
    pub released: u64,
    /// 保留策略处理的原文条数（`dry_run` 时为"将会删除"的数量）。
    pub deleted: u64,
    pub dry_run: bool,
}

/// 维护调用的选项（会话周期调用 vs 管理页「立即维护」）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MaintainOpts {
    /// `true` = 忽略 `maintenance_interval` 立即执行（管理页动作）。
    pub force: bool,
    /// `true` = 只演练不删除（先看清会处理多少条）。
    pub dry_run: bool,
}

impl MaintainOpts {
    /// 会话每轮调用：由实现按间隔自行决定是否真的跑。
    pub fn periodic() -> Self {
        Self::default()
    }

    /// 管理页「立即维护」。
    pub fn forced(dry_run: bool) -> Self {
        Self {
            force: true,
            dry_run,
        }
    }
}

impl MaintainOutcome {
    pub fn skipped() -> Self {
        Self::default()
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ran": self.ran,
            "released": self.released,
            "deleted": self.deleted,
            "dry_run": self.dry_run,
        })
    }
}

/// 记忆状态快照（`GET /api/memory/status` 与管理页状态卡）。
#[derive(Debug, Clone, Default)]
pub struct MemoryStats {
    /// 引擎 id（`graph` / `none`；P4 起 wire 键也叫 `engine`）。
    pub engine: &'static str,
    /// 记忆是否真正生效（启用 + 后端可用）。
    pub active: bool,
    pub db_path: String,
    pub db_bytes: u64,
    pub messages: i64,
    pub memories: i64,
    pub pending: i64,
    pub quarantined: i64,
    /// 各作用域（哪台设备有多少条记忆）。
    pub scopes: Vec<(String, i64)>,
    /// 当前是否为"无 embedding 的词法模式"（UI 据此显示降级原因）。
    pub lexical_only: bool,
    /// 抽取用的模型路由（**不含密钥**），如 `https://… · gpt-4o-mini`。
    pub extractor: String,
    pub error: Option<String>,
}

impl MemoryStats {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "engine": self.engine,
            "active": self.active,
            "db_path": self.db_path,
            "db_bytes": self.db_bytes,
            "messages": self.messages,
            "memories": self.memories,
            "pending": self.pending,
            "quarantined": self.quarantined,
            "scopes": self.scopes.iter().map(|(s, n)| serde_json::json!({"scope": s, "memories": n})).collect::<Vec<_>>(),
            "lexical_only": self.lexical_only,
            "extractor": self.extractor,
            "error": self.error,
        })
    }
}

/// 手动装箱 future（与 [`crate::plugins::llm::LlmProvider`] 同风格，不引 async-trait）。
pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 记忆提供方 trait。
///
/// ⚠️ 语义约定（调用方依赖）：
/// - [`Self::recall`] **绝不返回 `Err`**、**绝不长时间阻塞**（超预算即当作无命中）；
/// - [`Self::record`] 只落原文（一次本地 DB 写），**不做 LLM 调用**；
/// - [`Self::extract_pending`] 才做辅助 LLM 调用，由调用方放在**后台任务**里。
pub trait MemoryProvider: Send + Sync {
    /// 实现名（`graph` / `none`）。
    fn name(&self) -> &'static str;

    /// 关键路径召回：给定本轮用户文本，返回已渲染的背景资料块。
    /// `scope = None` 表示跨作用域检索（管理页的「测试召回」用）。
    fn recall<'a>(&'a self, scope: Option<&'a str>, query: &'a str) -> BoxFut<'a, RecallResult>;

    /// 写入一轮事实（user + assistant 原文，置为待抽取）。
    fn record<'a>(&'a self, turn: CompletedTurn) -> BoxFut<'a, anyhow::Result<()>>;

    /// 处理最多 `limit` 个待抽取轮次（每个 = 1 次辅助 LLM 调用）。
    fn extract_pending(&self, limit: usize) -> BoxFut<'_, ExtractOutcome>;

    /// 低频维护：保留策略 + 隔离重试释放。
    ///
    /// `force = false` 时由实现按 `maintenance_interval` 自行判断是否需要跑
    ///（会话每轮调用，代价只有一次原子读）；`force = true` 是管理页的「立即维护」动作。
    fn maintain(&self, opts: MaintainOpts) -> BoxFut<'_, anyhow::Result<MaintainOutcome>>;

    /// 状态快照（管理页状态卡）。
    fn stats(&self) -> MemoryStats;

    /// 清空记忆（`scope = None` = 全部）；返回删除的轮次记忆条数。
    fn clear(&self, scope: Option<String>) -> BoxFut<'_, anyhow::Result<u64>>;
}

/// 默认实现：什么都不做（`[memory].enabled = false`）。
///
/// 存在的意义不只是"省事"：它让**关闭记忆时的代码路径与打开时完全一致**，
/// 于是"关掉记忆就回到今天的行为"是结构保证，而不是靠分支小心维护。
pub struct NoopMemory;

impl MemoryProvider for NoopMemory {
    fn name(&self) -> &'static str {
        "none"
    }

    fn recall<'a>(&'a self, _scope: Option<&'a str>, _query: &'a str) -> BoxFut<'a, RecallResult> {
        Box::pin(async { RecallResult::none() })
    }

    fn record<'a>(&'a self, _turn: CompletedTurn) -> BoxFut<'a, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }

    fn extract_pending(&self, _limit: usize) -> BoxFut<'_, ExtractOutcome> {
        Box::pin(async { ExtractOutcome::default() })
    }

    fn maintain(&self, _opts: MaintainOpts) -> BoxFut<'_, anyhow::Result<MaintainOutcome>> {
        Box::pin(async { Ok(MaintainOutcome::skipped()) })
    }

    fn stats(&self) -> MemoryStats {
        MemoryStats {
            engine: "none",
            active: false,
            lexical_only: true,
            ..MemoryStats::default()
        }
    }

    fn clear(&self, _scope: Option<String>) -> BoxFut<'_, anyhow::Result<u64>> {
        Box::pin(async { Ok(0) })
    }
}

/// 依据配置构造记忆引擎（关闭时返回 [`NoopMemory`]）。
///
/// `llm` 是 `[llm]` 配置：抽取路由未单独配置时复用它（同一 trait，不另写 HTTP 客户端）。
pub fn build_memory(
    cfg: &MemoryConfig,
    llm: &crate::config::LlmConfig,
) -> anyhow::Result<Arc<dyn MemoryProvider>> {
    if !cfg.active() {
        return Ok(Arc::new(NoopMemory));
    }
    Ok(Arc::new(GraphMemory::new(cfg, llm)?))
}

#[cfg(test)]
mod tests;
