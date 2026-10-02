//! 远程 LLM 调用（OpenAI 兼容 **Responses API**：`POST {api_base}`，默认 `/v1/responses`）。
//!
//! - [`LlmClient`]：真实 HTTP 调用（OpenAI Responses 协议），支持 **SSE 流式**与
//!   **原生工具调用**（扁平 `tools[].{type,name,description,parameters}` / `function_call` 项）。
//! - [`MockLlm`]：无模型回显实现，用于**零配置本地联调**（无需真实 LLM 接口，
//!   默认 `llm.backend = "mock"` 时启用，保证 `cargo run` 即可端到端跑通）。
//!
//! 多轮历史由 [`crate::session`] 维护；本模块只负责单次请求。
//!
//! 与 ASR/TTS/VAD 不同，`LlmClient` 是 **异步 HTTP 调用**（`reqwest` + `rustls`），
//! 不占用 tokio worker 计算线程，无需 `spawn_blocking` 隔离。它是唯一的非 CPU 密集引擎。
//!
//! ## 流式与工具调用（Responses 协议）
//! - 请求体：`model` + `instructions`（system prompt）+ `input[]`（多轮历史，
//!   `input_text`/`output_text` 结构化内容）+ `tools[]`（扁平结构）+ `stream`。
//! - SSE 事件：`response.output_text.delta`（文本增量）、`response.output_item.added`
//!   （注册 `function_call` 项：`call_id`/`name`）、`response.function_call_arguments.delta`
//!   （入参增量）；`response.completed` 结束。
//! - 非 SSE 回退：响应非 `text/event-stream`（或 `[llm].stream=false`）时整段解析
//!   `output[]`（`message` 项拼 `output_text`、`function_call` 项收集工具调用）。
//! - [`Llm::chat_stream`] 以回调逐块回传 [`LlmEvent`]；`ToolCall` 累加完整后才回传。

use crate::config::LlmConfig;
use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde_json::{json, Value};

/// 设备/服务端可向 LLM 声明的工具规格（Responses 扁平结构 `tools[]` 项）。
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema 对象（`parameters`），描述工具入参。
    pub parameters: Value,
}

/// 一次收集完整的工具调用（来自 LLM 的 `function_call` 项）。
///
/// `id` 对应 Responses 的 `call_id`（回填 `function_call_output.call_id` 用）。
/// 字段由阶段三（MCP 工具闭环）消费，当前仅 session 聚合路径不读。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// 工具入参的原始 JSON 字符串（如 `{"location":"Beijing"}`）。
    pub arguments: String,
}

/// 流式过程中回传给调用方的事件。
/// `Text`/`ToolCall` 载荷由阶段二（按句下发）/阶段三（MCP 闭环）消费。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum LlmEvent {
    /// 文本增量片段（`response.output_text.delta` 的 `delta`）。
    Text(String),
    /// 一个完整收集的工具调用（流结束后才回传）。
    ToolCall(ToolCall),
    /// 本轮流转结束。
    Done,
}

/// 一轮对话的汇总结果（已收齐全部文本与工具调用）。
#[derive(Debug, Clone, Default)]
pub struct LlmTurnResult {
    pub text: String,
    /// 阶段三（MCP 工具闭环）消费；当前聚合路径仅用 `text`。
    #[allow(dead_code)]
    pub tool_calls: Vec<ToolCall>,
}

/// SSE 增量解析器：被喂入任意字节块，吐出完整 `data:` 行中解析出的文本增量。
///
/// 维护跨块的行缓冲（最后一行不完整时留存待下一块补齐），因此调用方无需关心
/// 网络分块边界。识别 Responses 事件：
/// - `response.output_text.delta` → 文本增量
/// - `response.output_item.added`（`item.type == "function_call"`）→ 按 `output_index` 注册工具调用
/// - `response.function_call_arguments.delta` → 按 `output_index` 追加入参片段
/// - `error` / `response.failed` → 记录错误（流结束后上报）
struct SseParser {
    // 字节缓冲：SSE 行边界（\n）是 ASCII，整行解码可避免多字节 UTF-8 被网络分块切断。
    buf: Vec<u8>,
    text: String,
    raw: Vec<RawToolCall>,
    error: Option<String>,
}

#[derive(Default)]
struct RawToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl SseParser {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            text: String::new(),
            raw: Vec::new(),
            error: None,
        }
    }

    /// 喂入一个字节块，返回本次新增的文本增量列表（按出现顺序）。
    fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(chunk);
        let mut deltas = Vec::new();
        // 逐完整行处理；最后一行若不含换行则留在 buf 待续。
        while let Some(nl) = self.buf.iter().position(|&b| b == b'\n') {
            let line_bytes: Vec<u8> = self.buf.drain(..=nl).collect();
            let line = String::from_utf8_lossy(&line_bytes);
            let line = line.trim();
            if !line.starts_with("data:") {
                continue;
            }
            let payload = line["data:".len()..].trim_start();
            if payload == "[DONE]" {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(payload) else {
                continue;
            };
            match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                "response.output_text.delta" => {
                    if let Some(d) = v.get("delta").and_then(|d| d.as_str()) {
                        self.text.push_str(d);
                        deltas.push(d.to_string());
                    }
                }
                "response.output_item.added" => {
                    let is_fn_call = v
                        .get("item")
                        .and_then(|i| i.get("type"))
                        .and_then(|t| t.as_str())
                        == Some("function_call");
                    if !is_fn_call {
                        continue;
                    }
                    let index =
                        v.get("output_index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                    if self.raw.len() <= index {
                        self.raw.resize_with(index + 1, RawToolCall::default);
                    }
                    let entry = &mut self.raw[index];
                    if let Some(id) = v["item"]
                        .get("call_id")
                        .and_then(|c| c.as_str())
                        .or_else(|| v["item"].get("id").and_then(|i| i.as_str()))
                    {
                        entry.id = Some(id.to_string());
                    }
                    if let Some(name) = v["item"].get("name").and_then(|n| n.as_str()) {
                        entry.name = Some(name.to_string());
                    }
                }
                "response.function_call_arguments.delta" => {
                    let index =
                        v.get("output_index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                    if self.raw.len() <= index {
                        self.raw.resize_with(index + 1, RawToolCall::default);
                    }
                    if let Some(d) = v.get("delta").and_then(|d| d.as_str()) {
                        self.raw[index].arguments.push_str(d);
                    }
                }
                "error" => {
                    let msg = v
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("未知 LLM 流式错误");
                    self.error = Some(msg.to_string());
                }
                "response.failed" => {
                    let msg = v["response"]["error"]["message"]
                        .as_str()
                        .unwrap_or("LLM 响应失败");
                    self.error = Some(msg.to_string());
                }
                _ => {}
            }
        }
        deltas
    }

    /// 流结束：丢弃未完成的残余行，返回（完整文本, 终态工具调用, 错误）。
    fn finish(self) -> (String, Vec<ToolCall>, Option<String>) {
        let calls = self
            .raw
            .into_iter()
            .filter_map(|r| {
                let name = r.name?;
                Some(ToolCall {
                    id: r.id.unwrap_or_default(),
                    name,
                    arguments: r.arguments,
                })
            })
            .collect();
        (self.text, calls, self.error)
    }
}

/// 解析非流式（整段）Responses 响应：遍历 `output[]`，
/// `message` 项拼接 `content[].output_text`，`function_call` 项收集工具调用。
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
}

impl LlmClient {
    pub fn new(cfg: &LlmConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            cfg: cfg.clone(),
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
    pub async fn chat_stream(
        &self,
        history: &[(String, String)],
        user_text: &str,
        tools: Option<&[ToolSpec]>,
        mut on_event: impl FnMut(LlmEvent) -> bool,
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

/// 无模型回显实现，用于本地零配置联调。
pub struct MockLlm {
    reply: String,
}

impl MockLlm {
    pub fn new() -> Self {
        Self {
            reply: "这是本地 mock 语音助手的回复。".to_string(),
        }
    }

    /// 流式回显：一次性回传完整文案（模拟首字即达），便于联调时确认 ASR 链路。
    pub async fn chat_stream(
        &self,
        _history: &[(String, String)],
        user_text: &str,
        _tools: Option<&[ToolSpec]>,
        mut on_event: impl FnMut(LlmEvent) -> bool,
    ) -> Result<LlmTurnResult> {
        let text = if user_text.trim().is_empty() {
            self.reply.clone()
        } else {
            format!("{}（你说：{}）", self.reply, user_text)
        };
        let _ = on_event(LlmEvent::Text(text.clone()));
        let _ = on_event(LlmEvent::Done);
        Ok(LlmTurnResult {
            text,
            tool_calls: Vec::new(),
        })
    }
}

/// LLM 引擎枚举：真实 HTTP 或本地 mock。
pub enum Llm {
    Http(LlmClient),
    Mock(MockLlm),
}

impl Llm {
    /// 回调返回 `false` 表示取消（见 [`LlmClient::chat_stream`]）。
    pub async fn chat_stream(
        &self,
        history: &[(String, String)],
        user_text: &str,
        tools: Option<&[ToolSpec]>,
        on_event: impl FnMut(LlmEvent) -> bool,
    ) -> Result<LlmTurnResult> {
        match self {
            Llm::Http(c) => c.chat_stream(history, user_text, tools, on_event).await,
            Llm::Mock(m) => m.chat_stream(history, user_text, tools, on_event).await,
        }
    }
}

/// 依据配置构造 LLM 引擎：`backend = "mock"` 用本地回显，否则真实 HTTP。
pub fn build_llm(cfg: &LlmConfig) -> Llm {
    if cfg.is_mock() {
        return Llm::Mock(MockLlm::new());
    }
    Llm::Http(LlmClient::new(cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_multi_delta_concatenates_text() {
        let sse = concat!(
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"你好\"}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"世界\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{}}\n\n",
        );
        let mut parser = SseParser::new();
        let mut all = String::new();
        for chunk in sse.as_bytes().chunks(7) {
            for d in parser.feed(chunk) {
                all.push_str(&d);
            }
        }
        let (text, calls, error) = parser.finish();
        assert_eq!(text, "你好世界");
        assert_eq!(all, "你好世界");
        assert!(calls.is_empty());
        assert!(error.is_none());
    }

    #[test]
    fn sse_split_across_chunk_boundary() {
        // 一个 data 行被切成两块的边界情况。
        let part1 = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"AB";
        let part2 = "CD\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{}}\n\n";
        let mut parser = SseParser::new();
        let mut all = String::new();
        for d in parser.feed(part1.as_bytes()) {
            all.push_str(&d);
        }
        assert!(all.is_empty(), "未完成的行不应产出自增");
        for d in parser.feed(part2.as_bytes()) {
            all.push_str(&d);
        }
        let (text, _, _) = parser.finish();
        assert_eq!(text, "ABCD");
    }

    #[test]
    fn sse_function_call_assembled_from_events() {
        let sse = concat!(
            "data: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"get_weather\",\"arguments\":\"\"}}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":1,\"delta\":\"{\\\"loc\"}\n\n",
            "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":1,\"delta\":\"\\\":\\\"BJ\\\"}\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{}}\n\n",
        );
        let mut parser = SseParser::new();
        for chunk in sse.as_bytes().chunks(40) {
            parser.feed(chunk);
        }
        let (_text, calls, error) = parser.finish();
        assert!(error.is_none());
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments, "{\"loc\":\"BJ\"}");
    }

    #[test]
    fn sse_ignores_non_data_and_message_items() {
        let sse = concat!(
            ": keep-alive\n\n",
            "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"message\",\"role\":\"assistant\"}}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"嗨 \"}\n\n",
            "\r\n",
            "data: [DONE]\n\n",
        );
        let mut parser = SseParser::new();
        for chunk in sse.as_bytes().chunks(10) {
            parser.feed(chunk);
        }
        let (text, calls, _) = parser.finish();
        assert_eq!(text, "嗨 ");
        assert!(calls.is_empty(), "message 项不应被当成工具调用");
    }

    #[test]
    fn sse_captures_error_event() {
        let sse = "data: {\"type\":\"error\",\"message\":\"boom\"}\n\n";
        let mut parser = SseParser::new();
        parser.feed(sse.as_bytes());
        let (_, _, error) = parser.finish();
        assert_eq!(error.as_deref(), Some("boom"));
    }

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
            backend: "http".into(),
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

    #[tokio::test]
    async fn mock_llm_stream_emits_single_text() {
        let m = MockLlm::new();
        let mut events = Vec::new();
        let result = m
            .chat_stream(&[], "你好", None, |e| {
                events.push(e);
                true
            })
            .await
            .unwrap();
        assert!(matches!(events.first(), Some(LlmEvent::Text(_))));
        assert!(matches!(events.last(), Some(LlmEvent::Done)));
        assert!(result.text.contains("你好"));
    }
}
