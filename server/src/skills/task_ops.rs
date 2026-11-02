use super::{
    calendar, schedule::Schedule, store, SkillActionError, SkillExecutionContext, SkillSystem,
};
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::{json, Value};

pub(super) fn create_speech(
    text: &str,
    schedule: &Schedule,
    args: &Value,
    next_ms: i64,
) -> Result<String> {
    let timezone = args["timezone"].as_str().unwrap_or("UTC");
    Ok(format!(
        "已设置提醒“{}”，{}，下次时间为{}。",
        short(text, 80),
        calendar::frequency(schedule, Some(args)),
        calendar::local_datetime(next_ms, timezone)?
    ))
}

pub(super) fn list(
    system: &SkillSystem,
    ctx: &SkillExecutionContext,
    args: &Value,
) -> Result<Value> {
    let limit = args["limit"].as_u64().unwrap_or(50) as i64;
    let offset = args["offset"].as_u64().unwrap_or(0) as i64;
    let mut conn = system.connect()?;
    let tx = conn.transaction()?;
    let filter = "device_id=?1 AND (state='active' OR (state='completed' AND EXISTS(SELECT 1 FROM skills_outbox o WHERE o.task_id=skills_tasks.id AND o.state IN ('pending','leased'))))";
    let total: i64 = tx.query_row(
        &format!("SELECT count(*) FROM skills_tasks WHERE {filter}"),
        params![ctx.device_id],
        |row| row.get(0),
    )?;
    let tasks = {
        let mut stmt = tx.prepare(&format!(
            "SELECT {} FROM skills_tasks WHERE {filter} ORDER BY next_due_ms,id LIMIT ?2 OFFSET ?3",
            store::TASK_COLUMNS
        ))?;
        let rows = stmt.query_map(params![ctx.device_id, limit, offset], store::task_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut speech = if total == 0 {
        "当前设备没有活跃或待投递的提醒。".into()
    } else {
        format!("当前设备有{total}项活跃或待投递的提醒。")
    };
    let mut spoken = 0;
    for task in tasks.iter().take(8) {
        let schedule: Schedule = serde_json::from_value(task["schedule"].clone())?;
        let zone = calendar::timezone(&schedule).unwrap_or("UTC");
        let time = calendar::local_datetime(
            task["next_due_ms"].as_i64().context("task date missing")?,
            zone,
        )?;
        let status = match task["delivery_state"].as_str() {
            Some("pending") => "待投递",
            Some("leased") => "正在提交，未确认播放",
            Some("submitted") => "上一条已提交，未确认播放",
            _ => "等待到期",
        };
        let part = format!(
            "提醒“{}”，{}，下次或原定时间{}，{}。",
            short(task["text"].as_str().unwrap_or_default(), 40),
            calendar::frequency(&schedule, None),
            time,
            status
        );
        if speech.chars().count() + part.chars().count() > 1800 {
            break;
        }
        speech.push_str(&part);
        spoken += 1;
    }
    if total > spoken {
        speech.push_str("其余提醒请查看任务列表。");
    }
    ctx.check()?;
    tx.commit()?;
    Ok(json!({"tasks":tasks,"total":total,"speech":speech}))
}

pub(super) fn cancel(
    tx: &Transaction<'_>,
    ctx: &SkillExecutionContext,
    args: &Value,
) -> Result<Value> {
    let task_id = if let Some(id) = args["task_id"].as_str() {
        id.to_owned()
    } else {
        let text = args["text"].as_str().context("cancel text missing")?;
        let mut stmt = tx.prepare("SELECT id FROM skills_tasks WHERE device_id=?1 AND text=?2 AND (state='active' OR (state='completed' AND EXISTS(SELECT 1 FROM skills_outbox o WHERE o.task_id=skills_tasks.id AND o.state IN ('pending','leased')))) ORDER BY id LIMIT 2")?;
        let ids = stmt
            .query_map(params![ctx.device_id, text], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        match ids.as_slice() {
            [id]=>id.clone(),
            []=>return Err(SkillActionError { speech:"没有找到当前设备上与该内容完全相同的活跃提醒，请核对提醒内容或编号。".into() }.into()),
            _=>return Err(SkillActionError { speech:"当前设备有多个内容完全相同的提醒，不能确定要取消哪一个，请在管理任务列表中确认编号后按编号取消。".into() }.into()),
        }
    };
    let row: Option<(String, String)> = tx
        .query_row(
            "SELECT state,text FROM skills_tasks WHERE id=?1 AND device_id=?2",
            params![task_id, ctx.device_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let (state, text) = row.ok_or_else(|| SkillActionError {
        speech: "没有找到当前设备上的这个提醒，请核对编号。".into(),
    })?;
    tx.execute(
        "UPDATE skills_tasks SET state='cancelled',updated_ms=?3 WHERE id=?1 AND device_id=?2",
        params![task_id, ctx.device_id, ctx.now_ms],
    )?;
    tx.execute("UPDATE skills_outbox SET state='cancelled',lease_token=NULL,lease_owner=NULL,lease_until_ms=NULL,updated_ms=?3 WHERE task_id=?1 AND device_id=?2 AND state IN ('pending','leased')",params![task_id,ctx.device_id,ctx.now_ms])?;
    let speech = if state == "cancelled" {
        format!("提醒“{}”已经取消。", short(&text, 80))
    } else {
        format!(
            "已取消提醒“{}”及尚未提交的投递，已经提交的播报不能撤回。",
            short(&text, 80)
        )
    };
    Ok(
        json!({"task_id":task_id,"state":"cancelled","cancelled":state!="cancelled","speech":speech,"deduplicated":false}),
    )
}

fn short(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}
