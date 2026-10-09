//! `server/src/plugins/llm/openai.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

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
        engine: String::new(),
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
    let input = client.build_input(&history, "天气如何", &TurnPrompt::default());
    assert_eq!(input.len(), 3);
    assert_eq!(input[0]["content"][0]["type"], "input_text");
    assert_eq!(input[1]["content"][0]["type"], "output_text");
    assert_eq!(input[2]["content"][0]["text"], "天气如何");
}
