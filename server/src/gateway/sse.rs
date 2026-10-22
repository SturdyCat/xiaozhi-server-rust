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
//! - `response.completed` → 记录 `response.usage`（**原始 JSON**，语义解释交给 `plugins/llm`）
//! - `error` / `response.failed` → 记录错误（流结束后上报）

use serde_json::Value;

use crate::plugins::llm::ToolCall;

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

/// 流结束时交给调用方的完整结果（用结构体而非元组：加字段不再破坏所有调用点）。
pub(crate) struct SseOutcome {
    pub text: String,
    pub calls: Vec<ToolCall>,
    pub error: Option<String>,
    /// `response.completed.response.usage` 的**原始 JSON**：本模块只做搬运，
    /// 令牌口径（含缓存桶/压力口径）由 [`crate::plugins::llm::TokenUsage`] 解释。
    pub usage: Option<Value>,
}

/// SSE 增量解析器：被喂入任意字节块，吐出完整 `data:` 行中解析出的文本增量。
pub struct SseParser {
    // 字节缓冲：SSE 行边界（\n）是 ASCII，整行解码可避免多字节 UTF-8 被网络分块切断。
    buf: Vec<u8>,
    text: String,
    raw: Vec<RawToolCall>,
    error: Option<String>,
    usage: Option<Value>,
}

impl SseParser {
    pub(crate) fn new() -> Self {
        Self {
            buf: Vec::new(),
            text: String::new(),
            raw: Vec::new(),
            error: None,
            usage: None,
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
                "response.completed" => {
                    // 用量在终态事件里；非流式路径由 openai.rs 直接读整段 JSON 的 usage
                    if let Some(u) = v.get("response").and_then(|r| r.get("usage")) {
                        if !u.is_null() {
                            self.usage = Some(u.clone());
                        }
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

    /// 流结束：丢弃未完成的残余行，返回完整结果（文本 / 工具调用 / 错误 / 用量）。
    pub(crate) fn finish(self) -> SseOutcome {
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
        SseOutcome {
            text: self.text,
            calls,
            error: self.error,
            usage: self.usage,
        }
    }
}

#[cfg(test)]
mod tests;
