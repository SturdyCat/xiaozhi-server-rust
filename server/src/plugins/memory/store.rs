//! 记忆库读写（SQLite）：原文事实源、轮次记忆、倒排索引、保留策略。
//!
//! 并发模型：**单连接 + `Mutex` 串行化**（对齐 `docs/soul-and-graph-memory-plan.md` §5.5
//! 的"单写者"建议）。SQLite 的写锁是全局的，连接池多写只会把锁竞争搬到 Rust 侧并更容易
//! 触发 `SQLITE_BUSY`；单连接 + `busy_timeout` 既简单又能与 DSH 进程共用同一 db 文件（P3）。
//!
//! ⚠️ 所有方法都是**同步阻塞**的：调用方（async 会话）必须包 `spawn_blocking`
//! （`engine.rs` 模块注释的线程预算铁律）。

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};

use super::schema;
use super::terms;
use super::MemoryConfig;

mod maintenance;
mod rag_index;
pub use maintenance::KeepPolicy;
// RAG 索引扩展（P1-9）：类型供 memory/rag.rs 使用（实现在同目录子文件，访问私有 conn()）
pub(super) use rag_index::{BuildSummary, RagSearchHit};

/// 打开记忆库（含父目录创建与迁移）。
pub fn open(cfg: &MemoryConfig) -> Result<Arc<Store>> {
    let path = cfg.db_path.trim();
    if path.is_empty() {
        anyhow::bail!("[memory].db_path 为空：请填一个可写路径（默认 /data/data.db）。");
    }
    if path != ":memory:" {
        if let Some(dir) = std::path::Path::new(path).parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("创建记忆库目录 {} 失败", dir.display()))?;
            }
        }
    }
    let conn = schema::open(path, cfg.db_busy_timeout_ms)?;
    Ok(Arc::new(Store {
        conn: Mutex::new(conn),
        path: path.to_string(),
    }))
}

/// 一轮对话的原文（user + assistant 两条）。
#[derive(Debug, Clone)]
pub struct TurnMessage {
    pub id: String,
    pub role: String,
    pub content: String,
}

/// 待抽取的轮次（抽取队列的队首）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTurn {
    pub scope: String,
    pub session_id: String,
    pub turn_index: i64,
}

/// 一条待写入的轮次记忆。
#[derive(Debug, Clone)]
pub struct NewMemory {
    pub scope: String,
    pub session_id: String,
    pub turn_index: i64,
    pub summary: String,
    pub outcome: String,
    pub keywords: Vec<String>,
}

/// 召回命中（摘要 + 它的原文证据）。
#[derive(Debug, Clone)]
pub struct Hit {
    pub memory_id: String,
    pub summary: String,
    pub outcome: String,
    pub turn_index: i64,
    pub hits: i64,
    /// `(role, content)`：**原始 Q/A**（证据层；导航层不得成为事实载荷）。
    pub sources: Vec<(String, String)>,
}

/// 库内计数（状态页用）。
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq)]
pub struct Counts {
    pub messages: i64,
    pub memories: i64,
    pub pending: i64,
    pub quarantined: i64,
    /// `extraction_state='stored'` 的消息条数（P1-7：store 后端积累的原文；
    /// graph 恒 0——切换到 graph 后端后该计数不再增长，但历史存量仍可见）。
    pub stored: i64,
}

/// SQLite 记忆库。
pub struct Store {
    conn: Mutex<Connection>,
    path: String,
}

impl Store {
    pub fn path(&self) -> &str {
        &self.path
    }

    /// 库文件字节数（`:memory:` 或统计失败返回 0）。
    pub fn db_bytes(&self) -> u64 {
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }

    /// 取锁（中毒后继续用：单条 SQL 失败不该让整个记忆能力永久不可用）。
    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 该 scope 的下一个轮次号（跨会话单调，保证"第 N 轮"语义稳定）。
    pub fn next_turn_index(&self, scope: &str) -> Result<i64> {
        let n: i64 = self
            .conn()
            .query_row(
                "SELECT COALESCE(MAX(turn_index), 0) + 1 FROM gm_messages WHERE scope = ?1",
                rusqlite::params![scope],
                |r| r.get(0),
            )
            .context("读取 turn_index 失败")?;
        Ok(n)
    }

    /// 写入一轮原文（user + assistant）并置为待抽取。
    ///
    /// graph 后端的既有入口（行为零变化）：委托 [`Self::insert_turn_state`]，state='pending'。
    pub fn insert_turn(
        &self,
        scope: &str,
        session_id: &str,
        turn_index: i64,
        user_text: &str,
        assistant_text: &str,
        now: i64,
    ) -> Result<Vec<String>> {
        self.insert_turn_state(
            scope,
            session_id,
            turn_index,
            user_text,
            assistant_text,
            "pending",
            now,
        )
    }

    /// 写入一轮原文并指定抽取状态（P1-7 参数化：`'pending'` = graph 队列，`'stored'` = 纯存储）。
    ///
    /// ⚠️ `'stored'` 的语义纪律：store 后端写入、**永不进入** graph 抽取队列
    /// （`pending_turn` 的 `WHERE extraction_state='pending'` 天然看不见它们）。
    /// 把 'stored' 批量改回 'pending' 属于 P2 的显式管理动作（批量重建图记忆），
    /// 任何新代码不得自动回填——否则用户从 store 切回 graph 会突发 N 次辅助 LLM 调用。
    pub fn insert_turn_state(
        &self,
        scope: &str,
        session_id: &str,
        turn_index: i64,
        user_text: &str,
        assistant_text: &str,
        state: &str,
        now: i64,
    ) -> Result<Vec<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction().context("开启事务失败")?;
        let mut ids = Vec::with_capacity(2);
        for (role, content) in [("user", user_text), ("assistant", assistant_text)] {
            let id = schema::stable_id(
                "msg",
                &[scope, session_id, &turn_index.to_string(), role, content],
            );
            tx.execute(
                "INSERT OR IGNORE INTO gm_messages
                   (id, scope, session_id, turn_index, role, content, created_at,
                    extraction_state, extraction_attempts, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?7)",
                rusqlite::params![id, scope, session_id, turn_index, role, content, now, state],
            )
            .context("写入原文失败")?;
            ids.push(id);
        }
        tx.commit().context("提交原文事务失败")?;
        Ok(ids)
    }

    /// 抽取队列队首：最早的、到期的 pending 轮次。
    pub fn pending_turn(&self, now: i64) -> Result<Option<PendingTurn>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT scope, session_id, turn_index FROM gm_messages
                 WHERE extraction_state = 'pending'
                   AND (next_retry_at IS NULL OR next_retry_at <= ?1)
                 GROUP BY scope, session_id, turn_index
                 ORDER BY MIN(created_at) ASC
                 LIMIT 1",
            )
            .context("准备抽取队列查询失败")?;
        let row = stmt
            .query_row(rusqlite::params![now], |r| {
                Ok(PendingTurn {
                    scope: r.get(0)?,
                    session_id: r.get(1)?,
                    turn_index: r.get(2)?,
                })
            })
            .optional()
            .context("查询抽取队列失败")?;
        Ok(row)
    }

    /// 某轮的全部原文（按 user → assistant 语义顺序）。
    pub fn turn_messages(&self, t: &PendingTurn) -> Result<Vec<TurnMessage>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, role, content FROM gm_messages
                 WHERE scope = ?1 AND session_id = ?2 AND turn_index = ?3
                 ORDER BY CASE role WHEN 'user' THEN 0 ELSE 1 END, id",
            )
            .context("准备原文查询失败")?;
        let rows = stmt
            .query_map(
                rusqlite::params![t.scope, t.session_id, t.turn_index],
                |r| {
                    Ok(TurnMessage {
                        id: r.get(0)?,
                        role: r.get(1)?,
                        content: r.get(2)?,
                    })
                },
            )
            .context("查询原文失败")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("读取原文失败")
    }

    /// 更新某轮抽取状态（同时累加尝试次数）。
    pub fn mark_turn(
        &self,
        t: &PendingTurn,
        state: &str,
        error: Option<&str>,
        next_retry_at: Option<i64>,
        now: i64,
    ) -> Result<()> {
        self.conn()
            .execute(
                "UPDATE gm_messages
                 SET extraction_state = ?4,
                     extraction_attempts = extraction_attempts + 1,
                     extraction_error = ?5,
                     next_retry_at = ?6,
                     updated_at = ?7
                 WHERE scope = ?1 AND session_id = ?2 AND turn_index = ?3",
                rusqlite::params![
                    t.scope,
                    t.session_id,
                    t.turn_index,
                    state,
                    error,
                    next_retry_at,
                    now
                ],
            )
            .context("更新抽取状态失败")?;
        Ok(())
    }

    /// 记录一次抽取失败：累加尝试次数，未超上限则安排**指数退避**重试，超限即隔离。
    ///
    /// 返回 `(新状态, 累计尝试次数)`。
    pub fn mark_turn_failed(
        &self,
        t: &PendingTurn,
        error: &str,
        max_attempts: u32,
        now: i64,
    ) -> Result<(String, u32)> {
        let attempts: i64 = self
            .conn()
            .query_row(
                "SELECT COALESCE(MAX(extraction_attempts), 0) FROM gm_messages
                 WHERE scope = ?1 AND session_id = ?2 AND turn_index = ?3",
                rusqlite::params![t.scope, t.session_id, t.turn_index],
                |r| r.get(0),
            )
            .context("读取抽取尝试次数失败")?;
        let next = attempts as u32 + 1;
        let quarantined = next >= max_attempts.max(1);
        let state = if quarantined {
            "quarantined"
        } else {
            "pending"
        };
        // 退避 30s → 60s → 120s（上限 10 分钟）：避免对"上游挂了"的轮次持续打点
        let backoff_ms = 30_000i64.saturating_mul(1i64 << next.min(5).saturating_sub(1));
        let retry_at = if quarantined {
            None
        } else {
            Some(now + backoff_ms.min(600_000))
        };
        self.mark_turn(t, state, Some(error), retry_at, now)?;
        Ok((state.to_string(), next))
    }

    /// 写入轮次记忆：摘要 + 倒排词项 + 原文溯源（一个事务）。
    ///
    /// ID 由 `(scope, session_id, turn_index)` 决定 → 重复抽取幂等（`INSERT OR REPLACE`）。
    pub fn upsert_memory(&self, m: &NewMemory, message_ids: &[String], now: i64) -> Result<String> {
        let id = schema::stable_id("tm", &[&m.scope, &m.session_id, &m.turn_index.to_string()]);
        let mut conn = self.conn();
        let tx = conn.transaction().context("开启事务失败")?;
        tx.execute(
            "INSERT OR REPLACE INTO gm_turn_memories
               (id, scope, session_id, turn_index, summary, outcome, keywords, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                id,
                m.scope,
                m.session_id,
                m.turn_index,
                m.summary,
                m.outcome,
                serde_json::to_string(&m.keywords).unwrap_or_else(|_| "[]".into()),
                now
            ],
        )
        .context("写入轮次记忆失败")?;
        // 重建索引与溯源（幂等：先删后插）
        tx.execute(
            "DELETE FROM gm_summary_terms WHERE memory_id = ?1",
            rusqlite::params![id],
        )
        .context("清理旧词项失败")?;
        tx.execute(
            "DELETE FROM gm_turn_memory_sources WHERE memory_id = ?1",
            rusqlite::params![id],
        )
        .context("清理旧溯源失败")?;
        let mut all_terms = terms::index_terms(&m.summary);
        for k in &m.keywords {
            all_terms.extend(terms::index_terms(k));
        }
        all_terms.sort();
        all_terms.dedup();
        for term in &all_terms {
            tx.execute(
                "INSERT OR IGNORE INTO gm_summary_terms (term, memory_id, scope) VALUES (?1, ?2, ?3)",
                rusqlite::params![term, id, m.scope],
            )
            .context("写入词项失败")?;
        }
        for mid in message_ids {
            tx.execute(
                "INSERT OR IGNORE INTO gm_turn_memory_sources (memory_id, message_id) VALUES (?1, ?2)",
                rusqlite::params![id, mid],
            )
            .context("写入溯源失败")?;
        }
        tx.commit().context("提交记忆事务失败")?;
        Ok(id)
    }

    /// 词法召回：按"命中的不同词项数"排序（倒排索引），并排除**最近若干轮**
    /// （那些原文本来就在近况窗口里，重复注入只会白烧 token）。
    pub fn search(
        &self,
        query_terms: &[String],
        scope: Option<&str>,
        fresh_turn_count: u32,
        limit: usize,
    ) -> Result<Vec<Hit>> {
        if query_terms.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let cutoff = match scope {
            Some(s) => {
                let max: Option<i64> = self
                    .conn()
                    .query_row(
                        "SELECT MAX(turn_index) FROM gm_turn_memories WHERE scope = ?1",
                        rusqlite::params![s],
                        |r| r.get(0),
                    )
                    .context("读取最大轮次失败")?;
                max.map(|m| m - fresh_turn_count as i64)
            }
            None => None,
        };
        let mut sql = String::from(
            "SELECT t.memory_id, m.summary, m.outcome, m.turn_index,
                    COUNT(DISTINCT t.term) AS hits
             FROM gm_summary_terms t
             JOIN gm_turn_memories m ON m.id = t.memory_id
             WHERE t.term IN (",
        );
        let placeholders = vec!["?"; query_terms.len()].join(",");
        sql.push_str(&placeholders);
        sql.push(')');
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = query_terms
            .iter()
            .map(|t| Box::new(t.clone()) as Box<dyn rusqlite::ToSql>)
            .collect();
        if let Some(s) = scope {
            sql.push_str(" AND t.scope = ?");
            params.push(Box::new(s.to_string()));
        }
        if let Some(c) = cutoff {
            sql.push_str(" AND m.turn_index <= ?");
            params.push(Box::new(c));
        }
        sql.push_str(" GROUP BY t.memory_id ORDER BY hits DESC, m.turn_index DESC LIMIT ?");
        params.push(Box::new(limit as i64));

        let conn = self.conn();
        let mut stmt = conn.prepare(&sql).context("准备召回查询失败")?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .context("执行召回查询失败")?;
        // m.id = t.memory_id 是主键等值连接：同一 memory_id 下 summary/outcome/turn_index
        // 在所有聚合行中取值确定（SQLite 允许聚合查询中的裸列）。
        let ranked: Vec<(String, String, String, i64, i64)> = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("读取召回结果失败")?;
        drop(stmt);

        let mut hits = Vec::with_capacity(ranked.len());
        for (memory_id, summary, outcome, turn_index, count) in ranked {
            let mut stmt = conn
                .prepare(
                    "SELECT m.role, m.content FROM gm_messages m
                     JOIN gm_turn_memory_sources s ON s.message_id = m.id
                     WHERE s.memory_id = ?1
                     ORDER BY m.turn_index, CASE m.role WHEN 'user' THEN 0 ELSE 1 END",
                )
                .context("准备证据查询失败")?;
            let sources = stmt
                .query_map(rusqlite::params![memory_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .context("查询证据失败")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("读取证据失败")?;
            hits.push(Hit {
                memory_id,
                summary,
                outcome,
                turn_index,
                hits: count,
                sources,
            });
        }
        Ok(hits)
    }

    /// 计数快照。
    pub fn counts(&self) -> Result<Counts> {
        let conn = self.conn();
        let one = |sql: &str| -> Result<i64> {
            conn.query_row(sql, [], |r| r.get(0))
                .context("统计查询失败")
        };
        Ok(Counts {
            messages: one("SELECT COUNT(*) FROM gm_messages")?,
            memories: one("SELECT COUNT(*) FROM gm_turn_memories")?,
            pending: one("SELECT COUNT(*) FROM gm_messages WHERE extraction_state = 'pending'")?,
            quarantined: one(
                "SELECT COUNT(*) FROM gm_messages WHERE extraction_state = 'quarantined'",
            )?,
            stored: one("SELECT COUNT(*) FROM gm_messages WHERE extraction_state = 'stored'")?,
        })
    }

    /// 累计轮次数（`COUNT(DISTINCT scope, session_id, turn_index)`，P1-7）。
    ///
    /// 这是 store 后端状态卡的**核心计数**（"先积累"积累了多少）；graph 一并填充
    /// （`MemoryStats.turns`）——同一口径下两后端可比。每轮 2 条消息，此处按轮去重。
    pub fn turn_count(&self) -> Result<i64> {
        self.conn()
            .query_row(
                // ⚠️ SQLite 的 COUNT 只收单参数（`COUNT(DISTINCT a,b,c)` 是 MySQL 语法），
                // 多列去重计数必须用 DISTINCT 子查询。
                "SELECT COUNT(*) FROM (SELECT DISTINCT scope, session_id, turn_index FROM gm_messages)",
                [],
                |r| r.get(0),
            )
            .context("统计轮次数失败")
    }

    /// 各 scope 的**原文轮次数**（P1-7：store 的 stats 按**原文**列 scope；
    /// graph 的 [`Self::scopes`] 按**记忆**列——两者口径不同，分开命名防混用）。
    pub fn message_scope_counts(&self) -> Result<Vec<(String, i64)>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                // ⚠️ 同 turn_count：SQLite 多列去重计数用 DISTINCT 子查询。
                "SELECT scope, COUNT(*) FROM (SELECT DISTINCT scope, session_id, turn_index FROM gm_messages)
                 GROUP BY scope ORDER BY COUNT(*) DESC",
            )
            .context("准备原文 scope 查询失败")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .context("查询原文 scope 失败")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("读取原文 scope 失败")
    }

    /// 各 scope 的**原文**条数（保留策略按它迭代：库里可能只有原文、还没有记忆，
    /// 若按 `gm_turn_memories` 的 scope 迭代就会出现"有原文却永远清不掉"）。
    pub fn message_scopes(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT DISTINCT scope FROM gm_messages ORDER BY scope")
            .context("准备 scope 查询失败")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .context("查询 scope 失败")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("读取 scope 失败")
    }

    /// 各 scope 的记忆条数（状态页展示"哪台设备有记忆"）。
    pub fn scopes(&self) -> Result<Vec<(String, i64)>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT scope, COUNT(*) FROM gm_turn_memories GROUP BY scope ORDER BY COUNT(*) DESC",
            )
            .context("准备 scope 查询失败")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .context("查询 scope 失败")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("读取 scope 失败")
    }

    /// 清空记忆（`scope = None` 表示全部）。返回删除的**轮次记忆**条数。
    ///
    /// 先删原文（溯源行随之级联删除），再删记忆（词项随之级联删除）。
    pub fn clear(&self, scope: Option<&str>) -> Result<u64> {
        let mut conn = self.conn();
        let tx = conn.transaction().context("开启事务失败")?;
        let deleted = match scope {
            Some(s) => {
                tx.execute(
                    "DELETE FROM gm_messages WHERE scope = ?1",
                    rusqlite::params![s],
                )?;
                tx.execute(
                    "DELETE FROM gm_turn_memories WHERE scope = ?1",
                    rusqlite::params![s],
                )?
            }
            None => {
                tx.execute("DELETE FROM gm_messages", [])?;
                tx.execute("DELETE FROM gm_turn_memories", [])?
            }
        };
        tx.commit().context("提交清空事务失败")?;
        Ok(deleted as u64)
    }
}

#[cfg(test)]
mod tests;
