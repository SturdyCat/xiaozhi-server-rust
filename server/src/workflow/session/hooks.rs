//! Optional context and completed-turn lifecycle hooks, separate from media IO.
use super::{plan::hook_result, Session};
use crate::plugins::prompt::TurnPrompt;
use crate::workflow::protocol::Transport;

impl<'a, T: Transport> Session<'a, T> {
    pub(super) async fn build_turn_prompt(
        &mut self,
        user_text: &str,
    ) -> anyhow::Result<TurnPrompt> {
        let cfg = self.eff_config.clone();
        let mut prompt = TurnPrompt {
            instructions: cfg.llm.system_prompt.clone(),
            memory: None,
            position: cfg.memory.inject_position(),
        };
        for binding in self.hook_bindings("link.nlu.llm", Some(user_text)) {
            let result = match binding.plugin_id.as_str() {
                "soul.profile" => {
                    crate::plugins::soul::instructions(&cfg, &self.device_id).map(|text| {
                        prompt.instructions = text;
                    })
                }
                "memory.recall" if cfg.memory.recall_enabled => {
                    let scope = cfg.memory.scope_for(&self.device_id);
                    let recalled = self.memory.recall(Some(&scope), user_text).await;
                    if let Some(error) = recalled.error {
                        Err(anyhow::anyhow!(error))
                    } else {
                        prompt.memory = recalled.block;
                        Ok(())
                    }
                }
                _ => Ok(()),
            };
            hook_result(&binding, result)?;
        }
        Ok(prompt)
    }

    /// Called once by the successful, non-cancelled local response lifecycle.
    /// Record is awaited so `stop` has real turn semantics; expensive extraction
    /// and maintenance stay off the response path. This is NOT crash exactly-once
    /// and a successful downlink is NOT a device playback acknowledgement.
    pub(super) async fn complete_memory_write(
        &mut self,
        turn_id: crate::observability::events::TurnId,
        user_text: &str,
        assistant_text: &str,
    ) -> anyhow::Result<()> {
        // Session turns execute serially and TurnIds increase monotonically.
        // Mark before attempting record: failures are not retried by re-entry.
        if self
            .memory_completed_through
            .is_some_and(|last| turn_id <= last)
        {
            return Ok(());
        }
        self.memory_completed_through = Some(turn_id);
        if !self.eff_config.memory.active() {
            return Ok(());
        }
        for binding in self.hook_bindings("end", Some(user_text)) {
            if binding.plugin_id != "memory.writeback" {
                continue;
            }
            let memory = self.memory.clone();
            let turn = crate::plugins::memory::CompletedTurn {
                scope: self.eff_config.memory.scope_for(&self.device_id),
                session_id: self.session_id.clone(),
                user_text: user_text.to_string(),
                assistant_text: assistant_text.to_string(),
            };
            let result = memory.record(turn).await;
            let recorded = result.is_ok();
            hook_result(&binding, result.map(|_| ()))?;
            if !recorded {
                continue;
            }
            let sid = self.session_id.clone();
            tokio::spawn(async move {
                let outcome = memory.extract_pending(1).await;
                if outcome.failed > 0 {
                    tracing::warn!("session {sid} 后台记忆抽取失败 {} 次", outcome.failed);
                }
                if let Err(error) = memory
                    .maintain(crate::plugins::memory::MaintainOpts::periodic())
                    .await
                {
                    tracing::warn!("session {sid} 后台记忆维护失败: {error:#}");
                }
            });
        }
        Ok(())
    }

    pub(super) async fn run_wake_hooks(&mut self, trigger: Option<&str>) -> anyhow::Result<()> {
        for binding in self.hook_bindings("start", trigger) {
            if binding.plugin_id == "wakeup.greeting" {
                let result = self.handle_wake_greeting().await;
                hook_result(&binding, result)?;
            }
        }
        Ok(())
    }
}
