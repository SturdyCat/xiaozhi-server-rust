//! Bounded declarative actions; device identity is always the bound session.
use super::{DeviceKind, Session};
use crate::plugins::skill::SkillInvocation;
use crate::skills::{SkillExecutionContext, SkillPlan, SkillStep};
use crate::workflow::protocol::Transport;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(super) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

impl<T: Transport> Session<'_, T> {
    /// Keep observing cancellation while synchronous SQLite/model work runs.
    /// Join even after cancellation: do not detach potentially committing work.
    pub(super) async fn skill_blocking<R, F>(&mut self, work: F) -> anyhow::Result<R>
    where
        R: Send + 'static,
        F: FnOnce() -> anyhow::Result<R> + Send + 'static,
    {
        let mut job = tokio::task::spawn_blocking(work);
        loop {
            tokio::select! {
                result = &mut job => {
                    let result = result.map_err(|e| anyhow::anyhow!("skill worker failed: {e}"))?;
                    anyhow::ensure!(!self.skill_cancelled(), "skill work cancelled");
                    return result;
                }
                _ = tokio::time::sleep(Duration::from_millis(10)) => {
                    self.skill_cancelled();
                }
            }
        }
    }

    pub(super) fn skill_context(&self, index: usize) -> SkillExecutionContext {
        SkillExecutionContext {
            device_id: self.device_id.clone(),
            session_id: self.session_id.clone(),
            request_id: format!("{}:{}:{index}", self.session_id, self.skill_turn_id),
            device_kind: Some(
                match self.params.kind {
                    DeviceKind::XiaozhiEsp => "xiaozhi_esp",
                    DeviceKind::EchoMuse => "echomuse",
                }
                .into(),
            ),
            now_ms: now_ms(),
            cancelled: self.abort.clone(),
        }
    }

    pub(super) async fn execute_skill_plan(
        &mut self,
        plan: &SkillPlan,
        inv: &SkillInvocation,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!self.skill_cancelled(), "skill plan cancelled");
        crate::skills::validate_plan(plan)?;
        let context = self.skill_context(0);
        anyhow::ensure!(
            crate::skills::conditions_match(plan, &inv.slots, context.device_kind.as_deref()),
            "skill plan conditions not satisfied"
        );
        // Resolve and validate *all* parameters before the first effect. No
        // partial run caused by a missing/unknown placeholder in a later step.
        let steps: Vec<SkillStep> = plan
            .steps
            .iter()
            .map(|step| crate::skills::resolve_step(step, &context, &inv.slots))
            .collect::<anyhow::Result<_>>()?;
        for step in &steps {
            crate::skills::validate_step(step)?;
            // Pure deterministic syntax/range checks must precede *all* effects.
            // The API still owns now-dependent checks and persisted replay.
            crate::skills::preflight_step_args(step)?;
        }
        let mut receipts = Vec::new();
        let step_count = steps.len();
        let mut results = Vec::with_capacity(step_count);
        for (index, step) in steps.into_iter().enumerate() {
            // Only validated, backward service-speech references are evaluated
            // here; no result can select an API/tool or change device identity.
            let step = crate::skills::resolve_step_results(&step, &results)?;
            let mut result = serde_json::json!({});
            anyhow::ensure!(!self.skill_cancelled(), "skill plan cancelled");
            match step.api.as_str() {
                "devices.call" => {
                    let tool = step.args["tool"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing device tool"))?;
                    self.dispatch_skill_device_tool(tool, step.args["arguments"].clone())
                        .await?;
                }
                "sessions.end" => {
                    anyhow::ensure!(
                        index + 1 == step_count,
                        "sessions.end must be the last plan step"
                    );
                    self.stand_down_via_mcp().await;
                }
                "replies.say" => {
                    receipts.push(
                        step.args["text"]
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("missing reply text"))?
                            .to_owned(),
                    );
                }
                "tuya.control" => {
                    // NLU iot 域语音开关（架构 §1 D1）：set_dp 是异步 rustuya TCP
                    // 调用且持有 per-device tokio::Mutex，不能进 spawn_blocking
                    // （阻塞线程上无法 .await）。voice_control infallible——设备
                    // 未找到/歧义/离线等业务失败也返回 {"ok":false,"speech"}，
                    // 作为正常结果进入 results 供 replies.say 引用播报，会话延续。
                    let name = step.args["name"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing tuya device name"))?;
                    let action = step.args["action"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing tuya action"))?;
                    #[cfg(feature = "pure-base")]
                    {
                        let _ = (name, action);
                        result = serde_json::json!({
                            "ok": false,
                            "kind": "unavailable",
                            "speech": "当前纯底座构建未启用 Tuya 设备能力。"
                        });
                    }
                    #[cfg(not(feature = "pure-base"))]
                    {
                        #[cfg(feature = "wasm-plugins")]
                        {
                            if self.engines.host.config().tuya.engine_id() == "wasm-policy" {
                                if let Some(host) = self.engines.wasm.as_ref() {
                                    result = self
                                        .engines
                                        .tuya
                                        .voice_control_with_policy(host, name, action)
                                        .await;
                                } else {
                                    result = serde_json::json!({
                                        "ok": false,
                                        "kind": "error",
                                        "speech": "Tuya 控制策略宿主未装载，请检查 wasm-plugins 配置。"
                                    });
                                }
                            } else {
                                result = self.engines.tuya.voice_control(name, action).await;
                            }
                        }
                        #[cfg(not(feature = "wasm-plugins"))]
                        {
                            result = self.engines.tuya.voice_control(name, action).await;
                        }
                    }
                }
                _ => {
                    let system = self.engines.skill_system.clone();
                    let ctx = self.skill_context(index);
                    result = self
                        .skill_blocking(move || system.execute_step(&ctx, &step))
                        .await?;
                }
            }
            results.push(result);
            if self.close_requested {
                anyhow::ensure!(
                    index + 1 == step_count,
                    "plan closed before all steps completed"
                );
                break;
            }
        }
        // Persisted/system/device actions succeeded before any success receipt.
        for receipt in receipts {
            anyhow::ensure!(
                !self.abort.load(Ordering::Relaxed),
                "skill receipt cancelled"
            );
            self.speak_canned_in_turn(&receipt).await?;
        }
        Ok(())
    }
}
