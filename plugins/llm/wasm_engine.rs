//! `llm.provider` 的 WASM 控制策略桥（P4）。
//!
//! WASM 只接收受限的本轮元数据并返回策略决策；OpenAI/原生 provider 仍负责
//! 实际 HTTP、SSE、工具调用和取消语义。策略插件不接触完整历史、TurnPrompt 或
//! 音频/大模型响应，避免把数据面搬进 WASM。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

use super::{Llm, LlmEvent, LlmProvider, LlmTurnResult, ToolSpec};
use crate::plugins::prompt::TurnPrompt;

const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_ROUTE_BYTES: usize = 64;

#[derive(Debug, Clone, Serialize)]
struct PrepareRequest<'a> {
    req_id: &'a str,
    user_text: &'a str,
    history_turns: usize,
    has_memory: bool,
    tool_names: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LlmPolicyDecision {
    #[serde(default)]
    pub user_text: Option<String>,
    #[serde(default)]
    pub route: Option<String>,
}

/// 以 WASM 策略包装原生 LLM provider。
///
/// `extension_point` 通常为 `llm.provider`。策略失败是显式错误；不会静默调用
/// 未经过策略的 native provider。未选择该包装器时，既有 OpenAI 路径完全不变。
pub struct WasmLlmPolicy {
    native: Llm,
    extension_point: &'static str,
    host_override: Option<Arc<crate::plugins::wasm::WasmHost>>,
}

impl WasmLlmPolicy {
    pub fn new(native: Llm) -> Self {
        Self {
            native,
            extension_point: "llm.provider",
            host_override: None,
        }
    }

    pub fn with_host(
        native: Llm,
        host: Arc<crate::plugins::wasm::WasmHost>,
        extension_point: &'static str,
    ) -> Self {
        Self {
            native,
            extension_point,
            host_override: Some(host),
        }
    }

    fn host(&self) -> Result<Arc<crate::plugins::wasm::WasmHost>> {
        self.host_override
            .clone()
            .or_else(crate::plugins::wasm::host_global)
            .context("WASM 插件平台未初始化（宿主缺失；见服务启动日志）")
    }

    fn plugin_id(&self, host: &crate::plugins::wasm::WasmHost) -> Result<String> {
        match host.resolve(self.extension_point) {
            crate::plugins::wasm::Resolution::Resolved(id) => Ok(id),
            resolution => anyhow::bail!(
                "LLM 策略插件未装载（扩展点 {} 无可用实现，resolution={resolution:?}）",
                self.extension_point
            ),
        }
    }

    fn prepare(
        host: &crate::plugins::wasm::WasmHost,
        plugin_id: &str,
        user_text: &str,
        history_turns: usize,
        has_memory: bool,
        tool_names: Vec<String>,
    ) -> Result<LlmPolicyDecision> {
        if user_text.len() > MAX_TEXT_BYTES {
            anyhow::bail!(
                "LLM 用户文本超长（{} > {MAX_TEXT_BYTES} bytes）",
                user_text.len()
            );
        }
        let req_id = uuid::Uuid::new_v4().to_string();
        let payload = serde_json::to_string(&PrepareRequest {
            req_id: &req_id,
            user_text,
            history_turns,
            has_memory,
            tool_names,
        })?;
        let errors = host
            .handle_event(plugin_id, "llm.prepare", &payload)
            .context("LLM 策略插件事件失败")?;
        if !errors.is_empty() {
            anyhow::bail!("LLM 策略效果执行失败：{errors:?}");
        }
        let bytes = host
            .kv()
            .get(plugin_id, &format!("llm-result:{req_id}"))
            .context("LLM 策略插件未返回结果（需要 kv 键 llm-result:<req_id>）")?;
        let decision: LlmPolicyDecision = serde_json::from_slice(&bytes)
            .context("LLM 策略结果损坏（需要 JSON {user_text?,route?}）")?;
        if let Some(text) = &decision.user_text {
            if text.len() > MAX_TEXT_BYTES {
                anyhow::bail!(
                    "LLM 策略文本超长（{} > {MAX_TEXT_BYTES} bytes）",
                    text.len()
                );
            }
        }
        if let Some(route) = &decision.route {
            if route.len() > MAX_ROUTE_BYTES {
                anyhow::bail!(
                    "LLM 策略 route 超长（{} > {MAX_ROUTE_BYTES} bytes）",
                    route.len()
                );
            }
            if route != "native" {
                anyhow::bail!("LLM 策略返回不支持的 route={route:?}（当前仅允许 native）");
            }
        }
        Ok(decision)
    }
}

impl LlmProvider for WasmLlmPolicy {
    fn chat_stream<'a>(
        &'a self,
        history: &'a [(String, String)],
        user_text: &'a str,
        prompt: &'a TurnPrompt,
        tools: Option<&'a [ToolSpec]>,
        on_event: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<LlmTurnResult>> + Send + 'a>>
    {
        let native = Arc::clone(&self.native);
        let host = self.host();
        let extension_point = self.extension_point;
        let history = history.to_vec();
        let user_text = user_text.to_string();
        let prompt = prompt.clone();
        let tool_names = tools
            .unwrap_or_default()
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        let history_turns = history.len();
        let has_memory = prompt.memory.is_some();
        let tools = tools.map(|items| items.to_vec());

        Box::pin(async move {
            let host = host?;
            let plugin_id = match host.resolve(extension_point) {
                crate::plugins::wasm::Resolution::Resolved(id) => id,
                resolution => anyhow::bail!(
                    "LLM 策略插件未装载（扩展点 {extension_point} 无可用实现，resolution={resolution:?}）"
                ),
            };
            let policy_text = user_text.clone();
            let decision = tokio::task::spawn_blocking(move || {
                Self::prepare(
                    &host,
                    &plugin_id,
                    &policy_text,
                    history_turns,
                    has_memory,
                    tool_names,
                )
            })
            .await
            .context("LLM 策略线程异常")??;
            let effective_text = decision.user_text.unwrap_or(user_text);
            native
                .chat_stream(
                    &history,
                    &effective_text,
                    &prompt,
                    tools.as_deref(),
                    on_event,
                )
                .await
        })
    }
}

/// 供测试/协议实现复用的最小响应形状。
pub fn decision_json(text: Option<&str>) -> String {
    json!({ "user_text": text, "route": "native" }).to_string()
}
