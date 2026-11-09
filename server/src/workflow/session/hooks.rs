//! Optional context and completed-turn lifecycle hooks, separate from media IO.
use super::{plan::hook_result, Session};
use crate::plugins::prompt::TurnPrompt;
use crate::workflow::protocol::Transport;

/// 会话**观察者挂钩**派发（P2d）：绑定到 hook 位的 wasm 插件在此收
/// `hook.<position>` 事件（payload `{session_id, device_id, position, text?}`），
/// 返回效果由宿主**异步执行**（fire-and-forget），不阻塞对话轮、不回灌、
/// 任何失败只告警——旁路观察者永不打断轮次。调用方须在 spawn_blocking 内。
/// （阶段二收口：从 wasm 宿主 crate 的 bridge 归位到会话层——`hook.*` 是
/// 工作流域约定，宿主 crate 不持业务事件名。）
#[cfg(feature = "wasm-plugins")]
pub(crate) fn observe_hook(
    host: &crate::plugins::wasm::WasmHost,
    plugin_id: &str,
    position: &str,
    payload: &str,
) {
    let event = format!("hook.{position}");
    match host.try_dispatch(plugin_id, &event, payload) {
        Ok(Some(effects)) => host.spawn_deferred(plugin_id, effects),
        Ok(None) => {}
        Err(e) => tracing::warn!(
            plugin = plugin_id,
            position,
            "观察者挂钩失败（轮次继续）：{e:#}"
        ),
    }
}

impl<'a, T: Transport> Session<'a, T> {
    /// wasm **观察者挂钩**派发（P2d）：绑定到本位的 wasm 插件收
    /// `hook.<position>` 事件（payload `{session_id, device_id, position, text?}`）。
    /// fire-and-forget——spawn_blocking 内跑 wasm 调用，效果由宿主异步执行；
    /// **旁路观察者永不阻塞/打断对话轮**，失败只告警，不进 hook_result 错误流。
    fn dispatch_observer(&self, plugin_id: &str, position: &str, text: Option<&str>) {
        #[cfg(feature = "wasm-plugins")]
        if let Some(host) = self.engines.wasm.as_ref() {
            // 会话快照可能早于插件热卸载：派发前再核对装载目录（未装载静默跳过，
            // start.rs 已在编译期剔除当时缺失的绑定）
            if !host.observers().iter().any(|(id, _)| id == plugin_id) {
                return;
            }
            let payload = serde_json::json!({
                "session_id": self.session_id,
                "device_id": self.device_id,
                "position": position,
                "text": text,
            })
            .to_string();
            let host = std::sync::Arc::clone(host);
            let plugin_id = plugin_id.to_string();
            let position = position.to_string();
            tokio::task::spawn_blocking(move || {
                observe_hook(&host, &plugin_id, &position, &payload);
            });
        }
        #[cfg(not(feature = "wasm-plugins"))]
        {
            let _ = (plugin_id, position, text);
        }
    }

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
                _ => {
                    self.dispatch_observer(&binding.plugin_id, "link.nlu.llm", Some(user_text));
                    Ok(())
                }
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
                self.dispatch_observer(&binding.plugin_id, "end", Some(user_text));
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
            // 唤醒问候是 legacy 业务绑定；pure-base 空底座只派发 WASM 观察者。
            #[cfg(feature = "native-plugins")]
            if binding.plugin_id == "wakeup.greeting" {
                let result = self.handle_wake_greeting().await;
                hook_result(&binding, result)?;
                continue;
            }
            self.dispatch_observer(&binding.plugin_id, "start", trigger);
        }
        Ok(())
    }
}
