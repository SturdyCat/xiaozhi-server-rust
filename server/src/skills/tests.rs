use super::*;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;

struct TestDb {
    root: PathBuf,
    system: SkillSystem,
}
impl TestDb {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("skills-test-{}", Uuid::new_v4()));
        Self {
            system: SkillSystem::new(root.join("skills.db")),
            root,
        }
    }
}
impl Drop for TestDb {
    fn drop(&mut self) {
        if self.root.starts_with(std::env::temp_dir())
            && self
                .root
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("skills-test-"))
        {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}
fn ctx(device: &str, request: &str, now_ms: i64) -> SkillExecutionContext {
    SkillExecutionContext {
        device_id: device.into(),
        session_id: "session-1".into(),
        request_id: request.into(),
        device_kind: Some("esp32".into()),
        now_ms,
        cancelled: Arc::new(AtomicBool::new(false)),
    }
}
fn create(system: &SkillSystem, context: &SkillExecutionContext, args: Value) -> String {
    system.execute_api(context, "tasks.create", &args).unwrap()["task"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}
fn pending_count(system: &SkillSystem) -> i64 {
    system
        .connect()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM skills_outbox WHERE state IN ('pending','leased')",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[path = "tests_action.rs"]
mod action;
#[path = "tests_calendar.rs"]
mod calendar;
#[path = "tests_limits.rs"]
mod limits;
#[path = "tests_plan.rs"]
mod plan;
#[path = "tests_preflight.rs"]
mod preflight;
#[path = "tests_results.rs"]
mod results;
#[path = "tests_scheduler.rs"]
mod scheduler;
#[path = "tests_time.rs"]
mod time;
