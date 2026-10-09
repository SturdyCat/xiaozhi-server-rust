//! `server/src/app/sse.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

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
    let out = parser.finish();
    assert_eq!(out.text, "你好世界");
    assert_eq!(all, "你好世界");
    assert!(out.calls.is_empty());
    assert!(out.error.is_none());
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
    let out = parser.finish();
    assert_eq!(out.text, "ABCD");
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
    let out = parser.finish();
    assert!(out.error.is_none());
    assert_eq!(out.calls.len(), 1);
    assert_eq!(out.calls[0].id, "call_1");
    assert_eq!(out.calls[0].name, "get_weather");
    assert_eq!(out.calls[0].arguments, "{\"loc\":\"BJ\"}");
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
    let out = parser.finish();
    assert_eq!(out.text, "嗨 ");
    assert!(out.calls.is_empty(), "message 项不应被当成工具调用");
}

/// 用量在 `response.completed` 里：必须被原样带出（解释权在 plugins/llm）。
#[test]
fn sse_captures_usage_from_completed() {
    let sse = concat!(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":8}}}}\n\n",
    );
    let mut parser = SseParser::new();
    parser.feed(sse.as_bytes());
    let out = parser.finish();
    let usage = out.usage.expect("应捕获 usage");
    assert_eq!(usage["input_tokens"], 12);
    assert_eq!(usage["input_tokens_details"]["cached_tokens"], 8);
}

#[test]
fn sse_captures_error_event() {
    let sse = "data: {\"type\":\"error\",\"message\":\"boom\"}\n\n";
    let mut parser = SseParser::new();
    parser.feed(sse.as_bytes());
    let out = parser.finish();
    assert_eq!(out.error.as_deref(), Some("boom"));
}
