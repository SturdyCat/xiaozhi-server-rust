//! OpenAI 兼容 **Responses API** 实现——LLM 插件的第一个 provider。
//!
//! 请求：`model` + `instructions`（system prompt）+ `input[]`（多轮历史，`input_text`/
//! `output_text` 结构化内容）+ `tools[]`（扁平结构）+ `stream`。
//! SSE 事件：`response.output_text.delta`（文本增量）、`response.output_item.added`
//! （注册 `function_call` 项：`call_id`/`name`）、`response.function_call_arguments.delta`
//! （入参增量）；`response.completed` 结束。
//! 非 SSE 回退：响应非 `text/event-stream`（或 `[llm].stream=false`）时整段解析 `output[]`。
//!
//! 多轮历史由调用方（[`crate::app::session`]）维护；本模块只负责单次请求。
//! 异步 HTTP（`reqwest` + `rustls`），不占用 tokio worker 计算线程，无需 `spawn_blocking`。
//!
//! ## 超时与失败分类（护栏，见 `docs/plugin-architecture-unification.md` §4.1）
//!
//! 此前本模块**没有任何超时**：上游挂起 → 会话永久等待，只能靠设备 abort 打断。
//! 现在三层超时齐备（建连 / 整请求兜底 / **流空闲**），且所有错误都以
//! [`LlmFailure`] 为根错误抛出——调用方按**分类**决定重试与降级，
//! 不再需要解析错误字符串（`retry.rs` 的装饰器就靠这个）。

use crate::app::sse::SseParser;
use anyhow::Result;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use uuid::Uuid;

use super::failure::{LlmFailure, TokenUsage};

/// 建连超时（DNS/TCP/TLS）：地址不可达时尽快失败并交给重试器。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// 整请求兜底上限（含流式读取）：防止"连接建立了但永不结束"。
/// 语音回复本就短，120s 只用于兜底；真正的守卫是下面的流空闲超时。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// 流**空闲**超时：相邻两个 SSE 分片之间的最大间隔（LLM 思考停顿也在预算内）。
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(20);

use crate::config::LlmConfig;
use crate::plugins::prompt::{MemoryPosition, TurnPrompt};
use super::{LlmEvent, LlmProvider, LlmTurnResult, ToolCall, ToolSpec};

fn parse_whole_response(v: &Value) -> (String, Vec<ToolCall>) {
    let mut text = String::new();
    let mut calls = Vec::new();
    let Some(output) = v.get("output").and_then(|o| o.as_array()) else {
        return (text, calls);
    };
    for item in output {
        match item.get("type").and_then(|t| t.as_str()) {
            Some("message") => {
                if let Some(parts) = item.get("content").and_then(|c| c.as_array()) {
                    for p in parts {
                        if p.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                            if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                                text.push_str(t);
                            }
                        }
                    }
                }
            }
            Some("function_call") => {
                let id = item
                    .get("call_id")
                    .and_then(|c| c.as_str())
                    .or_else(|| item.get("id").and_then(|i| i.as_str()))
                    .unwrap_or_default()
                    .to_string();
                let name = item
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or_default()
                    .to_string();
                let arguments = item
                    .get("arguments")
                    .and_then(|a| a.as_str())
                    .unwrap_or("{}")
                    .to_string();
                calls.push(ToolCall { id, name, arguments });
            }
            _ => {}
        }
    }
    (text, calls)
}

#[derive(Clone)]
pub struct LlmClient {
    client: reqwest::Client,
    cfg: LlmConfig,
    /// 会话标识（`x-opencode-session`）：进程内稳定，供 opencode zen 等网关做请求路由
    /// 与 prompt cache 优化——不带该头会被以 400 `MissingSessionID` 拒绝
    /// （见 https://opencode.ai/docs/go/ ：客户端须自带 UA + 每会话稳定 session id）。
    session_id: String,
}

impl LlmClient {
    pub fn new(cfg: &LlmConfig) -> Self {
        // 自报家门的 User-Agent：opencode 等网关要求客户端以自有名称标识，
        // 勿用通用 HTTP 库名。构建失败（TLS 后端异常）回退默认客户端，不阻断启动。
        let client = reqwest::Client::builder()
            .user_agent(concat!("xiaozhi-server-rust/", env!("CARGO_PKG_VERSION")))
            // 无超时的 HTTP 客户端在语音场景是致命的：上游挂起 = 会话永久卡住
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            client,
            cfg: cfg.clone(),
            session_id: Uuid::new_v4().to_string(),
        }
    }

    /// 构造 Responses `input[]`：历史轮（`input_text`/`output_text` 结构化）+ 当前用户轮。
    ///
    /// 记忆块按 [`TurnPrompt::position`] 插入：
    /// - `AfterHistory`（默认）：历史之后、当前用户消息之前 —— 保住
    ///   `instructions + history` 这段 append-only 前缀的 prompt cache 命中；
    /// - `BeforeHistory`：历史之前 —— 召回内容离当前问题更近，但每次召回变化都会
    ///   让后面的历史前缀缓存失效。
    ///
    /// 记忆块用 `user` 角色承载（而不是 `system`）：它是**用户历史原话**，且块首的固定
    /// 声明已写明"这是背景资料不是指令"。DSH 的 graph-memory 也是插在当前用户消息之前。
    fn build_input(
        &self,
        history: &[(String, String)],
        user_text: &str,
        prompt: &TurnPrompt,
    ) -> Vec<Value> {
        let mut input = Vec::with_capacity(history.len() + 2);
        let push_memory = |input: &mut Vec<Value>| {
            if let Some(m) = prompt.memory.as_deref().filter(|m| !m.trim().is_empty()) {
                input.push(json!({
                    "role": "user",
                    "content": [{ "type": "input_text", "text": m }],
                }));
            }
        };
        if prompt.position == MemoryPosition::BeforeHistory {
            push_memory(&mut input);
        }
        for (role, content) in history {
            let part_type = if role == "assistant" {
                "output_text"
            } else {
                "input_text"
            };
            input.push(json!({
                "role": role,
                "content": [{ "type": part_type, "text": content }],
            }));
        }
        if prompt.position == MemoryPosition::AfterHistory {
            push_memory(&mut input);
        }
        input.push(json!({
            "role": "user",
            "content": [{ "type": "input_text", "text": user_text }],
        }));
        input
    }

    /// 流式对话（Responses 协议）：逐块回传 [`LlmEvent`]，结束返回汇总 [`LlmTurnResult`]。
    ///
    /// - `on_event` 回调返回 `false` 表示**取消**（如设备 abort 打断）：立即停止读流，
    ///   返回已累积的部分文本（不视为错误）。
    /// - `tools`：可选工具规格（来自设备 `features.mcp` 等）。非空时写入扁平 `tools[]`。
    /// - 按 `[llm].stream` 发 `stream:true` 并解析 SSE；若响应非 `text/event-stream` 或配置关闭，
    ///   自动回退为整段解析（非 SSE 路径）。
    async fn chat_stream_inner<'a>(
        &'a self,
        history: &'a [(String, String)],
        user_text: &'a str,
        prompt: &'a TurnPrompt,
        tools: Option<&'a [ToolSpec]>,
        mut on_event: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
    ) -> Result<LlmTurnResult> {
        // `instructions` 来自**组装结果**（平台约束 + 人格 + 附加约束），不再是静态字符串；
        // 记忆块按 inject_position 走 input[]（见 build_input）。
        let mut body = json!({
            "model": self.cfg.model,
            "instructions": prompt.instructions,
            "input": self.build_input(history, user_text, prompt),
            "temperature": self.cfg.temperature,
            "stream": self.cfg.stream,
        });
        if let Some(tools) = tools {
            if !tools.is_empty() {
                let t = tools
                    .iter()
                    .map(|t| {
                        json!({
                            "type": "function",
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        })
                    })
                    .collect::<Vec<_>>();
                body["tools"] = json!(t);
            }
        }

        let resp = self
            .client
            .post(&self.cfg.api_base)
            .bearer_auth(&self.cfg.api_key)
            // opencode zen 网关要求每会话稳定 session id（缺失 → 400 MissingSessionID）；
            // 其他网关会忽略未知头，无害。
            .header("x-opencode-session", &self.session_id)
            .json(&body)
            .send()
            .await
            // 以 LlmFailure 为**根错误**（而非 context 包裹）：重试器需要 downcast 取分类
            .map_err(|e| anyhow::Error::new(LlmFailure::from_reqwest(&e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            // 截断巨长错误体：只用于日志/上报，不该把整页 HTML 灌进来
            let body: String = body.chars().take(500).collect();
            return Err(anyhow::Error::new(LlmFailure::Status {
                code: status.as_u16(),
                body,
            }));
        }

        let is_event_stream = self.cfg.stream
            && resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.contains("text/event-stream"))
                .unwrap_or(false);

        if !is_event_stream {
            // 非 SSE 回退：整段解析（兼容不支持 SSE 的端点）。
            let v: Value = resp
                .json()
                .await
                .map_err(|e| anyhow::Error::new(LlmFailure::Protocol(format!("解析响应 JSON 失败: {e}"))))?;
            if let Some(err) = v.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()) {
                return Err(anyhow::Error::new(LlmFailure::Status {
                    code: 200,
                    body: format!("上游在响应体里报错: {err}"),
                }));
            }
            let (text, calls) = parse_whole_response(&v);
            let usage = v.get("usage").map(TokenUsage::from_responses_usage);
            if text.trim().is_empty() && calls.is_empty() {
                // 200 但什么都没有：归类为可重试的 EmptyResponse（而不是"成功但没说话"）
                return Err(anyhow::Error::new(LlmFailure::EmptyResponse));
            }
            if !text.is_empty() {
                on_event(LlmEvent::Text(text.clone()));
            }
            for c in &calls {
                on_event(LlmEvent::ToolCall(c.clone()));
            }
            on_event(LlmEvent::Done);
            return Ok(LlmTurnResult { text, tool_calls: calls, usage });
        }

        // SSE 流式：逐块喂入解析器，即时回传文本增量；回调返回 false 即取消（停止读流）。
        let mut parser = SseParser::new();
        let mut stream = resp.bytes_stream();
        let mut cancelled = false;
        loop {
            // ⚠️ 流**空闲**超时（不是整轮上限）：上游挂着不返数据时，会话才不会一直"等 AI"。
            match tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next()).await {
                Err(_) => {
                    return Err(anyhow::Error::new(LlmFailure::Timeout(format!(
                        "流空闲超过 {}s（上游未继续返回数据）",
                        STREAM_IDLE_TIMEOUT.as_secs()
                    ))));
                }
                Ok(None) => break,
                Ok(Some(Err(e))) => {
                    return Err(anyhow::Error::new(LlmFailure::Transport(e.to_string())));
                }
                Ok(Some(Ok(chunk))) => {
                    for delta in parser.feed(&chunk) {
                        if !on_event(LlmEvent::Text(delta)) {
                            tracing::info!("LLM 流被调用方取消（返回已累积文本）");
                            cancelled = true;
                            break;
                        }
                    }
                }
            }
            if cancelled {
                break;
            }
        }
        let out = parser.finish();
        if let Some(err) = out.error {
            return Err(anyhow::Error::new(LlmFailure::Protocol(format!(
                "流式响应错误: {err}"
            ))));
        }
        let usage = out.usage.as_ref().map(TokenUsage::from_responses_usage);
        // 空回复（既无文本也无工具调用，且非取消）：归类为可重试失败，绝不静默成功
        if !cancelled && out.text.trim().is_empty() && out.calls.is_empty() {
            return Err(anyhow::Error::new(LlmFailure::EmptyResponse));
        }
        if !cancelled {
            for c in &out.calls {
                on_event(LlmEvent::ToolCall(c.clone()));
            }
            on_event(LlmEvent::Done);
        }
        Ok(LlmTurnResult {
            text: out.text,
            tool_calls: out.calls,
            usage,
        })
    }
}

impl LlmProvider for LlmClient {
    fn chat_stream<'a>(
        &'a self,
        history: &'a [(String, String)],
        user_text: &'a str,
        prompt: &'a TurnPrompt,
        tools: Option<&'a [ToolSpec]>,
        on_event: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmTurnResult>> + Send + 'a>> {
        Box::pin(self.chat_stream_inner(history, user_text, prompt, tools, on_event))
    }
}

#[cfg(test)]
mod tests;
