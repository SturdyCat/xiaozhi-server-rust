//! Stable declarative skills and device-bound persistent scheduling.
//! No networking, device playback or arbitrary code runs in this module.
mod api;
pub(super) mod weather_ops;
mod calendar;
mod delivery;
mod plan;
mod preflight;
mod results;
mod schedule;
mod store;
mod task_ops;
mod types;

pub use plan::{
    capabilities, conditions_match, resolve_plan, resolve_step, validate_plan, validate_step,
};
pub use preflight::preflight_step_args;
pub use results::resolve_step_results;
pub use types::{
    CapabilityMetadata, DeliveryClaim, SkillActionError, SkillCondition, SkillExecutionContext,
    SkillPlan, SkillStep,
};
// TickReport 是 `tick_due` 的公开返回类型（外部调用方需要能命名它），本 crate 内
// 无人按名引用它，属已知的 re-export 形态，不是漏用。
#[allow(unused_imports)]
pub use delivery::{DueAction, TickReport};

use anyhow::{bail, Result};
use serde_json::Value;
use std::{path::PathBuf, sync::atomic::Ordering};

/// File path only: construction has no filesystem effects; operations lazily open SQLite.
#[derive(Debug, Clone)]
pub struct SkillSystem {
    path: PathBuf,
}

impl SkillSystem {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Blocking SQLite operation. Async callers use spawn_blocking.
    pub fn execute_step(&self, ctx: &SkillExecutionContext, step: &SkillStep) -> Result<Value> {
        self.execute_api(ctx, &step.api, &step.args)
    }

    pub fn execute_api(
        &self,
        ctx: &SkillExecutionContext,
        api: &str,
        args: &Value,
    ) -> Result<Value> {
        ctx.check()?;
        let step = SkillStep {
            api: api.into(),
            args: args.clone(),
        };
        validate_step(&step)?;
        plan::ensure_resolved(&step.args)?;
        api::execute(self, ctx, &step)
    }
}

impl SkillExecutionContext {
    pub fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            bail!("skill execution cancelled");
        }
        for (name, value) in [
            ("device_id", &self.device_id),
            ("session_id", &self.session_id),
            ("request_id", &self.request_id),
        ] {
            if value.trim().is_empty()
                || value.len() > 256
                || value.chars().any(char::is_control)
                || value.contains("{{")
                || value.contains("}}")
            {
                bail!("{name} must be nonempty and at most 256 bytes without control characters");
            }
        }
        if self.now_ms < 0 {
            bail!("now_ms must be nonnegative");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
