use super::{CapabilityMetadata, SkillCondition, SkillExecutionContext, SkillPlan, SkillStep};
use anyhow::{bail, ensure, Result};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub fn capabilities() -> Vec<CapabilityMetadata> {
    [
        ("tasks.create", "Persist a device-bound reminder", "sqlite", json!({"required":["text"],"one_of":["after","at","interval","hourly","daily","workdays","weekly","monthly","yearly"],"calendar_fields":{"hourly":"0..59 minute integer or string, e.g.20分","workdays":"clock; Monday-Friday only","weekly":{"weekday":"1..7 or weekday string","time":"clock"},"monthly":{"day":"1..31","time":"clock"},"yearly":{"month":"1..12","day":"1..31","time":"clock"}},"invalid_dates":"skip; never clamp","timezone":"UTC or Asia/Shanghai; default UTC","speech":"actual local date, time, frequency; countdown target","optional":["timezone"]})),
        ("tasks.list", "List this device's tasks", "sqlite", json!({"optional":{"limit":"1..100","offset":"0..10000"}})),
        ("tasks.get", "Read one device-bound task", "sqlite", json!({"required":["task_id"]})),
        ("tasks.cancel", "Cancel exact device-bound task and unsubmitted delivery; ambiguous text is rejected", "sqlite", json!({"one_of":["task_id","text"],"speech":"actual cancellation result or typed clarification error"})),
        ("messages.send", "Persist text in this device's outbox", "sqlite", json!({"required":["text"]})),
        ("devices.call", "Session dispatch to an advertised MCP tool", "session", json!({"required":["tool","arguments"],"tool":"static literal; no interpolation"})),
        ("sessions.end", "Session-scoped stop listening", "session", json!({})),
        ("replies.say", "Deferred receipt: session speaks only after all plan actions succeed", "session", json!({"required":["text"]})),
    ].into_iter().map(|(api, description, execution, mut args_schema)| {
        if matches!(api, "tasks.create" | "tasks.cancel" | "messages.send") {
            args_schema["idempotency"] = json!({"scope":"device_id + request_id","retention_days":7,"max_records_global":8192,"conflicting_payload":"rejected","request_id":"stable and unique per logical step"});
        }
        CapabilityMetadata { api, description, execution, args_schema }
    }).collect()
}

pub fn validate_plan(plan: &SkillPlan) -> Result<()> {
    ensure!(
        !plan.steps.is_empty() && plan.steps.len() <= 16,
        "plan requires 1..16 steps"
    );
    ensure!(plan.conditions.len() <= 16, "at most 16 conditions");
    ensure!(
        serde_json::to_vec(plan)?.len() <= 32768,
        "plan exceeds 32768 bytes"
    );
    for condition in &plan.conditions {
        match condition {
            SkillCondition::SlotPresent { slot } => validate_name(slot)?,
            SkillCondition::SlotEquals { slot, value } => {
                validate_name(slot)?;
                ensure!(value.len() <= 4096, "condition value too long");
            }
            SkillCondition::DeviceKind { value } => {
                validate_name(value)?;
            }
        }
    }
    for (index, step) in plan.steps.iter().enumerate() {
        validate_step(step)?;
        ensure!(
            step.api != "sessions.end" || index + 1 == plan.steps.len(),
            "sessions.end must be the final plan step"
        );
    }
    super::results::validate_plan_refs(plan)?;
    Ok(())
}

fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 128
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'),
        "invalid identifier: {name}"
    );
    Ok(())
}

pub fn validate_step(step: &SkillStep) -> Result<()> {
    ensure!(
        serde_json::to_vec(&step.args)?.len() <= 32768,
        "step args exceeds 32768 bytes"
    );
    let args = step
        .args
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("step args must be an object"))?;
    let (required, allowed): (&[&str], &[&str]) = match step.api.as_str() {
        "tasks.create" => (
            &["text"],
            &[
                "text", "after", "at", "interval", "hourly", "daily", "workdays", "weekly",
                "monthly", "yearly", "timezone",
            ],
        ),
        "tasks.list" => (&[], &["limit", "offset"]),
        "tasks.get" => (&["task_id"], &["task_id"]),
        "tasks.cancel" => (&[], &["task_id", "text"]),
        "messages.send" | "replies.say" => (&["text"], &["text"]),
        "devices.call" => (&["tool", "arguments"], &["tool", "arguments"]),
        "sessions.end" => (&[], &[]),
        _ => bail!("unknown skill capability: {}", step.api),
    };
    for key in required {
        ensure!(args.contains_key(*key), "{} requires {key}", step.api);
    }
    for key in args.keys() {
        ensure!(
            allowed.contains(&key.as_str()),
            "unexpected argument {key} for {}",
            step.api
        );
    }
    if step.api == "tasks.create" {
        ensure!(
            [
                "after", "at", "interval", "hourly", "daily", "workdays", "weekly", "monthly",
                "yearly"
            ]
            .iter()
            .filter(|k| args.contains_key(**k))
            .count()
                == 1,
            "tasks.create requires exactly one schedule mode"
        );
        super::calendar::validate_args(&step.args)?;
    }
    if step.api == "tasks.cancel" {
        ensure!(
            args.contains_key("task_id") != args.contains_key("text"),
            "tasks.cancel requires exactly one of task_id or exact text"
        );
    }
    for key in [
        "text", "task_id", "after", "at", "interval", "daily", "workdays", "timezone", "tool",
    ] {
        if let Some(value) = args.get(key) {
            let text = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("{key} must be a string"))?;
            let max = if key == "text" { 2000 } else { 256 };
            ensure!(
                !text.trim().is_empty() && text.chars().count() <= max && !text.contains('\0'),
                "{key} must contain 1..{max} characters without NUL"
            );
        }
    }
    for (key, max, min) in [("limit", 100, 1), ("offset", 10000, 0)] {
        if let Some(value) = args.get(key) {
            let number = value
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("{key} must be a nonnegative integer"))?;
            ensure!((min..=max).contains(&number), "invalid {key}");
        }
    }
    if let Some(tool) = args.get("tool") {
        let tool = tool.as_str().unwrap_or_default();
        ensure!(
            !tool.contains('{') && !tool.contains('}') && tool.len() <= 128,
            "devices.call tool must be a static literal"
        );
        validate_name(tool)?;
        ensure!(
            args.get("arguments").is_some_and(Value::is_object),
            "devices.call arguments must be an object"
        );
    }
    let mut nodes = 0;
    validate_tree(&step.args, 0, &mut nodes)?;
    super::results::validate_step_refs(step)?;
    Ok(())
}

pub(super) fn ensure_resolved(value: &Value) -> Result<()> {
    match value {
        Value::String(text) => ensure!(
            !text.contains("{{") && !text.contains("}}"),
            "unresolved placeholders cannot be executed"
        ),
        Value::Object(map) => {
            for child in map.values() {
                ensure_resolved(child)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                ensure_resolved(child)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_tree(value: &Value, depth: usize, nodes: &mut usize) -> Result<()> {
    *nodes += 1;
    ensure!(
        depth <= 12 && *nodes <= 1024,
        "args JSON exceeds depth/node limits"
    );
    match value {
        Value::String(text) => {
            ensure!(text.len() <= 8192, "argument string too long");
            // Validation only: empty dummy slots ensure reference names are checked, not resolved.
            walk_refs(text, |reference| {
                validate_reference(reference)?;
                Ok(format!("{{{{{reference}}}}}"))
            })?;
        }
        Value::Object(map) => {
            for (key, child) in map {
                ensure!(
                    !key.contains("{{") && key.len() <= 256,
                    "argument keys cannot be interpolated"
                );
                validate_tree(child, depth + 1, nodes)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                validate_tree(child, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_reference(reference: &str) -> Result<()> {
    if super::results::reference_index(reference)?.is_some() {
        return Ok(());
    }
    if let Some(slot) = reference.strip_prefix("slots.") {
        return validate_name(slot);
    }
    ensure!(
        reference == "ctx.device_id" || reference == "ctx.session_id",
        "unknown reference: {reference}"
    );
    Ok(())
}

pub(super) fn walk_refs(
    text: &str,
    mut resolve: impl FnMut(&str) -> Result<String>,
) -> Result<String> {
    let mut rest = text;
    let mut output = String::new();
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let end = tail
            .find("}}")
            .ok_or_else(|| anyhow::anyhow!("unclosed placeholder"))?;
        let reference = &tail[..end];
        validate_reference(reference)?;
        output.push_str(&resolve(reference)?);
        rest = &tail[end + 2..];
    }
    ensure!(!rest.contains("}}"), "unmatched placeholder close");
    output.push_str(rest);
    ensure!(output.len() <= 8192, "resolved argument too long");
    Ok(output)
}

/// Byte spans distinguish original references from identical tokens assembled by interpolation.
fn reference_spans(text: &str) -> Result<Vec<(usize, usize, String)>> {
    let mut spans = vec![];
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find("{{") {
        let start = cursor + relative;
        ensure!(
            !text[cursor..start].contains("}}"),
            "unmatched placeholder close"
        );
        let tail = &text[start + 2..];
        let close = tail
            .find("}}")
            .ok_or_else(|| anyhow::anyhow!("unclosed placeholder"))?;
        let reference = &tail[..close];
        validate_reference(reference)?;
        cursor = start + 2 + close + 2;
        spans.push((start, cursor, reference.to_owned()));
    }
    ensure!(
        !text[cursor..].contains("}}"),
        "unmatched placeholder close"
    );
    Ok(spans)
}

fn resolve_text(
    text: &str,
    slots: &HashMap<String, String>,
    ctx: Option<&SkillExecutionContext>,
) -> Result<String> {
    let mut output = String::new();
    let mut expected = vec![];
    let mut cursor = 0;
    for (start, end, reference) in reference_spans(text)? {
        output.push_str(&text[cursor..start]);
        let replacement = if let Some(name) = reference.strip_prefix("slots.") {
            let value = slots
                .get(name)
                .ok_or_else(|| anyhow::anyhow!("missing slot: {name}"))?;
            ensure!(
                !value.contains("{{") && !value.contains("}}"),
                "slot values cannot introduce placeholders"
            );
            value.clone()
        } else if let Some(ctx) = ctx.filter(|_| reference.starts_with("ctx.")) {
            match reference.as_str() {
                "ctx.device_id" => ctx.device_id.clone(),
                "ctx.session_id" => ctx.session_id.clone(),
                _ => anyhow::bail!("unknown context reference"),
            }
        } else {
            // None preserves original ctx+steps; Some preserves only original steps.
            let token = format!("{{{{{reference}}}}}");
            expected.push((output.len(), output.len() + token.len(), reference));
            token
        };
        output.push_str(&replacement);
        cursor = end;
    }
    output.push_str(&text[cursor..]);
    ensure!(output.len() <= 8192, "resolved argument too long");
    ensure!(
        reference_spans(&output)? == expected,
        "interpolation cannot assemble or alter placeholder references"
    );
    Ok(output)
}

fn resolve_value(
    value: &Value,
    slots: &HashMap<String, String>,
    ctx: Option<&SkillExecutionContext>,
) -> Result<Value> {
    Ok(match value {
        Value::String(text) => Value::String(resolve_text(text, slots, ctx)?),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| resolve_value(v, slots, ctx))
                .collect::<Result<_>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), resolve_value(v, slots, ctx)?)))
                .collect::<Result<Map<_, _>>>()?,
        ),
        other => other.clone(),
    })
}

pub fn resolve_plan(plan: &SkillPlan, slots: &HashMap<String, String>) -> Result<SkillPlan> {
    validate_plan(plan)?;
    let mut plan = plan.clone();
    for step in &mut plan.steps {
        step.args = resolve_value(&step.args, slots, None)?;
    }
    validate_plan(&plan)?;
    Ok(plan)
}

pub fn resolve_step(
    step: &SkillStep,
    ctx: &SkillExecutionContext,
    slots: &HashMap<String, String>,
) -> Result<SkillStep> {
    ctx.check()?;
    validate_step(step)?;
    let step = SkillStep {
        api: step.api.clone(),
        args: resolve_value(&step.args, slots, Some(ctx))?,
    };
    validate_step(&step)?;
    Ok(step)
}

pub fn conditions_match(
    plan: &SkillPlan,
    slots: &HashMap<String, String>,
    device_kind: Option<&str>,
) -> bool {
    plan.conditions.iter().all(|condition| match condition {
        SkillCondition::SlotPresent { slot } => slots
            .get(slot)
            .is_some_and(|value| !value.trim().is_empty()),
        SkillCondition::SlotEquals { slot, value } => slots.get(slot) == Some(value),
        SkillCondition::DeviceKind { value } => device_kind == Some(value.as_str()),
    })
}
