//! SSE 增量解析器（OpenAI Responses 流式协议）。
//!
//! [`SseParser`] 被喂入任意字节块，吐出完整 `data:` 行中解析出的文本增量。
//! 维护跨块的行缓冲（最后一行不完整时留存待下一块补齐），因此调用方无需关心
//! 网络分块边界——**整行解码**可避免多字节 UTF-8 被网络分块切断（String 逐块 lossy 会切坏）。
//!
//! 识别 Responses 事件：
//! - `response.output_text.delta` → 文本增量
//! - `response.output_item.added`（`item.type == "function_call"`）→ 按 `output_index` 注册工具调用
//! - `response.function_call_arguments.delta` → 按 `output_index` 追加入参片段
//! - `error` / `response.failed` → 记录错误（流结束后上报）

use serde_json::Value;

use crate::llm::ToolCall;

struct RawToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl Default for RawToolCall {
    fn default() -> Self {
        Self {
            id: None,
            name: None,
            arguments: String::new(),
        }
    }
}

/// SSE 增量解析器：被喂入任意字节块，吐出完整 `data:` 行中解析出的文本增量。
pub struct SseParser {
    // 字节缓冲：SSE 行边界（\n）是 ASCII，整行解码可避免多字节 UTF-8 被网络分块切断。
    buf: Vec<u8>,
    text: String,
    raw: Vec<RawToolCall>,
    error: Option<String>,
}

impl SseParser {
    pub(crate) fn new() -> Self {
        Self {
            buf: Vec::new(),
            text: String::new(),
            raw: Vec::new(),
            error: None,
        }
    }

    /// 喂入一个字节块，返回本次新增的文本增量列表（按出现顺序）。
    pub(crate) fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
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
    pub(crate) fn finish(self) -> (String, Vec<ToolCall>, Option<String>) {
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
}
