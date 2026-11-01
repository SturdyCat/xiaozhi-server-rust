//! Session-pinned executable hook selection. Missing context never matches.
use super::{DeviceKind, Session};
use crate::workflow::designer::{CompiledBinding, CompiledWorkflow, Condition};
use crate::workflow::protocol::Transport;

pub(super) struct HookContext<'a> {
    pub text: Option<&'a str>,
    pub device_kind: Option<&'a str>,
    pub confidence: Option<f32>,
}

pub(super) fn matches(condition: &Condition, ctx: &HookContext<'_>) -> bool {
    match condition {
        Condition::Always => true,
        Condition::TextContains {
            text,
            case_sensitive,
        } => ctx.text.is_some_and(|value| {
            if *case_sensitive {
                value.contains(text)
            } else {
                value.to_lowercase().contains(&text.to_lowercase())
            }
        }),
        Condition::DeviceKind { value } => ctx.device_kind.is_some_and(|kind| kind == value),
        Condition::Confidence { min, max } => ctx
            .confidence
            .is_some_and(|c| c.is_finite() && c >= *min && max.is_none_or(|max| c <= max)),
    }
}

pub(super) fn select(
    plan: &CompiledWorkflow,
    position: &str,
    ctx: &HookContext<'_>,
) -> Vec<CompiledBinding> {
    plan.plan
        .iter()
        .filter(|b| b.position_id == position && matches(&b.condition, ctx))
        .cloned()
        .collect()
}

pub(super) fn hook_result(
    binding: &CompiledBinding,
    result: anyhow::Result<()>,
) -> anyhow::Result<()> {
    if let Err(error) = result {
        if binding.on_error == "stop"
            || matches!(
                binding.plugin_id.as_str(),
                "nlu.model" | "command.gate" | "nlu.rules" | "skill.gate"
            )
        {
            return Err(error.context(format!("workflow {} stopped turn", binding.plugin_id)));
        }
        tracing::warn!(
            plugin = binding.plugin_id,
            "workflow hook continued after failure: {error:#}"
        );
    }
    Ok(())
}

impl<'a, T: Transport> Session<'a, T> {
    pub(super) fn hook_bindings(&self, position: &str, text: Option<&str>) -> Vec<CompiledBinding> {
        let kind = match self.params.kind {
            DeviceKind::XiaozhiEsp => "xiaozhi_esp",
            DeviceKind::EchoMuse => "echomuse",
        };
        // ASR/skill APIs expose no calibrated confidence. Never invent 1.0.
        select(
            &self.workflow,
            position,
            &HookContext {
                text,
                device_kind: Some(kind),
                confidence: None,
            },
        )
    }

    pub(super) fn model_gate_allows(
        &self,
        text: &str,
        confidence: Option<f32>,
        name: &str,
    ) -> bool {
        if !confidence.is_some_and(|c| c.is_finite() && (0.0..=1.0).contains(&c)) {
            return false;
        }
        self.model_gate_bindings(text, confidence, false)
            .iter()
            .any(|binding| match binding.plugin_id.as_str() {
                "command.gate" => self.eff_config.command.enabled && name == "end_session",
                "nlu.rules" => {
                    self.eff_config.nlu.enabled
                        && self
                            .eff_config
                            .nlu
                            .intents
                            .iter()
                            .any(|intent| intent.name == name)
                }
                "nlu.model" | "skill.gate" => true,
                _ => false,
            })
    }

    /// Legacy IDs select one shared model authorization gate. Confidence is
    /// evaluated only after that single decision, never by another inference.
    pub(super) fn model_gate_bindings(
        &self,
        text: &str,
        confidence: Option<f32>,
        before_model: bool,
    ) -> Vec<CompiledBinding> {
        let kind = match self.params.kind {
            DeviceKind::XiaozhiEsp => "xiaozhi_esp",
            DeviceKind::EchoMuse => "echomuse",
        };
        let ctx = HookContext {
            text: Some(text),
            device_kind: Some(kind),
            confidence,
        };
        self.workflow
            .plan
            .iter()
            .filter(|binding| {
                matches!(
                    binding.plugin_id.as_str(),
                    "nlu.model" | "command.gate" | "nlu.rules" | "skill.gate"
                ) && ((before_model && matches!(binding.condition, Condition::Confidence { .. }))
                    || matches(&binding.condition, &ctx))
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests;
