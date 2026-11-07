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

use std::sync::Arc;

pub mod assemble;
pub mod config;
mod graph;
mod llm_extract;
/// trait 与 NoopMemory 单源文件（pure-base 空壳经 `#[path]` 共享同一份定义）。
#[path = "provider.rs"]
mod provider;
mod rag;
mod schema;
mod store;
mod store_only;
mod terms;
pub mod types;
pub use graph::GraphMemory;
pub use provider::{BoxFut, MemoryProvider, NoopMemory};
pub use rag::RagMemory;
pub use store_only::StoreOnlyMemory;
// 作为插件对外 API 面保留（bin crate 内暂无直接引用者；抽取链路的单测经 `super::*` 使用）
#[allow(unused_imports)]
pub use llm_extract::{Extraction, EXTRACT_INSTRUCTIONS, OUTCOMES};

// 配置与数据类型拆分到子文件，此处 `pub use` 再导出，
// 保证既有 `crate::plugins::memory::Xxx` 与子模块 `super::Xxx` 引用路径不变。
// RagConfig：类型经 `MemoryConfig.rag` 字段暴露；re-export 仅为 rag 单测 `super::*` 可达。
#[allow(unused_imports)]
pub use config::{MemoryBackend, MemoryConfig, RagConfig, RetentionConfig};
// 抽取路由配置：类型经 `MemoryConfig.extractor` 字段暴露；显式 re-export 仅为单测 `super::*` 可达。
#[allow(unused_imports)]
pub use config::ExtractorConfig;
pub use types::{
    CompletedTurn, ExtractOutcome, MaintainOpts, MaintainOutcome, MemoryStats, RecallResult,
};

/// 依据配置构造记忆引擎（关闭时返回 [`NoopMemory`]）。
///
/// `llm` 是 `[llm]` 配置：抽取路由未单独配置时复用它（同一 trait，不另写 HTTP 客户端）。
///
/// ⚠️ **本函数是后端分发的唯一收口点**（P1-7 多后端注册表）：`PluginHost::boot` 与
/// `Engines::refresh_memory` 热切换都走这里——新增后端只改这一个 match + 注册表描述符，
/// engine.rs 永远不动。store 分支**不触碰 llm**（零辅助 LLM 成本的结构保证，见
/// [`StoreOnlyMemory`] 模块注释）。
pub fn build_memory(
    cfg: &MemoryConfig,
    llm: &crate::config::LlmConfig,
) -> anyhow::Result<Arc<dyn MemoryProvider>> {
    if !cfg.active() {
        return Ok(Arc::new(NoopMemory));
    }
    match cfg.engine {
        MemoryBackend::Graph => Ok(Arc::new(GraphMemory::new(cfg, llm)?)),
        MemoryBackend::Store => Ok(Arc::new(StoreOnlyMemory::new(cfg)?)),
        // rag 与 store 同为零 LLM 后端：不触碰 llm（P1-9）
        MemoryBackend::Rag => Ok(Arc::new(RagMemory::new(cfg)?)),
        // 不可达（active() 已挡 none），match 穷尽需要
        MemoryBackend::None => Ok(Arc::new(NoopMemory)),
    }
}

#[cfg(test)]
mod tests;
