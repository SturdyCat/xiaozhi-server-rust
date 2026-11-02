use super::SkillSystem;
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use serde_json::{json, Value};
use std::time::Duration;

pub(super) const MAX_ACTIVE_TASKS: i64 = 1024;
pub(super) const MAX_DEVICE_TASKS: i64 = 128;
pub(super) const MAX_PENDING: i64 = 2048;
pub(super) const MAX_DEVICE_PENDING: i64 = 256;
pub(super) const TASK_COLUMNS: &str = "id,device_id,text,schedule,state,next_due_ms,created_ms,updated_ms,(SELECT o.state FROM skills_outbox o WHERE o.task_id=skills_tasks.id ORDER BY o.created_ms DESC,o.id DESC LIMIT 1)";

impl SkillSystem {
    pub(super) fn connect(&self) -> Result<Connection> {
        ensure!(
            self.path.to_str() != Some(":memory:"),
            "skills requires a persistent SQLite file"
        );
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).context("create skills database directory")?;
        }
        let conn = Connection::open(&self.path).context("open skills database")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS skills_tasks (
                id TEXT PRIMARY KEY, device_id TEXT NOT NULL, text TEXT NOT NULL,
                schedule TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('active','completed','cancelled')),
                next_due_ms INTEGER NOT NULL, created_ms INTEGER NOT NULL, updated_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS skills_tasks_due ON skills_tasks(state,next_due_ms);
            CREATE INDEX IF NOT EXISTS skills_tasks_device ON skills_tasks(device_id,created_ms);
            CREATE TABLE IF NOT EXISTS skills_outbox (
                id TEXT PRIMARY KEY, task_id TEXT REFERENCES skills_tasks(id), device_id TEXT NOT NULL,
                text TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('pending','leased','submitted','cancelled')),
                available_ms INTEGER NOT NULL, created_ms INTEGER NOT NULL, updated_ms INTEGER NOT NULL,
                lease_token TEXT, lease_owner TEXT, lease_until_ms INTEGER
            );
            CREATE INDEX IF NOT EXISTS skills_outbox_device ON skills_outbox(device_id,state,available_ms);
            CREATE UNIQUE INDEX IF NOT EXISTS skills_outbox_unsettled_task ON skills_outbox(task_id)
                WHERE task_id IS NOT NULL AND state IN ('pending','leased');
            CREATE TABLE IF NOT EXISTS skills_requests (
                device_id TEXT NOT NULL, request_id TEXT NOT NULL, fingerprint TEXT NOT NULL,
                response TEXT NOT NULL, created_ms INTEGER NOT NULL,
                PRIMARY KEY(device_id,request_id)
            );")?;
        Ok(conn)
    }

    /// Management-only cross-device read; caller must enforce authorization.
    pub fn list_tasks(
        &self,
        device_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Value> {
        ensure!(
            (1..=100).contains(&limit) && offset <= 10000,
            "invalid task pagination"
        );
        let conn = self.connect()?;
        let total: i64 = conn.query_row(
            "SELECT count(*) FROM skills_tasks WHERE (?1 IS NULL OR device_id=?1)",
            params![device_id],
            |row| row.get(0),
        )?;
        let mut stmt = conn.prepare(&format!("SELECT {TASK_COLUMNS} FROM skills_tasks WHERE (?1 IS NULL OR device_id=?1) ORDER BY created_ms DESC,id LIMIT ?2 OFFSET ?3"))?;
        let tasks = stmt
            .query_map(params![device_id, limit as i64, offset as i64], task_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"tasks":tasks,"total":total}))
    }

    /// None deliberately covers both not-found and a mismatched bound device.
    pub fn get_task(&self, task_id: &str, device_id: Option<&str>) -> Result<Option<Value>> {
        let conn = self.connect()?;
        Ok(conn.query_row(&format!("SELECT {TASK_COLUMNS} FROM skills_tasks WHERE id=?1 AND (?2 IS NULL OR device_id=?2)"), params![task_id,device_id], task_row).optional()?)
    }
}

pub(super) fn task_row(row: &Row<'_>) -> rusqlite::Result<Value> {
    let schedule: String = row.get(3)?;
    let schedule: Value = serde_json::from_str(&schedule).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(json!({
        "id":row.get::<_,String>(0)?, "device_id":row.get::<_,String>(1)?,
        "text":row.get::<_,String>(2)?, "schedule":schedule,
        "state":row.get::<_,String>(4)?, "schedule_state":row.get::<_,String>(4)?,
        "delivery_state":row.get::<_,Option<String>>(8)?.unwrap_or_else(|| "none".into()),
        "played":Value::Null, "next_due_ms":row.get::<_,i64>(5)?,
        "created_ms":row.get::<_,i64>(6)?, "updated_ms":row.get::<_,i64>(7)?
    }))
}

pub(super) fn pending_capacity(tx: &Transaction<'_>, device_id: &str) -> Result<()> {
    let (all, device): (i64, i64) = tx.query_row("SELECT count(*),coalesce(sum(device_id=?1),0) FROM skills_outbox WHERE state IN ('pending','leased')", params![device_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    ensure!(
        all < MAX_PENDING && device < MAX_DEVICE_PENDING,
        "outbox capacity exceeded; pending deliveries retained"
    );
    Ok(())
}

pub(super) fn prune(tx: &Transaction<'_>, now_ms: i64) -> Result<()> {
    // Terminal history is bounded; pending/offline deliveries are never evicted.
    tx.execute("DELETE FROM skills_outbox WHERE id IN (SELECT id FROM skills_outbox WHERE state IN ('submitted','cancelled') ORDER BY updated_ms DESC,id LIMIT -1 OFFSET 2048)", [])?;
    tx.execute("DELETE FROM skills_tasks WHERE id IN (SELECT t.id FROM skills_tasks t WHERE t.state!='active' AND NOT EXISTS(SELECT 1 FROM skills_outbox o WHERE o.task_id=t.id) ORDER BY t.updated_ms DESC,t.id LIMIT -1 OFFSET 2048)", [])?;
    // Idempotency retained for seven days, then may be reused. Hard cap below prevents unbounded logs.
    tx.execute(
        "DELETE FROM skills_requests WHERE created_ms < ?1",
        params![now_ms.saturating_sub(7 * 86_400_000)],
    )?;
    Ok(())
}
