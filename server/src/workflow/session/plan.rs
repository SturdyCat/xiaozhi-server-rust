//! Session-pinned executable hook selection. Missing context never matches.
use super::{DeviceKind, Session};
use crate::plugins::skill::SkillRegistry;
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
                "command.gate" | "nlu.rules" | "skill.gate"
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

    pub(super) fn text_hook_bindings(&self, text: &str) -> Vec<CompiledBinding> {
        // Fixed stage order; order only reorders bindings within one position.
        let mut selected = self.hook_bindings("link.asr.nlu", Some(text));
        selected.extend(self.hook_bindings("nlu", Some(text)));
        selected
    }

    pub(super) fn turn_skills(&self, text: &str) -> SkillRegistry {
        let selected = self.text_hook_bindings(text);
        let mut command = self.eff_config.command.clone();
        let mut nlu = self.eff_config.nlu.clone();
        command.enabled &= selected
            .iter()
            .any(|b| matches!(b.plugin_id.as_str(), "command.gate" | "skill.gate"));
        nlu.enabled &= selected
            .iter()
            .any(|b| matches!(b.plugin_id.as_str(), "nlu.rules" | "skill.gate"));
        SkillRegistry::new(&command, &nlu, self.skills_bert.clone())
    }
}

#[cfg(test)]
mod tests;
