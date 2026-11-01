//! Durable skill scheduling is separate from intent recognition and device audio.
//! This worker only materializes due outbox entries; sessions own delivery/ack.
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::skills::SkillSystem;

/// Dropping the application lifetime guard cancels the timer task. In-flight SQLite
/// transactions finish atomically in the blocking pool; no device side effect runs here.
pub(crate) struct SchedulerGuard(tokio::task::JoinHandle<()>);

impl Drop for SchedulerGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) fn start(system: &Arc<SkillSystem>) -> SchedulerGuard {
    let weak = Arc::downgrade(system);
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
            let result = tokio::task::spawn_blocking(move || system.tick_due(now_ms)).await;
            let error = match result {
                Ok(Ok(_)) => None,
                Ok(Err(error)) => Some(format!("{error:#}")),
                Err(error) => Some(format!("scheduler worker failed: {error}")),
            };
            if let Some(error) = error {
                if last_error.is_none_or(|t| t.elapsed() >= Duration::from_secs(60)) {
                    tracing::warn!("技能任务调度暂不可用，未投递、下轮重试：{error}");
                    last_error = Some(Instant::now());
                }
            } else {
                last_error = None;
            }
        }
    }))
}
