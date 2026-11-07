//! 记忆库 schema 与迁移（复刻上游 graph-memory 的 m15/m16 两层表，**跳过 legacy 节点表**）。
//!
//! ## 纪律（照抄上游）
//!
//! - 建表全部 `IF NOT EXISTS` + `_migrations` 编号迁移：**只能追加、禁止改写已发布编号**；
//! - 连接初始化顺序：`busy_timeout` → `journal_mode=WAL` → `foreign_keys=ON` → 迁移；
//! - 稳定 ID 生成与上游同**方案**（sha256 前缀）：
//!   `memory = tm-<sha256(scope\0排序后的 source messageIds)[..32]>`。
//!   ⚠️ 输入里含 scope（我们与 DSH 的作用域不同），因此"与 DSH 同库直读"是 P3 的目标，
//!   这里保证的是**同方案**（幂等、可复算），不是逐字节同 ID。
//!
//! ## 我们的两处增补（上游缺陷，见 `docs/soul-and-graph-memory-plan.md` §2.4）
//!
//! 1. `gm_summary_terms`：给**轮次摘要**建倒排索引（上游只对 legacy 节点建了 FTS5，
//!    turn memory 的词法回退是 `LIKE '%完整短语%'`，中文场景几乎不可用）；
//! 2. `gm_turn_memory_sources` 的保留策略查询**必须排除被引用的消息**（上游
//!    `runMessageRetention` 只看 legacy 表，会把轮次记忆引用的原文删掉，证据层直接变空）。

use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::Connection;

/// 已发布的迁移编号（只增不改）。
pub const LATEST_VERSION: i64 = 2;

/// 打开连接并完成初始化（busy_timeout → WAL → foreign_keys → 迁移）。
pub fn open(path: &str, busy_timeout_ms: u64) -> Result<Connection> {
    let conn = Connection::open(path).with_context(|| format!("打开记忆库 {path} 失败"))?;
    conn.busy_timeout(Duration::from_millis(busy_timeout_ms))
        .context("设置 busy_timeout 失败")?;
    // :memory: 的 journal_mode 会返回 "memory"，不是错误；显式忽略结果即可。
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    conn.pragma_update(None, "foreign_keys", "ON")
        .context("开启 foreign_keys 失败")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);",
    )
    .context("创建 _migrations 失败")?;
    migrate(&conn)?;
    Ok(conn)
}

/// 按编号顺序补齐迁移（幂等：已应用的跳过）。
fn migrate(conn: &Connection) -> Result<()> {
    let applied: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM _migrations",
            [],
            |r| r.get(0),
        )
        .context("读取 _migrations 失败")?;
    for v in (applied + 1)..=LATEST_VERSION {
        let ddl = match v {
            1 => MIGRATION_1,
            2 => MIGRATION_2,
            _ => anyhow::bail!("记忆库迁移编号 {v} 未定义（代码与库版本不匹配）"),
        };
        conn.execute_batch(ddl)
            .with_context(|| format!("应用记忆库迁移 m{v} 失败"))?;
        conn.execute(
            "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
            rusqlite::params![v, now_ms()],
        )
        .context("记录迁移失败")?;
    }
    Ok(())
}

/// 当前毫秒时间戳（`created_at`/`next_retry_at` 用）。
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 稳定 ID 前缀（与上游同方案）。
pub fn stable_id(prefix: &str, parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            h.update([0u8]);
        }
        h.update(p.as_bytes());
    }
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    format!("{prefix}-{}", &hex[..32])
}

/// m1：原文事实源 + 抽取状态机。
const MIGRATION_1: &str = r#"
-- 原文事实源（不可变）：一轮 = user + assistant 两条
CREATE TABLE IF NOT EXISTS gm_messages (
    id                TEXT PRIMARY KEY,
    scope             TEXT NOT NULL,
    session_id        TEXT NOT NULL,
    turn_index        INTEGER NOT NULL,
    role              TEXT NOT NULL,
    content           TEXT NOT NULL,
    created_at        INTEGER NOT NULL,
    extraction_state  TEXT NOT NULL DEFAULT 'pending',
    extraction_attempts INTEGER NOT NULL DEFAULT 0,
    extraction_error  TEXT,
    next_retry_at     INTEGER,
    updated_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_gm_messages_scope_turn ON gm_messages(scope, turn_index);
CREATE INDEX IF NOT EXISTS idx_gm_messages_queue ON gm_messages(extraction_state, created_at);
"#;

/// m2：轮次记忆（导航层）+ 溯源（证据绑定）+ 我们的倒排索引。
const MIGRATION_2: &str = r#"
CREATE TABLE IF NOT EXISTS gm_turn_memories (
    id          TEXT PRIMARY KEY,
    scope       TEXT NOT NULL,
    session_id  TEXT NOT NULL,
    turn_index  INTEGER NOT NULL,
    summary     TEXT NOT NULL,
    outcome     TEXT NOT NULL,
    keywords    TEXT NOT NULL DEFAULT '[]',
    created_at  INTEGER NOT NULL,
    UNIQUE(scope, session_id, turn_index)
);
CREATE INDEX IF NOT EXISTS idx_gm_turn_memories_scope_turn
    ON gm_turn_memories(scope, turn_index);
-- memory → 原文的溯源（删原文会连带删溯源行；记忆本身保留，只是失去证据）
CREATE TABLE IF NOT EXISTS gm_turn_memory_sources (
    memory_id  TEXT NOT NULL REFERENCES gm_turn_memories(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL REFERENCES gm_messages(id) ON DELETE CASCADE,
    PRIMARY KEY (memory_id, message_id)
) WITHOUT ROWID;
-- 我们的增补：摘要的倒排词项索引（见文件头"两处增补"）
CREATE TABLE IF NOT EXISTS gm_summary_terms (
    term       TEXT NOT NULL,
    memory_id  TEXT NOT NULL REFERENCES gm_turn_memories(id) ON DELETE CASCADE,
    scope      TEXT NOT NULL,
    PRIMARY KEY (term, memory_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS idx_gm_summary_terms_lookup ON gm_summary_terms(scope, term);
"#;
