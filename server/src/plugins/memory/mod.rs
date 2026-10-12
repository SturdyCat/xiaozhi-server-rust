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

pub mod assemble;
pub mod config;
pub mod types;
mod graph;
mod llm_extract;
mod schema;
mod store;
mod terms;
pub use graph::GraphMemory;
// 作为插件对外 API 面保留（bin crate 内暂无直接引用者；抽取链路的单测经 `super::*` 使用）
#[allow(unused_imports)]
pub use llm_extract::{Extraction, EXTRACT_INSTRUCTIONS, OUTCOMES};

// 配置与数据类型拆分到子文件，此处 `pub use` 再导出，
// 保证既有 `crate::plugins::memory::Xxx` 与子模块 `super::Xxx` 引用路径不变。
pub use config::{ExtractorConfig, MemoryBackend, MemoryConfig, RetentionConfig};
pub use types::{
    CompletedTurn, ExtractOutcome, MaintainOpts, MaintainOutcome, MemoryStats, RecallResult,
};

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
