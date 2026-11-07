//! 纯存储后端（P1-7：`[memory].engine = "store"`）：只把对话原文落 SQLite，
//! 不抽取（零辅助 LLM 调用）、`recall` 恒空——语义定位是"**先积累，再决定**"。
//!
//! ## 与 graph 的 `extraction_enabled` 开关分层
//!
//! - **backend 维度**（`[memory].engine`）选引擎：graph = 抽取 + 召回；store = 都不做；
//! - **开关维度**（`[memory].extraction_enabled`）只管 graph 内部行为（要不要摘要）；
//! 两者并存不混淆：选了 store 再开 extraction_enabled 不会让 store 抽取。
//!
//! ## 零成本的结构保证（PRD P1-7 验收 1）
//!
//! - 构造（[`StoreOnlyMemory::new`]）**不触碰 LLM**：不调 `build_llm`、不读
//!   `[memory.extractor]`——"零辅助 LLM 成本"不靠约定，靠类型系统；
//! - `record` 只写 `gm_messages`（`extraction_state='stored'`），代码里没有引用
//!   `gm_turn_memories` / `gm_summary_terms` 的路径 → 摘要/倒排零写入；
//! - `extraction_state='stored'` 的轮次**永不进入** graph 抽取队列
//!   （`store.rs::pending_turn` 的 `WHERE extraction_state='pending'` 天然看不见），
//!   因此从 store 切回 graph 不会突发 N 次辅助 LLM 调用（成本惊吓防护）。
//!
//! ## 与 graph 共用同一 `db_path`
//!
//! schema 迁移幂等（`schema::open`），store 只写 `gm_messages`（'stored' 行），
//! graph 表零触碰——graph ⇄ store 来回切换数据不丢（PRD 验收 3）。
//! "积累后批量重建图记忆"（把 'stored' 回填为 'pending'）是 P2 的显式管理动作，本轮不做。

use std::sync::Arc;

use anyhow::Result;

use super::store::{self, KeepPolicy, Store};
use super::{
    schema, BoxFut, CompletedTurn, ExtractOutcome, MaintainOpts, MaintainOutcome, MemoryConfig,
    MemoryProvider, MemoryStats, RecallResult,
};

/// SQLite 是同步阻塞 API，统一经 `spawn_blocking` 隔离（与 graph 同纪律）。
async fn run_blocking<T, F>(label: &str, f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| anyhow::anyhow!("{label}任务异常: {e}"))?
}

/// 纯存储引擎：`record` 落原文，其余读路径全部为空/无操作。
pub struct StoreOnlyMemory {
    cfg: MemoryConfig,
    store: Arc<Store>,
}

impl StoreOnlyMemory {
    /// ⚠️ 构造**不触碰 LLM**：不调 `build_llm`、不读 `[memory.extractor]`——
    /// 这是"零辅助 LLM 成本"的结构保证（PRD 验收 1），别在这里加任何上游客户端。
    pub fn new(cfg: &MemoryConfig) -> Result<Self> {
        cfg.validate()?;
        let store = store::open(cfg)?;
        Ok(Self {
            cfg: cfg.clone(),
            store,
        })
    }
}

impl MemoryProvider for StoreOnlyMemory {
    fn name(&self) -> &'static str {
        "store"
    }

    /// 恒空（绝不 `Err`，语义对齐 trait 约定）：store 的定位就是"不参与召回"，
    /// 需要召回时用户应选 graph（或 P1-9 的 rag）。
    fn recall<'a>(&'a self, _scope: Option<&'a str>, _query: &'a str) -> BoxFut<'a, RecallResult> {
        Box::pin(async { RecallResult::none() })
    }

    /// 写入一轮原文（`extraction_state='stored'`，不进抽取队列）。
    fn record<'a>(&'a self, turn: CompletedTurn) -> BoxFut<'a, Result<()>> {
        Box::pin(async move {
            if !self.cfg.active() {
                return Ok(());
            }
            if turn.user_text.trim().is_empty() || turn.assistant_text.trim().is_empty() {
                return Ok(()); // 空轮（静音/失败兜底）没有留存价值
            }
            let store = self.store.clone();
            let now = schema::now_ms();
            run_blocking("记忆写入(store)", move || -> Result<()> {
                let idx = store.next_turn_index(&turn.scope)?;
                store.insert_turn_state(
                    &turn.scope,
                    &turn.session_id,
                    idx,
                    &turn.user_text,
                    &turn.assistant_text,
                    "stored",
                    now,
                )?;
                Ok(())
            })
            .await
        })
    }

    /// 恒无操作（零 LLM 调用）：返回默认值（`attempted == 0`，会话侧不进日志分支）。
    fn extract_pending(&self, _limit: usize) -> BoxFut<'_, ExtractOutcome> {
        Box::pin(async { ExtractOutcome::default() })
    }

    /// 低频维护：保留策略对**原文**生效（复用 graph 的 retention 代码，零新逻辑）。
    ///
    /// 默认开（已拍板）：用户可用 `[memory.retention]` 显式控制——纯存储不等于
    /// "存了永不删"，库体积无界增长同样是一种事故。隔离释放（release_quarantine）
    /// 对 store 无意义（没有隔离行），恒 0。
    fn maintain(&self, opts: MaintainOpts) -> BoxFut<'_, Result<MaintainOutcome>> {
        Box::pin(async move {
            if !self.cfg.active() {
                return Ok(MaintainOutcome::skipped());
            }
            let interval = self.cfg.maintenance_interval;
            if !opts.force {
                // store 无 since_maintain 计数器（graph 按轮数判断）：用轮次数近似——
                // 每 maintenance_interval 轮跑一次保留策略，语义与 graph 对齐。
                let turns = self.store.turn_count().unwrap_or(0);
                if interval == 0 || turns % interval as i64 != 0 {
                    return Ok(MaintainOutcome::skipped());
                }
            }
            let store = self.store.clone();
            let cfg = self.cfg.clone();
            let dry = opts.dry_run || cfg.retention.dry_run;
            let deleted = run_blocking("记忆维护(store)", move || {
                let keep = KeepPolicy::from_config(&cfg.retention);
                store.retention(
                    keep,
                    cfg.retention.recent_turns,
                    cfg.retention.retention_days,
                    dry,
                )
            })
            .await?;
            tracing::info!(
                "store 记忆维护完成：保留策略处理 {deleted} 条原文（dry_run={}）",
                dry
            );
            Ok(MaintainOutcome {
                ran: true,
                released: 0, // store 无隔离语义
                deleted,
                dry_run: dry,
            })
        })
    }

    /// 状态快照：engine="store"、turns/stored 为核心计数，scopes 按**原文**口径。
    fn stats(&self) -> MemoryStats {
        let base = MemoryStats {
            engine: "store",
            active: self.cfg.active(),
            db_path: self.store.path().to_string(),
            db_bytes: self.store.db_bytes(),
            // store 恒为"无召回"模式——但这是设计定位不是降级，UI 不据此报降级原因
            lexical_only: true,
            extractor: String::new(), // 不构建抽取路由（构造不触碰 LLM）
            ..MemoryStats::default()
        };
        match self.store.counts() {
            Ok(c) => MemoryStats {
                turns: self.store.turn_count().unwrap_or_default(),
                messages: c.messages,
                memories: c.memories,
                pending: c.pending,
                quarantined: c.quarantined,
                stored: c.stored,
                scopes: self.store.message_scope_counts().unwrap_or_default(),
                ..base
            },
            Err(e) => MemoryStats {
                error: Some(format!("{e:#}")),
                ..base
            },
        }
    }

    /// 清空（与 graph 同款：删 `gm_messages`[+`gm_turn_memories`]——store 数据库里
    /// 后者恒空，删了也是零行）。
    fn clear(&self, scope: Option<String>) -> BoxFut<'_, Result<u64>> {
        Box::pin(async move {
            let store = self.store.clone();
            run_blocking("清空记忆(store)", move || store.clear(scope.as_deref())).await
        })
    }
}

#[cfg(test)]
mod tests;
