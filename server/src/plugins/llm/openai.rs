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

use crate::app::sse::SseParser;
use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use uuid::Uuid;

use crate::config::LlmConfig;
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
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            client,
            cfg: cfg.clone(),
            session_id: Uuid::new_v4().to_string(),
        }
    }

    /// 构造 Responses `input[]`：历史轮（`input_text`/`output_text` 结构化）+ 当前用户轮。
    fn build_input(&self, history: &[(String, String)], user_text: &str) -> Vec<Value> {
        let mut input = Vec::with_capacity(history.len() + 1);
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
        tools: Option<&'a [ToolSpec]>,
        mut on_event: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
    ) -> Result<LlmTurnResult> {
        let mut body = json!({
            "model": self.cfg.model,
            "instructions": self.cfg.system_prompt,
            "input": self.build_input(history, user_text),
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
            .context("调用 LLM API 失败（网络或 URL 错误）")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("LLM API 返回错误 {status}: {body}");
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
            let v: Value = resp.json().await.context("解析 LLM 响应 JSON 失败")?;
            if let Some(err) = v.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()) {
                anyhow::bail!("LLM API 错误: {err}");
            }
            let (text, calls) = parse_whole_response(&v);
            if !text.is_empty() {
                on_event(LlmEvent::Text(text.clone()));
            }
            for c in &calls {
                on_event(LlmEvent::ToolCall(c.clone()));
            }
            on_event(LlmEvent::Done);
            return Ok(LlmTurnResult { text, tool_calls: calls });
        }

        // SSE 流式：逐块喂入解析器，即时回传文本增量；回调返回 false 即取消（停止读流）。
        let mut parser = SseParser::new();
        let mut stream = resp.bytes_stream();
        let mut cancelled = false;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("读取 LLM 流失败")?;
            for delta in parser.feed(&chunk) {
                if !on_event(LlmEvent::Text(delta)) {
                    tracing::info!("LLM 流被调用方取消（返回已累积文本）");
                    cancelled = true;
                    break;
                }
            }
            if cancelled {
                break;
            }
        }
        let (text, calls, error) = parser.finish();
        if let Some(err) = error {
            anyhow::bail!("LLM 流式响应错误: {err}");
        }
        if !cancelled {
            for c in &calls {
                on_event(LlmEvent::ToolCall(c.clone()));
            }
            on_event(LlmEvent::Done);
        }
        Ok(LlmTurnResult { text, tool_calls: calls })
    }
}

impl LlmProvider for LlmClient {
    fn chat_stream<'a>(
        &'a self,
        history: &'a [(String, String)],
        user_text: &'a str,
        tools: Option<&'a [ToolSpec]>,
        on_event: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmTurnResult>> + Send + 'a>> {
        Box::pin(self.chat_stream_inner(history, user_text, tools, on_event))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_response_message_and_function_call() {
        let v = json!({
            "id": "resp_1",
            "status": "completed",
            "output": [
                { "type": "reasoning", "summary": [] },
                { "type": "message", "role": "assistant",
                  "content": [ { "type": "output_text", "text": "今天天气不错", "annotations": [] } ] },
                { "type": "function_call", "id": "fc_x", "call_id": "call_x",
                  "name": "get_weather", "arguments": "{\"city\":\"BJ\"}" }
            ]
        });
        let (text, calls) = parse_whole_response(&v);
        assert_eq!(text, "今天天气不错");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_x");
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments, "{\"city\":\"BJ\"}");
    }

    #[test]
    fn whole_response_no_output() {
        let v = json!({ "id": "resp_2", "status": "completed", "output": [] });
        let (text, calls) = parse_whole_response(&v);
        assert!(text.is_empty());
        assert!(calls.is_empty());
    }

    #[test]
    fn build_input_maps_history_roles() {
        let cfg = LlmConfig {
            api_base: "https://example.com/v1/responses".into(),
            api_key: String::new(),
            model: "gpt-4o".into(),
            system_prompt: "你是一个助手。".into(),
            max_history: 10,
            temperature: 0.7,
            stream: true,
        };
        let client = LlmClient::new(&cfg);
        let history = vec![
            ("user".to_string(), "你好".to_string()),
            ("assistant".to_string(), "你好呀".to_string()),
        ];
        let input = client.build_input(&history, "天气如何");
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[1]["content"][0]["type"], "output_text");
        assert_eq!(input[2]["content"][0]["text"], "天气如何");
    }
}
