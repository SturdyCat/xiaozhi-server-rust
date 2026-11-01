//! Only already-successful task speech can flow into a later receipt, never into effects.
use super::{plan, SkillPlan, SkillStep};
use anyhow::{bail, ensure, Context, Result};
use serde_json::Value;

pub(super) fn reference_index(reference: &str) -> Result<Option<usize>> {
    let Some(rest) = reference.strip_prefix("steps.") else {
        return Ok(None);
    };
    let index = rest
        .strip_suffix(".speech")
        .context("step references may access only speech")?;
    ensure!(
        !index.is_empty() && index.len() <= 2 && index.chars().all(|c| c.is_ascii_digit()),
        "invalid result step index"
    );
    let number: usize = index.parse()?;
    ensure!(
        number < 16 && index == number.to_string(),
        "invalid result step index"
    );
    Ok(Some(number))
}

fn indices(text: &str) -> Result<Vec<usize>> {
    let mut indices = vec![];
    plan::walk_refs(text, |reference| {
        if let Some(index) = reference_index(reference)? {
            indices.push(index);
        }
        Ok(format!("{{{{{reference}}}}}"))
    })?;
    Ok(indices)
}

fn contains_reference(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains("{{steps."),
        Value::Array(items) => items.iter().any(contains_reference),
        Value::Object(map) => map.values().any(contains_reference),
        _ => false,
    }
}

pub(super) fn validate_step_refs(step: &SkillStep) -> Result<()> {
    if contains_reference(&step.args) {
        ensure!(
            step.api == "replies.say" && step.args["text"].is_string(),
            "step results are allowed only in replies.say.text"
        );
        indices(step.args["text"].as_str().unwrap_or_default())?;
    }
    Ok(())
}

pub(super) fn validate_plan_refs(plan: &SkillPlan) -> Result<()> {
    for (index, step) in plan.steps.iter().enumerate() {
        validate_step_refs(step)?;
        if step.api == "replies.say" {
            for source in indices(step.args["text"].as_str().unwrap_or_default())? {
                ensure!(
                    source < index,
                    "step result references must point backwards"
                );
                ensure!(
                    matches!(
                        plan.steps[source].api.as_str(),
                        "tasks.create" | "tasks.list" | "tasks.cancel"
                    ),
                    "only successful task create/list/cancel expose speech references"
                );
            }
        }
    }
    Ok(())
}

/// Called after slot/ctx resolution, using only successful results in original step order.
pub fn resolve_step_results(step: &SkillStep, results: &[Value]) -> Result<SkillStep> {
    plan::validate_step(step)?;
    let mut step = step.clone();
    if step.api == "replies.say" {
        let text = step.args["text"].as_str().context("receipt text missing")?;
        let resolved = plan::walk_refs(text, |reference| {
            let index =
                reference_index(reference)?.context("slot/ctx must resolve before step results")?;
            let speech = results
                .get(index)
                .and_then(|value| value.get("speech"))
                .and_then(Value::as_str)
                .context("referenced successful result has no speech")?;
            ensure!(
                !speech.is_empty() && speech.chars().count() <= 2000,
                "result speech exceeds limits"
            );
            ensure!(
                !speech.contains("{{") && !speech.contains("}}"),
                "result speech cannot introduce placeholders"
            );
            Ok(speech.into())
        })?;
        step.args["text"] = Value::String(resolved);
    } else if contains_reference(&step.args) {
        bail!("result cannot feed a side effect");
    }
    plan::validate_step(&step)?;
    plan::ensure_resolved(&step.args)?;
    Ok(step)
}
