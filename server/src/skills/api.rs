use super::{schedule::Schedule, store, SkillExecutionContext, SkillStep, SkillSystem};
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use uuid::Uuid;

pub(super) fn execute(
    system: &SkillSystem,
    ctx: &SkillExecutionContext,
    step: &SkillStep,
) -> Result<Value> {
    match step.api.as_str() {
        "tasks.list" => return super::task_ops::list(system, ctx, &step.args),
        "tasks.get" => {
            return Ok(
                json!({"task": system.get_task(string(&step.args,"task_id")?,Some(&ctx.device_id))?.context("task not found for this device")?}),
            )
        }
        "devices.call" | "sessions.end" | "replies.say" => {
            ctx.check()?;
            return Ok(json!({"dispatch":step.api,"args":step.args}));
        }
        _ => {}
    }
    let mut conn = system.connect()?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ctx.check()?;
    store::prune(&tx, ctx.now_ms)?;
    let fingerprint = serde_json::to_string(step)?;
    let prior: Option<(String, String)> = tx
        .query_row(
            "SELECT fingerprint,response FROM skills_requests WHERE device_id=?1 AND request_id=?2",
            params![ctx.device_id, ctx.request_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((old_fingerprint, response)) = prior {
        ensure!(
            old_fingerprint == fingerprint,
            "idempotency conflict: request_id reused with different arguments"
        );
        let mut response: Value = serde_json::from_str(&response)?;
        if let Some(map) = response.as_object_mut() {
            map.insert("deduplicated".into(), Value::Bool(true));
        }
        ctx.check()?;
        tx.commit()?;
        return Ok(response);
    }
    let count: i64 = tx.query_row("SELECT count(*) FROM skills_requests", [], |row| row.get(0))?;
    ensure!(
        count < 8192,
        "idempotency capacity exceeded (seven-day retention)"
    );
    let response = match step.api.as_str() {
        "tasks.create" => {
            let (all, device): (i64, i64) = tx.query_row("SELECT count(*),coalesce(sum(device_id=?1),0) FROM skills_tasks WHERE state='active'", params![ctx.device_id], |row| Ok((row.get(0)?,row.get(1)?)))?;
            ensure!(
                all < store::MAX_ACTIVE_TASKS && device < store::MAX_DEVICE_TASKS,
                "task capacity exceeded"
            );
            let (schedule, next_due_ms) = Schedule::parse(&step.args, ctx.now_ms).map_err(|_| super::SkillActionError { speech:"时间参数无法解析或已过期，请提供明确的日期和钟点、合法的循环日期或正时长；十二点请使用00:00或12:00。".into() })?;
            let id = Uuid::new_v4().to_string();
            let text = string(&step.args, "text")?;
            let schedule_json = serde_json::to_string(&schedule)?;
            // 到期动作（可空）：validate_step 已把它锁成单个 devices.call 步骤；这里
            // 原样序列化落库，到期时由 scheduler 取出直推（在线设备）或降级为提醒。
            let action_json: Option<String> = match step.args.get("action") {
                None | Some(Value::Null) => None,
                Some(action) => {
                    let parsed: super::SkillStep = serde_json::from_value(action.clone())
                        .map_err(|_| super::SkillActionError { speech: "到期动作参数无法解析，请提供单个 devices.call 步骤。".into() })?;
                    Some(serde_json::to_string(&parsed)?)
                }
            };
            tx.execute("INSERT INTO skills_tasks(id,device_id,text,schedule,state,next_due_ms,created_ms,updated_ms,action) VALUES (?1,?2,?3,?4,'active',?5,?6,?6,?7)", params![id,ctx.device_id,text,schedule_json,next_due_ms,ctx.now_ms,action_json])?;
            let task = tx.query_row(
                &format!(
                    "SELECT {} FROM skills_tasks WHERE id=?1",
                    store::TASK_COLUMNS
                ),
                params![id],
                store::task_row,
            )?;
            let speech = super::task_ops::create_speech(text, &schedule, &step.args, next_due_ms)?;
            json!({"task":task,"speech":speech,"deduplicated":false})
        }
        "tasks.cancel" => super::task_ops::cancel(&tx, ctx, &step.args)?,
        "messages.send" => {
            store::pending_capacity(&tx, &ctx.device_id)?;
            let id = Uuid::new_v4().to_string();
            tx.execute("INSERT INTO skills_outbox(id,task_id,device_id,text,state,available_ms,created_ms,updated_ms) VALUES (?1,NULL,?2,?3,'pending',?4,?4,?4)", params![id,ctx.device_id,string(&step.args,"text")?,ctx.now_ms])?;
            json!({"message_id":id,"device_id":ctx.device_id,"state":"pending","deduplicated":false})
        }
        other => bail!("unknown skill capability: {other}"),
    };
    tx.execute("INSERT INTO skills_requests(device_id,request_id,fingerprint,response,created_ms) VALUES (?1,?2,?3,?4,?5)", params![ctx.device_id,ctx.request_id,fingerprint,serde_json::to_string(&response)?,ctx.now_ms])?;
    // Cancellation wins whenever observed before this commit; previously committed steps are not undone.
    ctx.check()?;
    tx.commit()?;
    Ok(response)
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("{key} must be a string"))
}
