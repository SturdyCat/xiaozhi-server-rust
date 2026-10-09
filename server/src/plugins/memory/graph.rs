//! 图记忆实现（本地 SQLite + 词法召回 + 后台抽取）。
//!
//! 数据流（对齐 `docs/soul-and-graph-memory-plan.md` §5.2）：
//!
//! ```text
//! ASR 出文本
//!   ├─ (关键路径) recall(scope, text)  ── 预算 ≤ recall_budget_ms，超时/失败 → 空块
//!   ├─ 组装 instructions → LLM 流式 → 按句 TTS → 逐帧下行（既有流水线不动）
//!   └─ (后台) record(本轮 Q/A)  ── 只落原文（快）
//!              └─ extract_pending()  ── 1 次辅助 LLM 调用 → 摘要 + 倒排索引
//!                        └─ 每 N 轮 maintain()（保留策略 + 隔离重试）
//! ```
//!
//! 每次调用都经 `spawn_blocking` 隔离（SQLite 是同步阻塞 API，直接调用会独占 tokio worker）。

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};

use crate::config::LlmConfig;
use crate::plugins::llm::{build_llm, Llm, LlmEvent};

use super::llm_extract;
use super::store::{self, KeepPolicy, PendingTurn, Store};
use super::{assemble, schema, terms};
use super::{
    BoxFut, CompletedTurn, ExtractOutcome, MaintainOpts, MaintainOutcome, MemoryConfig,
    MemoryProvider, MemoryStats, RecallResult,
};

/// 单次抽取的模型输出上限（字符）：正常只有几十字，超过即视为跑偏。
const MAX_EXTRACT_CHARS: usize = 2000;

/// 图记忆引擎。
pub struct GraphMemory {
    cfg: MemoryConfig,
    store: Arc<Store>,
    /// 抽取用的 LLM（`[memory.extractor]` 覆盖 `[llm]`；同一 trait，不另写 HTTP 客户端）。
    llm: Llm,
    /// 抽取路由描述（**不含密钥**，仅供日志/状态页）。
    llm_desc: String,
    /// 距上次维护经过的轮数（每轮 `record` +1；`maintain` 按间隔自行判断是否该跑）。
    since_maintain: AtomicU32,
}

impl GraphMemory {
    pub fn new(cfg: &MemoryConfig, llm_cfg: &LlmConfig) -> Result<Self> {
        let extract_cfg = cfg.extractor.resolve(llm_cfg);
        let llm_desc = format!("{} · {}", extract_cfg.api_base, extract_cfg.model);
        let mut g = Self::with_llm(cfg, build_llm(&extract_cfg))?;
        g.llm_desc = llm_desc;
        Ok(g)
    }

    /// 用**指定的** LLM 构造（抽取路由的注入点）：便于复用已有客户端，
    /// 也让"抽取 → 落库 → 召回"这条链路可以在单测里用一个假 provider 完整跑通。
    pub fn with_llm(cfg: &MemoryConfig, llm: Llm) -> Result<Self> {
        cfg.validate()?;
        let store = store::open(cfg)?;
        Ok(Self {
            cfg: cfg.clone(),
            store,
            llm,
            llm_desc: String::from("(injected)"),
            since_maintain: AtomicU32::new(0),
        })
    }

    /// 一次辅助 LLM 调用，收集完整文本（抽取要的是完整 JSON，不需要流式下游）。
    async fn extract_once(&self, user_text: &str) -> Result<String> {
        let prompt = llm_extract::turn_prompt();
        let mut collected = String::new();
        let r = self
            .llm
            .chat_stream(
                &[],
                user_text,
                &prompt,
                None,
                Box::new(|event| {
                    if let LlmEvent::Text(delta) = event {
                        if collected.len() < MAX_EXTRACT_CHARS {
                            collected.push_str(&delta);
                        }
                    }
                    true
                }),
            )
            .await;
        match r {
            Ok(out) => Ok(if collected.is_empty() {
                out.text
            } else {
                collected
            }),
            Err(e) => Err(e),
        }
    }

    /// 处理队列队首一轮：`Ok(None)` = 队列空，`Ok(Some(true))` = 抽取成功，
    /// `Ok(Some(false))` = 抽取失败（已按上限决定重试/隔离）。
    async fn extract_one(&self) -> Result<Option<bool>> {
        let store = self.store.clone();
        let pending =
            match tokio::task::spawn_blocking(move || store.pending_turn(schema::now_ms())).await {
                Ok(Ok(Some(p))) => p,
                Ok(Ok(None)) => return Ok(None),
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(anyhow!("查询抽取队列任务异常: {e}")),
            };
        let store = self.store.clone();
        let key = pending.clone();
        let msgs = tokio::task::spawn_blocking(move || store.turn_messages(&key))
            .await
            .map_err(|e| anyhow!("读取原文任务异常: {e}"))??;
        let user_text = msgs
            .iter()
            .find(|m| m.role == "user")
            .map(|m| m.content.clone())
            .unwrap_or_default();
        let assistant_text = msgs
            .iter()
            .find(|m| m.role == "assistant")
            .map(|m| m.content.clone())
            .unwrap_or_default();

        let input = llm_extract::build_user_text(&user_text, &assistant_text);
        let raw = self.extract_once(&input).await;
        match raw {
            Ok(text) => match llm_extract::parse(&text) {
                Ok(ex) => {
                    let store = self.store.clone();
                    let ids = msgs.iter().map(|m| m.id.clone()).collect::<Vec<_>>();
                    let m = store::NewMemory {
                        scope: pending.scope.clone(),
                        session_id: pending.session_id.clone(),
                        turn_index: pending.turn_index,
                        summary: ex.summary.clone(),
                        outcome: ex.outcome.clone(),
                        keywords: ex.keywords.clone(),
                    };
                    let now = schema::now_ms();
                    let key = pending.clone();
                    let written = tokio::task::spawn_blocking(move || -> Result<()> {
                        store.upsert_memory(&m, &ids, now)?;
                        store.mark_turn(&key, "succeeded", None, None, now)
                    })
                    .await
                    .map_err(|e| anyhow!("写入记忆任务异常: {e}"))?;
                    written?;
                    tracing::info!(
                        "记忆抽取成功：scope={} turn={} outcome={} summary={}",
                        pending.scope,
                        pending.turn_index,
                        ex.outcome,
                        ex.summary
                    );
                    Ok(Some(true))
                }
                Err(e) => {
                    self.mark_failed(&pending, &format!("解析抽取结果失败: {e:#}"))
                        .await?;
                    Ok(Some(false))
                }
            },
            Err(e) => {
                self.mark_failed(&pending, &format!("抽取调用失败: {e:#}"))
                    .await?;
                Ok(Some(false))
            }
        }
    }

    /// 记录失败并按上限决定"重试或隔离"。
    async fn mark_failed(&self, pending: &PendingTurn, error: &str) -> Result<()> {
        let store = self.store.clone();
        let key = pending.clone();
        let max_attempts = self.cfg.quarantine_max_attempts;
        let now = schema::now_ms();
        let err = error.to_string();
        let (state, attempts) = tokio::task::spawn_blocking(move || {
            store.mark_turn_failed(&key, &err, max_attempts, now)
        })
        .await
        .map_err(|e| anyhow!("标记抽取失败任务异常: {e}"))??;
        tracing::warn!(
            "记忆抽取失败（第 {attempts} 次，状态={state}，scope={} turn={}）: {error}",
            pending.scope,
            pending.turn_index
        );
        Ok(())
    }

    /// 实际执行维护（跳过间隔判断）。
    async fn maintain_now(&self, dry_run: bool) -> Result<MaintainOutcome> {
        let store = self.store.clone();
        let cfg = self.cfg.clone();
        let released = store.release_quarantine(cfg.quarantine_max_attempts, schema::now_ms())?;
        let keep = KeepPolicy::from_config(&cfg.retention);
        let deleted = store.retention(
            keep,
            cfg.retention.recent_turns,
            cfg.retention.retention_days,
            dry_run || cfg.retention.dry_run,
        )?;
        tracing::info!(
            "记忆维护完成：隔离释放 {released} 条，保留策略处理 {deleted} 条原文（dry_run={}）",
            dry_run || cfg.retention.dry_run
        );
        Ok(MaintainOutcome {
            ran: true,
            released,
            deleted,
            dry_run: dry_run || cfg.retention.dry_run,
        })
    }
}

impl MemoryProvider for GraphMemory {
    fn name(&self) -> &'static str {
        "graph"
    }

    fn recall<'a>(&'a self, scope: Option<&'a str>, query: &'a str) -> BoxFut<'a, RecallResult> {
        Box::pin(async move {
            if !self.cfg.recall_enabled {
                return RecallResult::none();
            }
            let t0 = Instant::now();
            let terms = terms::query_terms(query);
            if terms.is_empty() {
                return RecallResult {
                    elapsed_ms: t0.elapsed().as_millis(),
                    ..RecallResult::none()
                };
            }
            let store = self.store.clone();
            let scope_owned = scope.map(|s| s.to_string());
            let fresh = self.cfg.fresh_turn_count;
            let limit = self.cfg.recall_max_nodes;
            let budget = Duration::from_millis(self.cfg.recall_budget_ms.max(1));
            let task = tokio::task::spawn_blocking(move || {
                store.search(&terms, scope_owned.as_deref(), fresh, limit)
            });
            let hits = match tokio::time::timeout(budget, task).await {
                // 硬预算：超时即"无记忆"继续。⚠️ 后台任务无法取消（SQLite 是同步的），
                // 但它不会再影响本轮 TTFA。
                Err(_) => {
                    tracing::warn!(
                        "记忆召回超预算（{}ms），本轮按无记忆继续",
                        self.cfg.recall_budget_ms
                    );
                    return RecallResult {
                        elapsed_ms: t0.elapsed().as_millis(),
                        error: Some("召回超预算（已按无记忆继续）".into()),
                        ..RecallResult::none()
                    };
                }
                Ok(Err(e)) => {
                    return RecallResult {
                        elapsed_ms: t0.elapsed().as_millis(),
                        error: Some(format!("召回任务异常: {e}")),
                        ..RecallResult::none()
                    }
                }
                Ok(Ok(Err(e))) => {
                    return RecallResult {
                        elapsed_ms: t0.elapsed().as_millis(),
                        error: Some(format!("{e:#}")),
                        ..RecallResult::none()
                    }
                }
                Ok(Ok(Ok(h))) => h,
            };
            let matched = hits.iter().map(|h| h.hits).sum::<i64>() as usize;
            if hits.is_empty() {
                return RecallResult {
                    hits: 0,
                    matched_terms: 0,
                    elapsed_ms: t0.elapsed().as_millis(),
                    ..RecallResult::none()
                };
            }
            let block = assemble::render_within_budget(&hits, self.cfg.recall_max_tokens);
            let block = if block.is_empty() { None } else { Some(block) };
            RecallResult {
                hits: hits.len(),
                matched_terms: matched,
                elapsed_ms: t0.elapsed().as_millis(),
                block,
                error: None,
            }
        })
    }

    fn record<'a>(&'a self, turn: CompletedTurn) -> BoxFut<'a, Result<()>> {
        Box::pin(async move {
            if !self.cfg.active() {
                return Ok(());
            }
            if turn.user_text.trim().is_empty() || turn.assistant_text.trim().is_empty() {
                return Ok(()); // 空轮（静音/失败兜底）没有记忆价值
            }
            self.since_maintain.fetch_add(1, Ordering::Relaxed);
            let store = self.store.clone();
            let now = schema::now_ms();
            let store2 = store.clone();
            tokio::task::spawn_blocking(move || -> Result<()> {
                let idx = store2.next_turn_index(&turn.scope)?;
                store2.insert_turn(
                    &turn.scope,
                    &turn.session_id,
                    idx,
                    &turn.user_text,
                    &turn.assistant_text,
                    now,
                )?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow!("记忆写入任务异常: {e}"))?
        })
    }

    fn extract_pending(&self, limit: usize) -> BoxFut<'_, ExtractOutcome> {
        Box::pin(async move {
            let mut out = ExtractOutcome::default();
            if !self.cfg.active() || !self.cfg.extraction_enabled {
                return out;
            }
            for _ in 0..limit.max(1) {
                match self.extract_one().await {
                    Ok(Some(true)) => {
                        out.attempted += 1;
                        out.succeeded += 1;
                    }
                    Ok(Some(false)) => {
                        out.attempted += 1;
                        out.failed += 1;
                    }
                    Ok(None) => break, // 队列空
                    Err(e) => {
                        // 队列查询/落库失败（非模型失败）：记日志但不反复重试
                        tracing::warn!("记忆抽取批次中断: {e:#}");
                        break;
                    }
                }
            }
            out
        })
    }

    fn maintain(&self, opts: MaintainOpts) -> BoxFut<'_, Result<MaintainOutcome>> {
        Box::pin(async move {
            if !self.cfg.active() {
                return Ok(MaintainOutcome::skipped());
            }
            let interval = self.cfg.maintenance_interval;
            if !opts.force {
                let done = self.since_maintain.load(Ordering::Relaxed);
                if interval == 0 || done < interval {
                    return Ok(MaintainOutcome::skipped());
                }
            }
            self.since_maintain.store(0, Ordering::Relaxed);
            self.maintain_now(opts.dry_run).await
        })
    }

    fn stats(&self) -> MemoryStats {
        let base = MemoryStats {
            engine: "graph",
            active: self.cfg.active(),
            db_path: self.store.path().to_string(),
            db_bytes: self.store.db_bytes(),
            lexical_only: true,
            extractor: self.llm_desc.clone(),
            ..MemoryStats::default()
        };
        match self.store.counts() {
            Ok(c) => MemoryStats {
                messages: c.messages,
                memories: c.memories,
                pending: c.pending,
                quarantined: c.quarantined,
                scopes: self.store.scopes().unwrap_or_default(),
                ..base
            },
            Err(e) => MemoryStats {
                error: Some(format!("{e:#}")),
                ..base
            },
        }
    }

    fn clear(&self, scope: Option<String>) -> BoxFut<'_, Result<u64>> {
        Box::pin(async move {
            let store = self.store.clone();
            tokio::task::spawn_blocking(move || store.clear(scope.as_deref()))
                .await
                .map_err(|e| anyhow!("清空记忆任务异常: {e}"))?
                .context("清空记忆失败")
        })
    }
}

#[cfg(test)]
mod tests;
