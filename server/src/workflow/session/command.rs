//! Model-only authorization and the sole session-bound skill side-effect exit.
//! Legacy workflow IDs are aliases, never rule/heuristic recognizers. One model
//! decision may authorize one invocation; the chat LLM has no execution path.
use std::sync::atomic::Ordering;

use super::Session;
use crate::plugins::skill::{Recognition, SkillAction, SkillInvocation};
use crate::workflow::mcp::STOP_LISTENING_TOOL;
use crate::workflow::protocol::Transport;

impl<'a, T: Transport> Session<'a, T> {
    /// true consumes this turn (including clarification and genuine failure).
    pub(super) async fn handle_skill_gate(&mut self, user_text: &str) -> bool {
        let bindings = self.model_gate_bindings(user_text, None, true);
        if bindings.is_empty() {
            return false;
        }
        let registry = self.skills.clone();
        let text = user_text.to_owned();
        let recognition = match self
            .skill_blocking(move || Ok(registry.recognize(&text)))
            .await
        {
            Ok(result) => result,
            Err(error) => {
                self.note_error(format!("model recognition task failed: {error}"));
                return true;
            }
        };
        if self.skill_cancelled() {
            return true;
        }
        match recognition {
            Recognition::Accepted { inv, confidence } => {
                if !self.model_gate_allows(user_text, Some(confidence), &inv.name) {
                    return false;
                }
                if let Err(error) = self.execute_invocation(&inv, "model").await {
                    let clarification = error
                        .downcast_ref::<crate::skills::SkillActionError>()
                        .map(|error| error.speech.clone());
                    self.note_error(format!("model-authorized skill failed: {error:#}"));
                    if let Some(clarification) = clarification {
                        if !self.skill_cancelled() {
                            if let Err(error) = self.speak_canned_in_turn(&clarification).await {
                                self.note_error(format!(
                                    "skill error clarification failed: {error:#}"
                                ));
                            }
                        }
                    }
                }
                true
            }
            Recognition::NeedsSlots {
                name,
                missing,
                confidence,
            } => {
                if !self.model_gate_allows(user_text, Some(confidence), &name) {
                    return false;
                }
                // No re-selection, LLM tool or invented slot values. Clarification
                // consumes this utterance without authorizing any action.
                let prompt = if missing.is_empty() {
                    format!("执行{name}的参数存在歧义，请说明完整信息。")
                } else {
                    format!(
                        "执行{name}还需要补充：{}。请说明完整信息。",
                        missing.join("、")
                    )
                };
                if let Err(error) = self.speak_canned_in_turn(&prompt).await {
                    self.note_error(format!("skill clarification failed: {error:#}"));
                }
                true
            }
            Recognition::Unavailable { reason } => {
                self.note_error(format!("model recognition unavailable: {reason}"));
                true
            }
            Recognition::Disabled | Recognition::Rejected => false,
        }
    }

    /// Check the existing transport abort/closed contract before *every* new
    /// effect, including after awaits. Never clear a same-turn cancellation.
    pub(super) fn skill_cancelled(&mut self) -> bool {
        if crate::workflow::downlink::poll_abort(self.transport, &self.session_id, &self.abort) {
            self.abort.store(true, Ordering::Relaxed);
        }
        self.abort.load(Ordering::Relaxed) || self.close_requested
    }

    pub(super) async fn execute_invocation(
        &mut self,
        inv: &SkillInvocation,
        source: &str,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!self.skill_cancelled(), "skill cancelled");
        match &inv.action {
            SkillAction::EndSession => {
                // Preserve farewell -> tts stop -> MCP, including all four legacy
                // unsupported/missing/unknown-tool/transport downgrade paths.
                let reply = if inv.reply.trim().is_empty() {
                    Ok(())
                } else {
                    self.speak_canned_in_turn(&inv.reply).await
                };
                if !self.skill_cancelled() {
                    self.stand_down_via_mcp().await;
                }
                reply?;
            }
            SkillAction::LocalCommand { kind } if kind == "reply" => {
                if !inv.reply.trim().is_empty() {
                    self.speak_canned_in_turn(&inv.reply).await?;
                }
            }
            SkillAction::LocalCommand { kind } => {
                let tool = kind
                    .strip_prefix("mcp:")
                    .ok_or_else(|| anyhow::anyhow!("invalid skill action: {kind}"))?;
                self.dispatch_device_tool(tool, inv).await?;
                // A receipt must never claim success before the device action.
                if !inv.reply.trim().is_empty() {
                    self.speak_canned_in_turn(&inv.reply).await?;
                }
            }
            SkillAction::Plan { plan } => {
                self.execute_skill_plan(plan, inv).await?;
                if !inv.reply.trim().is_empty() {
                    self.speak_canned_in_turn(&inv.reply).await?;
                }
            }
        }
        tracing::info!(
            "session {} model skill {} completed ({source})",
            self.session_id,
            inv.name
        );
        Ok(())
    }

    async fn dispatch_device_tool(
        &mut self,
        tool: &str,
        inv: &SkillInvocation,
    ) -> anyhow::Result<()> {
        let arguments = inv
            .slots
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        self.dispatch_skill_device_tool(tool, serde_json::Value::Object(arguments))
            .await
    }

    pub(super) async fn dispatch_skill_device_tool(
        &mut self,
        tool: &str,
        arguments: serde_json::Value,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!self.skill_cancelled(), "device action cancelled");
        anyhow::ensure!(
            self.engines.mcp.has_tool(&self.device_id, tool) == Some(true),
            "device tool unavailable without an advertised capability: {tool}"
        );
        match self.mcp_call_tool(tool, arguments).await {
            Ok(reply) if reply.succeeded() => Ok(()),
            Ok(reply) => anyhow::bail!("device rejected tool {tool}: {}", reply.summary()),
            Err(error) => anyhow::bail!("device tool {tool} failed: {error}"),
        }
    }

    /// Device stop preserves the original four-way MCP compatibility fallback.
    pub(super) async fn stand_down_via_mcp(&mut self) {
        if self.skill_cancelled() {
            return;
        }
        if self
            .engines
            .mcp
            .has_tool(&self.device_id, STOP_LISTENING_TOOL)
            == Some(false)
        {
            tracing::warn!(
                "session {} device lacks stop tool; close fallback",
                self.session_id
            );
            self.close_requested = true;
            return;
        }
        match self
            .mcp_call_tool(STOP_LISTENING_TOOL, serde_json::json!({}))
            .await
        {
            Ok(reply) if reply.succeeded() => {
                if let Some(vad) = self.vad.as_mut() {
                    vad.flush(&mut |_| {});
                }
            }
            Ok(reply) => {
                tracing::warn!(
                    "session {} device stop rejected; close fallback: {}",
                    self.session_id,
                    reply.summary()
                );
                self.close_requested = true;
            }
            Err(error) => {
                tracing::warn!(
                    "session {} device stop unavailable; close fallback: {error}",
                    self.session_id
                );
                self.close_requested = true;
            }
        }
    }
}
