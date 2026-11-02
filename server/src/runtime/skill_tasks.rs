//! Durable skill scheduling is separate from intent recognition and device audio.
//! This worker materializes due outbox entries; sessions own delivery/ack.
//!
//! 到期**动作**任务（`skills_tasks.action` 非空）是本 worker 唯一触发的设备副作用：
//! 经 [`DeviceCmdHub`] 把工具调用投给该设备的活动会话执行（transport 独占在会话
//! 循环里，见 `app/device_cmds`）。在线直推一次，不重试；离线 / 忙 / 失败一律
//! 降级为纯文字提醒（原文经 [`SkillSystem::enqueue_task_speech`] 进 outbox）。
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::runtime::Engines;
use crate::skills::{DueAction, SkillStep, SkillSystem};
use crate::workflow::device_cmds::{DeviceCmd, DeviceCmdHub};

/// 直推结果等待上限。会话侧 MCP 调用自带 2s 超时（`mcp::DEFAULT_CALL_TIMEOUT`），
/// 这里只是兜底（正常必然先回）；即便超时，会话侧的工具调用仍会执行完——
/// 命令已经投出去了，放弃的只是"等确认"。
const ACTION_CALL_TIMEOUT: Duration = Duration::from_secs(15);

/// Dropping the application lifetime guard cancels the timer task. In-flight SQLite
/// transactions finish atomically in the blocking pool; an in-flight device command
/// already handed to a session may still execute (the session owns its transport).
pub(crate) struct SchedulerGuard(tokio::task::JoinHandle<()>);

impl Drop for SchedulerGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) fn start(engines: &Engines) -> SchedulerGuard {
    let weak = Arc::downgrade(&engines.skill_system);
    let hub = engines.device_cmds.clone();
    SchedulerGuard(tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_error: Option<Instant> = None;
        loop {
            interval.tick().await;
            let Some(system) = weak.upgrade() else {
                break;
            };
            let now_ms = chrono::Utc::now().timestamp_millis();
            let system_for_tick = Arc::clone(&system);
            let result = tokio::task::spawn_blocking(move || system_for_tick.tick_due(now_ms)).await;
            match result {
                Ok(Ok(report)) => {
                    last_error = None;
                    for action in report.actions {
                        run_due_action(&system, &hub, action).await;
                    }
                }
                Ok(Err(error)) => note_error(&mut last_error, format!("{error:#}")),
                Err(error) => note_error(&mut last_error, format!("scheduler worker failed: {error}")),
            }
        }
    }))
}

fn note_error(last_error: &mut Option<Instant>, error: String) {
    if last_error.is_none_or(|t| t.elapsed() >= Duration::from_secs(60)) {
        tracing::warn!("技能任务调度暂不可用，未投递、下轮重试：{error}");
        *last_error = Some(Instant::now());
    }
}

/// 直推一次到期动作并按结果写播报。三种落点：
/// - 直推成功 → 播任务原文（"已按时关闭电源"）；
/// - 直推失败（设备拒绝 / 发送失败 / 无确认）→ 播失败提示；
/// - 无通道（离线 / 测试台 / AIUI）/ 通道满 → 降级播原文提醒（旧固件语义不变）。
async fn run_due_action(system: &SkillSystem, hub: &DeviceCmdHub, due: DueAction) {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let step: Result<SkillStep, _> = serde_json::from_str(&due.action);
    let speech = match step.map(extract_tool_call) {
        Ok(Ok((tool, arguments))) => {
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            let sent = hub.send(
                &due.device_id,
                DeviceCmd::CallDeviceTool {
                    tool,
                    arguments,
                    reply: reply_tx,
                },
            );
            match sent {
                Ok(()) => match tokio::time::timeout(ACTION_CALL_TIMEOUT, reply_rx).await {
                    Ok(Ok(Ok(()))) => due.text.clone(),
                    Ok(Ok(Err(e))) => format!("{}（到期动作未执行：{e}）", due.text),
                    // 回执通道被丢弃 = 会话在调用中途结束（设备掉线 / 会话被取消）。
                    Ok(Err(_)) | Err(_) => {
                        format!("{}（到期动作未执行：设备未确认）", due.text)
                    }
                },
                // 无通道（离线）与通道满（忙）都降级为原文提醒——旧固件语义。
                Err(_) => due.text.clone(),
            }
        }
        // 创建时已过 validate_step，这里解析失败只可能是库内数据被手工改坏：
        // 按失败播报处理，不让坏行静默消失。
        _ => format!("{}（到期动作无法解析，未执行）", due.text),
    };
    match system.enqueue_task_speech(&due.task_id, &due.device_id, &speech, now_ms) {
        Ok(true) => {}
        Ok(false) => tracing::info!(
            "task {} 上一轮播报未投递，本轮播报跳过（动作已按结果执行/降级）",
            due.task_id
        ),
        Err(e) => tracing::warn!("task {} 播报写入失败: {e:#}", due.task_id),
    }
}

/// 从落库的 action JSON 取 `(tool, arguments)`（validate_step 已保证形状）。
fn extract_tool_call(step: SkillStep) -> anyhow::Result<(String, Value)> {
    anyhow::ensure!(step.api == "devices.call", "action must be devices.call");
    let tool = step
        .args
        .get("tool")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("action missing tool"))?
        .to_string();
    let arguments = step
        .args
        .get("arguments")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("action missing arguments"))?;
    Ok((tool, arguments))
}
