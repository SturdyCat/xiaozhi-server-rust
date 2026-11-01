//! Deterministic semantic checks before the first plan effect; never consults a clock or DB.
use super::{calendar, plan, schedule, SkillActionError, SkillStep};
use anyhow::Result;

pub(super) fn invalid_time() -> SkillActionError {
    SkillActionError { speech:"时间参数无法解析或已过期，请提供明确的日期和钟点、合法的循环日期或正时长；十二点请使用00:00或12:00。".into() }
}

/// Run on every slot/ctx-resolved step before executing any action in the plan.
/// Past but syntactically valid `at` values deliberately pass: an idempotent API retry
/// must be able to read its committed receipt before performing current-time checks.
pub fn preflight_step_args(step: &SkillStep) -> Result<()> {
    plan::validate_step(step)?;
    if step.api != "tasks.create" {
        return Ok(());
    }
    let check = || -> Result<()> {
        plan::ensure_resolved(&step.args)?;
        let args = &step.args;
        let timezone = args["timezone"].as_str().unwrap_or("UTC");
        schedule::offset(timezone)?;
        if let Some(value) = args.get("after").or_else(|| args.get("interval")) {
            schedule::duration_ms(value.as_str().unwrap_or_default())?;
        } else if let Some(value) = args.get("at") {
            schedule::parse_at_spec(value.as_str().unwrap_or_default())?;
        } else {
            calendar::parse(args, timezone)?;
        }
        Ok(())
    };
    check().map_err(|_| invalid_time().into())
}
