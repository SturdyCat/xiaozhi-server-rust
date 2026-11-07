//! `MemoryProvider` trait 与 `NoopMemory`：记忆能力的**唯一接口源**。
//!
//! 本文件由 `plugins/memory/mod.rs` 拆出（纯移动，不改任何签名与语义），
//! 目的是让 legacy 原生实现（`mod.rs`）与 pure-base 空壳（`pure_base.rs`）
//! 经 `#[path]` **单源共享**同一份 trait 定义——接口一行不改的承诺由物理
//! 单文件保证，而不是靠两处拷贝小心同步。
//!
//! 依赖约定：只能引用 `super::types`（纯数据），不得引用任何后端实现。

use std::future::Future;
use std::pin::Pin;

use super::types::{
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

    /// 触发一次后台索引重建（P1-9，仅 rag 后端实现）。
    ///
    /// 返回 `false` = 未触发（后端不支持，或已在构建中防重入）。
    /// 管理台「重建索引」按钮经此入口；结果查 `/api/memory/status` 的 `rag` 段。
    fn reindex(&self) -> bool {
        false
    }
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
