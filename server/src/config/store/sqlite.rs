//! SQLite 配置后端（P1/P2 落地）。
//!
//! ## 为什么是 SQLite
//!
//! 设计目标见 [`super::AGENTS.md`]（配置迁移设计）：无配置文件也能保存、单文件 bind mount
//! 下写配置非原子、没有历史/回滚/审计、overlay/预设散落文件、配置与记忆各存一处。
//! 配置进 SQLite 后一次事务搞定以上全部，且天然获得历史/回滚/审计。
//!
//! ## 与 memory 共库纪律（最高优先级，见 AGENTS.md §3.1 / §3.2）
//!
//! - 迁移命名空间**独立**用 `_cfg_migrations`，绝不写进 memory 的 `_migrations`
//!   （后果：把两个子系统的编号语义搅在一起，导致静默漏迁移）；
//! - 连接**自持**：`[memory].enabled=false` 时记忆库不打开，但配置库仍可读写
//!   （配置是必需能力），因此 `cfg_*` 表在记忆库文件里也成立；
//! - 并发模型与 `plugins/memory/store.rs` 同款：单连接 + `Mutex` 串行化（SQLite 写锁全局，
//!   连接池多写只会把竞争搬到 Rust 侧），配合 WAL + `busy_timeout` 与 DSH 进程共用同一文件。
//!
//! ## Schema 取舍（见 AGENTS.md §4）
//!
//! 整份 Config 作为 JSON blob 存于 `cfg_documents`（等价于今天的 config.toml），而非逐字段
//! KV 拆表——现在**全部**语义都作用在"整份文档"上（部分更新合并、内容指纹、normalize、
//! 密钥打码、serde 精确类型错误），拆 KV 等于重写一遍这些逻辑，收益不值。overlay / 预设
//! 各一张表（取代原来的 `overlays/devices/<id>.toml` 与 `presets/<name>.toml`）。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::types::ValueRef;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{Map, Value};

use super::ConfigStore;
use crate::config::Config;

/// 配置库迁移编号（只增不改）。
const CFG_LATEST_VERSION: i64 = 1;

/// SQLite 配置后端。
pub struct SqliteConfigStore {
    conn: Mutex<Connection>,
    path: String,
}

impl SqliteConfigStore {
    /// 打开（或创建）配置库并完成初始化：父目录创建 → `busy_timeout` → `WAL` → `foreign_keys`
    /// → `_cfg_migrations` → 迁移建表。返回 `Arc`（与 [`crate::engine::Engines`] 共享）。
    pub fn open(path: &str) -> Result<Arc<Self>> {
        if path != ":memory:" {
            if let Some(dir) = std::path::Path::new(path).parent() {
                if !dir.as_os_str().is_empty() {
                    std::fs::create_dir_all(dir)
                        .with_context(|| format!("创建配置库目录 {} 失败", dir.display()))?;
                }
            }
        }
        let conn =
            Connection::open(path).with_context(|| format!("打开配置库 {path} 失败"))?;
        conn.busy_timeout(Duration::from_millis(5000))
            .context("设置 busy_timeout 失败")?;
        // :memory: 的 journal_mode 返回 "memory"，不是错误；显式忽略结果即可。
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        conn.pragma_update(None, "foreign_keys", "ON")
            .context("开启 foreign_keys 失败")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _cfg_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);",
        )
        .context("创建 _cfg_migrations 失败")?;
        migrate(&conn)?;
        Ok(Arc::new(Self {
            conn: Mutex::new(conn),
            path: path.to_string(),
        }))
    }

    /// 取锁（中毒后继续用：单条 SQL 失败不该让配置能力永久不可用）。
    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 配置内容指纹（FNV-1a）：与 `ws/config.rs::config_revision` 同源思路，
    /// 但此处基于**规范 JSON 字符串**（整份 Config 序列化后稳定），用于 `cfg_documents.revision`
    /// 与 `cfg_revisions` 主键。无新依赖、跨进程稳定、够快（配置仅几 KB）。
    fn revision_of(doc: &str) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for b in doc.as_bytes() {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{hash:016x}")
    }

    /// 写 `cfg_documents`（BEGIN IMMEDIATE 抢占写锁）+ 落一份到 `cfg_revisions`
    /// （`source` 区分来源：save / bootstrap / preset:<name> / rollback）。
    fn write_doc(&self, cfg: &Config, source: &str) -> Result<()> {
        let doc = serde_json::to_string(cfg).context("序列化配置失败")?;
        let now = now_ms();
        let rev = Self::revision_of(&doc);
        let mut conn = self.conn();
        let tx = conn.transaction().context("开启事务失败")?;
        tx.execute(
            "INSERT INTO cfg_documents (id, doc, revision, updated_at, updated_by)
             VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               doc=excluded.doc, revision=excluded.revision,
               updated_at=excluded.updated_at, updated_by=excluded.updated_by",
            params![doc, rev, now, source],
        )
        .context("写入 cfg_documents 失败")?;
        tx.execute(
            "INSERT INTO cfg_revisions (revision, doc, created_at, source, author, note)
             VALUES (?1, ?2, ?3, ?4, NULL, NULL)",
            params![rev, doc, now, source],
        )
        .context("写入 cfg_revisions 失败")?;
        tx.commit().context("提交配置事务失败")
    }
}

/// 按编号顺序补齐配置库迁移（幂等：已应用的跳过）。
fn migrate(conn: &Connection) -> Result<()> {
    let applied: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM _cfg_migrations",
            [],
            |r| r.get(0),
        )
        .context("读取 _cfg_migrations 失败")?;
    for v in (applied + 1)..=CFG_LATEST_VERSION {
        let ddl = match v {
            1 => MIGRATION_1,
            _ => anyhow::bail!("配置库迁移编号 {v} 未定义（代码与库版本不匹配）"),
        };
        conn.execute_batch(ddl)
            .with_context(|| format!("应用配置库迁移 m{v} 失败"))?;
        conn.execute(
            "INSERT INTO _cfg_migrations (version, applied_at) VALUES (?1, ?2)",
            params![v, now_ms()],
        )
        .context("记录配置库迁移失败")?;
    }
    Ok(())
}

/// 当前毫秒时间戳。
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 把 SQLite 列值转成 JSON（blob 只报长度，避免把二进制塞进 JSON）。
fn value_to_json(v: ValueRef) -> Result<Value> {
    use ValueRef::*;
    Ok(match v {
        Null => Value::Null,
        Integer(i) => Value::from(i),
        Real(f) => Value::from(f),
        Text(t) => Value::from(String::from_utf8_lossy(t).into_owned()),
        Blob(b) => Value::from(format!("blob:{}bytes", b.len())),
    })
}

impl ConfigStore for SqliteConfigStore {
    fn location(&self) -> Option<&str> {
        Some(&self.path)
    }

    fn backend(&self) -> &'static str {
        "sqlite"
    }

    fn load(&self) -> Result<Config> {
        let conn = self.conn();
        let doc: Option<String> = conn
            .query_row("SELECT doc FROM cfg_documents WHERE id = 1", [], |r| r.get(0))
            .optional()
            .context("读取 cfg_documents 失败")?;
        match doc {
            Some(doc) => serde_json::from_str::<Config>(&doc).context("解析配置库中的 Config 失败"),
            // 空库：返回内置默认（与文件后端"无文件→内置默认"语义一致；首次保存即落库）
            None => Ok(Config::default()),
        }
    }

    fn persist(&self, cfg: &Config) -> Result<()> {
        self.write_doc(cfg, "save")
    }

    fn bootstrap_import(&self, cfg: &Config) -> Result<()> {
        self.write_doc(cfg, "bootstrap")
    }

    fn is_empty(&self) -> bool {
        let conn = self.conn();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM cfg_documents", [], |r| r.get(0))
            .unwrap_or(0);
        n == 0
    }

    // ---- 每设备 overlay ----

    fn load_overlay(&self, device_id: &str) -> Result<Option<Value>> {
        let conn = self.conn();
        let patch: Option<String> = conn
            .query_row(
                "SELECT patch FROM cfg_device_overlays WHERE device_id = ?1",
                params![device_id],
                |r| r.get(0),
            )
            .optional()
            .context("读取设备 overlay 失败")?;
        match patch {
            Some(p) => Ok(Some(
                serde_json::from_str::<Value>(&p).context("解析设备 overlay 失败")?,
            )),
            None => Ok(None),
        }
    }

    fn save_overlay(&self, device_id: &str, patch: &Value) -> Result<()> {
        let patch = serde_json::to_string(patch).context("序列化 overlay 失败")?;
        let conn = self.conn();
        conn.execute(
            "INSERT INTO cfg_device_overlays (device_id, patch, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(device_id) DO UPDATE SET patch=excluded.patch, updated_at=excluded.updated_at",
            params![device_id, patch, now_ms()],
        )
        .context("写入设备 overlay 失败")?;
        Ok(())
    }

    fn delete_overlay(&self, device_id: &str) -> Result<()> {
        self.conn()
            .execute(
                "DELETE FROM cfg_device_overlays WHERE device_id = ?1",
                params![device_id],
            )
            .context("删除设备 overlay 失败")?;
        Ok(())
    }

    // ---- 配置预设 ----

    fn list_presets(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT name FROM cfg_presets ORDER BY name")
            .context("准备预设列表查询失败")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .context("查询预设列表失败")?;
        let mut names = Vec::new();
        for n in rows.collect::<rusqlite::Result<Vec<_>>>().context("读取预设列表失败")? {
            names.push(n);
        }
        Ok(names)
    }

    fn save_preset(&self, name: &str, doc: &Config) -> Result<()> {
        let doc = serde_json::to_string(doc).context("序列化预设失败")?;
        let conn = self.conn();
        conn.execute(
            "INSERT INTO cfg_presets (name, doc, created_at, note)
             VALUES (?1, ?2, ?3, NULL)
             ON CONFLICT(name) DO UPDATE SET doc=excluded.doc, created_at=excluded.created_at",
            params![name, doc, now_ms()],
        )
        .context("保存预设失败")?;
        Ok(())
    }

    fn load_preset(&self, name: &str) -> Result<Option<Config>> {
        let conn = self.conn();
        let doc: Option<String> = conn
            .query_row(
                "SELECT doc FROM cfg_presets WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )
            .optional()
            .context("读取预设失败")?;
        match doc {
            Some(doc) => Ok(Some(
                serde_json::from_str::<Config>(&doc).context("解析预设失败")?,
            )),
            None => Ok(None),
        }
    }

    fn delete_preset(&self, name: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM cfg_presets WHERE name = ?1", params![name])
            .context("删除预设失败")?;
        Ok(())
    }

    // ---- 只读 SQL 查询（远程客户端访问入口）----

    /// 只读 SQL 查询：仅允许单条 `SELECT` / `WITH` / `PRAGMA` / `EXPLAIN`，返回行数组
    /// （每行是列名 → 值的 map，最多 1000 行）。拒绝写入/DDL，避免把"远程查库"变成"远程改库"。
    fn query(&self, sql: &str) -> Result<Vec<Map<String, Value>>> {
        let trimmed = sql.trim();
        if trimmed.is_empty() {
            anyhow::bail!("SQL 为空。");
        }
        // 单语句：去掉末尾 ; 后不得再含 ;（防止 "SELECT 1; DROP TABLE ..."）。
        let without_tail = trimmed.trim_end_matches(';').trim();
        if without_tail.contains(';') {
            anyhow::bail!("只读查询仅支持单条语句（检测到多个 ; ）。");
        }
        let upper = without_tail.to_ascii_uppercase();
        let allowed = upper.starts_with("SELECT")
            || upper.starts_with("WITH")
            || upper.starts_with("PRAGMA")
            || upper.starts_with("EXPLAIN");
        if !allowed {
            anyhow::bail!(
                "只读查询仅允许 SELECT / WITH / PRAGMA / EXPLAIN 开头，已拒绝写入/DDL 语句。"
            );
        }
        let conn = self.conn();
        let mut stmt = conn.prepare(trimmed).context("准备查询失败")?;
        let column_count = stmt.column_count();
        let column_names: Vec<String> = (0..column_count)
            .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
            .collect();
        let mut rows = stmt.query([]).context("执行查询失败")?;
        let mut out = Vec::new();
        const MAX_ROWS: usize = 1000;
        while let Some(row) = rows.next().context("读取行失败")? {
            if out.len() >= MAX_ROWS {
                break;
            }
            let mut obj = Map::new();
            for (i, name) in column_names.iter().enumerate() {
                let v = value_to_json(row.get_ref(i).context("读列失败")?)?;
                obj.insert(name.clone(), v);
            }
            out.push(obj);
        }
        if out.len() >= MAX_ROWS {
            tracing::warn!("只读查询返回行数已达上限 {MAX_ROWS}，后续被截断。");
        }
        Ok(out)
    }

    /// 备份数据库：`VACUUM INTO <dest>`（整库一致性快照，含 -wal/-shm 无需单独处理）。
    /// `dest` 由调用方生成（服务端唯一路径），本方法再做一次注入护栏。
    fn backup(&self, dest: &str) -> Result<()> {
        if dest.contains('\'') || dest.contains(';') || dest.contains('\0') {
            anyhow::bail!("备份路径含非法字符（' 或 ; 或 NUL）。");
        }
        if let Some(dir) = std::path::Path::new(dest).parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("创建备份目录 {} 失败", dir.display()))?;
            }
        }
        let conn = self.conn();
        conn.execute_batch(&format!("VACUUM INTO '{dest}'"))
            .with_context(|| format!("备份数据库到 {dest} 失败"))
    }
}

/// m1：配置库全部表（整份配置 blob + 历史/回滚 + 设备 overlay + 预设）。
///
/// 与 memory 的 `_migrations` **完全独立**（见文件头"与 memory 共库纪律"）。
const MIGRATION_1: &str = r#"
-- 全局配置：整份 Config 的 JSON blob（等价于今天的 config.toml），唯一一行
CREATE TABLE IF NOT EXISTS cfg_documents (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    doc         TEXT    NOT NULL,   -- 规范化后的 Config JSON
    revision    TEXT    NOT NULL,   -- FNV-1a 内容指纹
    updated_at  INTEGER NOT NULL,
    updated_by  TEXT                -- 审计：bootstrap / save / preset:<name> / rollback
);

-- 历史与回滚：每次保存/导入/应用预设各留一份（source 区分来源）
CREATE TABLE IF NOT EXISTS cfg_revisions (
    revision    TEXT PRIMARY KEY,
    doc         TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    source      TEXT NOT NULL,      -- save | bootstrap | preset:<name> | rollback
    author      TEXT,
    note        TEXT
);
CREATE INDEX IF NOT EXISTS idx_cfg_revisions_created ON cfg_revisions(created_at);

-- 设备 overlay：取代 overlays/devices/<id>.toml（白名单校验在调用方已完成）
CREATE TABLE IF NOT EXISTS cfg_device_overlays (
    device_id   TEXT PRIMARY KEY,
    patch       TEXT NOT NULL,      -- JSON，只含被覆盖字段
    updated_at  INTEGER NOT NULL
);

-- 预设：取代 presets/<name>.toml（快照语义与今天一致）
CREATE TABLE IF NOT EXISTS cfg_presets (
    name        TEXT PRIMARY KEY,
    doc         TEXT NOT NULL,      -- 完整 Config JSON
    created_at  INTEGER NOT NULL,
    note        TEXT
);
"#;

#[cfg(test)]
mod tests;
