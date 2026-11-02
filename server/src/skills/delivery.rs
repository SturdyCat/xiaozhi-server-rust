use super::{schedule::Schedule, store, DeliveryClaim, SkillSystem};
use anyhow::{ensure, Result};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

const LEASE_MS: i64 = 300_000;

/// 到期动作任务：`skills_tasks.action` 非空的任务到期后**不进 outbox**，
/// 由 scheduler 拿去直推（在线设备）或降级为纯文字提醒（离线/忙/失败）。
#[derive(Debug, Clone)]
pub struct DueAction {
    pub task_id: String,
    pub device_id: String,
    /// 任务原文，兼作直推成功后的播报文本与离线降级的提醒文本。
    pub text: String,
    /// 落库的原始 action JSON（单个 `devices.call` SkillStep，见 plan.rs 校验）。
    pub action: String,
}

/// [`SkillSystem::tick_due`] 的结果：普通任务照旧物化进 outbox（`materialized` 计数），
/// 动作任务原样交还调用方（`actions`），调度推进（next_due_ms / 完成）在本函数内完成。
#[derive(Debug, Default)]
pub struct TickReport {
    pub materialized: usize,
    pub actions: Vec<DueAction>,
}

impl SkillSystem {
    /// Materialize at most 256 due tasks atomically; no actual speech is performed.
    pub fn tick_due(&self, now_ms: i64) -> Result<TickReport> {
        ensure!(now_ms >= 0, "now_ms must be nonnegative");
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        store::prune(&tx, now_ms)?;
        let due = {
            let mut stmt = tx.prepare("SELECT id,device_id,text,schedule,next_due_ms,action FROM skills_tasks WHERE state='active' AND next_due_ms<=?1 AND (EXISTS(SELECT 1 FROM skills_outbox own WHERE own.task_id=skills_tasks.id AND own.state IN ('pending','leased')) OR ((SELECT count(*) FROM skills_outbox all_pending WHERE all_pending.state IN ('pending','leased'))<2048 AND (SELECT count(*) FROM skills_outbox device_pending WHERE device_pending.device_id=skills_tasks.device_id AND device_pending.state IN ('pending','leased'))<256)) ORDER BY next_due_ms,id LIMIT 256")?;
            let rows = stmt.query_map(params![now_ms], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut report = TickReport::default();
        for (id, device_id, text, schedule_json, previous_ms, action) in due {
            let action = action.filter(|a| !a.trim().is_empty());
            match action {
                // 动作任务：不进 outbox（直推结果决定播报文本），但沿用同一背压——
                // outbox 满时连调度也推进不了，否则一次性动作任务会"标记完成却无声无息"。
                Some(action) => {
                    let (all, device): (i64, i64) = tx.query_row("SELECT count(*),coalesce(sum(device_id=?1),0) FROM skills_outbox WHERE state IN ('pending','leased')", params![device_id], |row| Ok((row.get(0)?,row.get(1)?)))?;
                    if all >= store::MAX_PENDING || device >= store::MAX_DEVICE_PENDING {
                        continue;
                    }
                    report.actions.push(DueAction {
                        task_id: id.clone(),
                        device_id: device_id.clone(),
                        text,
                        action,
                    });
                }
                None => {
                    let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM skills_outbox WHERE task_id=?1 AND state IN ('pending','leased'))", params![id], |row| row.get(0))?;
                    if !exists {
                        let (all, device): (i64,i64) = tx.query_row("SELECT count(*),coalesce(sum(device_id=?1),0) FROM skills_outbox WHERE state IN ('pending','leased')", params![device_id], |row| Ok((row.get(0)?,row.get(1)?)))?;
                        if all >= store::MAX_PENDING || device >= store::MAX_DEVICE_PENDING {
                            // Backpressure: don't lose a one-shot or advance an undelivered schedule.
                            continue;
                        }
                        tx.execute("INSERT INTO skills_outbox(id,task_id,device_id,text,state,available_ms,created_ms,updated_ms) VALUES (?1,?2,?3,?4,'pending',?5,?5,?5)", params![Uuid::new_v4().to_string(),id,device_id,text,now_ms])?;
                        report.materialized += 1;
                    }
                }
            }
            let schedule: Schedule = serde_json::from_str(&schedule_json)?;
            match schedule.next_after(previous_ms, now_ms)? {
                Some(next) => {
                    tx.execute(
                        "UPDATE skills_tasks SET next_due_ms=?2,updated_ms=?3 WHERE id=?1",
                        params![id, next, now_ms],
                    )?;
                }
                None => {
                    tx.execute(
                        "UPDATE skills_tasks SET state='completed',updated_ms=?2 WHERE id=?1",
                        params![id, now_ms],
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(report)
    }

    /// scheduler 在动作直推后写入的播报（成功原文 / 失败提示 / 离线降级原文）。
    /// 与 `tick_due` 同一套约束：每任务至多一条未投递（唯一 unsettled 索引）、
    /// 全局/设备背压。写不进返回 `false`——不丢任务，只是本轮播报让位于
    /// 上一轮设备还没认领的播报（与循环任务在 `tick_due` 内的跳过语义一致）。
    pub fn enqueue_task_speech(
        &self,
        task_id: &str,
        device_id: &str,
        text: &str,
        now_ms: i64,
    ) -> Result<bool> {
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        store::prune(&tx, now_ms)?;
        let unsettled: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM skills_outbox WHERE task_id=?1 AND state IN ('pending','leased'))",
            params![task_id],
            |row| row.get(0),
        )?;
        if unsettled {
            tx.commit()?;
            return Ok(false);
        }
        let (all, device): (i64, i64) = tx.query_row(
            "SELECT count(*),coalesce(sum(device_id=?1),0) FROM skills_outbox WHERE state IN ('pending','leased')",
            params![device_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if all >= store::MAX_PENDING || device >= store::MAX_DEVICE_PENDING {
            tx.commit()?;
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO skills_outbox(id,task_id,device_id,text,state,available_ms,created_ms,updated_ms) VALUES (?1,?2,?3,?4,'pending',?5,?5,?5)",
            params![Uuid::new_v4().to_string(), task_id, device_id, text, now_ms],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Atomic per-device lease. Expired leases recover after process death; offline rows persist.
    pub fn claim_delivery(
        &self,
        device_id: &str,
        session_id: &str,
        now_ms: i64,
    ) -> Result<Option<DeliveryClaim>> {
        ensure!(
            !device_id.is_empty()
                && device_id.len() <= 256
                && !session_id.is_empty()
                && session_id.len() <= 256
                && now_ms >= 0,
            "invalid delivery claim context"
        );
        let mut conn = self.connect()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<(String,Option<String>,String)> = tx.query_row(
            "SELECT o.id,o.task_id,o.text FROM skills_outbox o WHERE o.device_id=?1 AND o.available_ms<=?2 AND NOT EXISTS(SELECT 1 FROM skills_outbox busy WHERE busy.device_id=?1 AND busy.state='leased' AND busy.lease_until_ms>?2) AND (o.state='pending' OR (o.state='leased' AND o.lease_until_ms<=?2)) AND (o.task_id IS NULL OR EXISTS(SELECT 1 FROM skills_tasks t WHERE t.id=o.task_id AND t.state!='cancelled')) ORDER BY o.created_ms,o.id LIMIT 1",
            params![device_id,now_ms], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional()?;
        let Some((id, task_id, text)) = row else {
            tx.commit()?;
            return Ok(None);
        };
        let lease_token = Uuid::new_v4().to_string();
        let lease_until_ms = now_ms
            .checked_add(LEASE_MS)
            .ok_or_else(|| anyhow::anyhow!("lease timestamp overflow"))?;
        tx.execute("UPDATE skills_outbox SET state='leased',lease_token=?2,lease_owner=?3,lease_until_ms=?4,updated_ms=?5 WHERE id=?1", params![id,lease_token,session_id,lease_until_ms,now_ms])?;
        tx.commit()?;
        Ok(Some(DeliveryClaim {
            id,
            task_id,
            device_id: device_id.into(),
            session_id: session_id.into(),
            text,
            lease_token,
            lease_until_ms,
        }))
    }

    pub fn delivery_is_current(&self, claim: &DeliveryClaim, now_ms: i64) -> Result<bool> {
        let conn = self.connect()?;
        Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM skills_outbox o WHERE o.id=?1 AND o.device_id=?2 AND o.lease_owner=?3 AND o.lease_token=?4 AND o.state='leased' AND o.lease_until_ms>?5 AND (o.task_id IS NULL OR EXISTS(SELECT 1 FROM skills_tasks t WHERE t.id=o.task_id AND t.state!='cancelled')))", params![claim.id,claim.device_id,claim.session_id,claim.lease_token,now_ms], |row| row.get(0))?)
    }

    /// submitted means all audio was handed to transport, never proof of device playback.
    /// false releases for retry. Stale/cancelled claims cannot ack another owner's lease.
    pub fn finish_delivery(
        &self,
        claim: &DeliveryClaim,
        submitted: bool,
        now_ms: i64,
    ) -> Result<bool> {
        ensure!(now_ms >= 0, "now_ms must be nonnegative");
        let conn = self.connect()?;
        let changed = conn.execute(
            "UPDATE skills_outbox SET state=?6,lease_token=NULL,lease_owner=NULL,lease_until_ms=NULL,available_ms=?7,updated_ms=?5 WHERE id=?1 AND device_id=?2 AND lease_owner=?3 AND lease_token=?4 AND state='leased' AND (?6='pending' OR lease_until_ms>?5) AND (task_id IS NULL OR EXISTS(SELECT 1 FROM skills_tasks t WHERE t.id=task_id AND t.state!='cancelled'))",
            params![claim.id,claim.device_id,claim.session_id,claim.lease_token,now_ms,if submitted { "submitted" } else { "pending" },now_ms.saturating_add(1000)],
        )?;
        Ok(changed == 1)
    }
}
