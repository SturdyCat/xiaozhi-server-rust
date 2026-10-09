//! 记忆库的**维护类**操作：隔离重试释放 + 原文保留策略。
//!
//! 从 `store.rs` 拆出（见 `AGENTS.md` §5.10：单文件接近 600 行即按职责拆分）。
//! 这些方法写在同一 SQLite 连接上，只是语义上属于"低频维护"，与读写主路径分开更清楚。

use anyhow::{Context, Result};

use super::schema;
use super::Store;

impl Store {
    /// 把到期的隔离轮次放回队列（有限重试；超过上限的永久隔离，等人工 `maintain`）。
    pub fn release_quarantine(&self, max_attempts: u32, now: i64) -> Result<u64> {
        let n = self
            .conn()
            .execute(
                "UPDATE gm_messages
                 SET extraction_state = 'pending', next_retry_at = NULL, updated_at = ?1
                 WHERE extraction_state = 'quarantined'
                   AND extraction_attempts < ?2
                   AND (next_retry_at IS NULL OR next_retry_at <= ?1)",
                rusqlite::params![now, max_attempts],
            )
            .context("释放隔离轮次失败")?;
        Ok(n as u64)
    }

    /// 保留策略：删除**未被轮次记忆引用**的旧原文。
    ///
    /// ⚠️ 候选查询必须带 `NOT IN (SELECT message_id FROM gm_turn_memory_sources)`——
    /// 这是上游的已知缺陷（它只查 legacy 表，会把轮次记忆引用的原文删掉，
    /// 于是唯一的"证据层"直接变空，见 `docs/soul-and-graph-memory-plan.md` §2.4 缺陷 1）。
    pub fn retention(
        &self,
        keep: KeepPolicy,
        recent_turns: u32,
        retention_days: u32,
        dry_run: bool,
    ) -> Result<u64> {
        if keep == KeepPolicy::All
            || (keep == KeepPolicy::Recent && recent_turns == 0 && retention_days == 0)
        {
            return Ok(0);
        }
        let now = schema::now_ms();
        let mut total = 0u64;
        for scope in self.message_scopes()? {
            let cutoff = if keep == KeepPolicy::Recent && recent_turns > 0 {
                let max: Option<i64> = self
                    .conn()
                    .query_row(
                        "SELECT MAX(turn_index) FROM gm_messages WHERE scope = ?1",
                        rusqlite::params![scope],
                        |r| r.get(0),
                    )
                    .context("读取最大轮次失败")?;
                max.map(|m| m - recent_turns as i64)
            } else {
                None
            };
            let age_cutoff = if retention_days > 0 {
                Some(now - retention_days as i64 * 86_400_000)
            } else {
                None
            };
            let mut sql = String::from(
                "FROM gm_messages WHERE id NOT IN (SELECT message_id FROM gm_turn_memory_sources) AND scope = ?1",
            );
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(scope.clone())];
            if let Some(c) = cutoff {
                sql.push_str(" AND turn_index <= ?");
                params.push(Box::new(c));
            }
            if let Some(a) = age_cutoff {
                sql.push_str(" AND created_at < ?");
                params.push(Box::new(a));
            }
            let count_sql = format!("SELECT COUNT(*) {sql}");
            let conn = self.conn();
            if dry_run {
                let n: i64 = conn
                    .query_row(&count_sql, rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())), |r| r.get(0))
                    .context("保留策略演练失败")?;
                total += n as u64;
            } else {
                let del_sql = format!("DELETE {sql}");
                let n = conn
                    .execute(&del_sql, rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())))
                    .context("保留策略删除失败")?;
                total += n as u64;
            }
        }
        Ok(total)
    }
}

/// 保留策略（`[memory.retention].keep`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepPolicy {
    /// 全部保留（默认；上游默认也是它，且它掩盖了上面的缺陷）。
    All,
    /// 只保留被轮次记忆引用的原文（未引用的原文 → 可删）。
    Referenced,
    /// 最近 `recent_turns` 轮之外的**未引用**原文 → 可删。
    Recent,
}

impl KeepPolicy {
    pub fn from_config(cfg: &crate::plugins::memory::RetentionConfig) -> Self {
        match cfg.keep.trim() {
            "referenced" => KeepPolicy::Referenced,
            "recent" => KeepPolicy::Recent,
            _ => KeepPolicy::All,
        }
    }
}
