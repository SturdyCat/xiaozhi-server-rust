//! `server/src/plugins/llm/openai.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。
//!
//! ## 取消契约的回归测试（本文件的核心职责）
//!
//! 缺陷：调用方 `on_event` 返 `false`（设备打断）后，引擎仍`Ok(LlmTurnResult{ tool_calls })`
//! 把**打断前累积的工具调用**交出去。工具调用的下游是 MCP `tools/call`（控制设备音量、
//! 播放音乐……）——**不可回滚的外部副作用**，"用户已闭嘴、设备还在执行上一轮指令"。
//! `LlmEvent::Done` 当时是被`!cancelled` 守卫挡住的，但 `tool_calls` 走"累积到 `out.calls`
//! → 在返回值里一次性带出"这条路**绕过**了 `on_event`，两条路径语义不一致 = 根因。
//!
//! 契约（见 `LlmTurnResult` 类型文档）：取消时 `tool_calls` **恒空**、`cancelled == true`、
//! 不发 `ToolCall`/`Done`，但 `usage` 与部分文本**照常返回**（钱已花、观测要用）。

use super::*;
use crate::plugins::llm::LlmTurnResult;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
        sentence_flush_ms: 1500,
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

// ---------------------------------------------------------------------------
// 取消契约测试用的本地 mock 上游
// ---------------------------------------------------------------------------

/// 一次性 mock Responses 端点：在 `127.0.0.1:0` 上按给定 `content_type` + body 应答。
///
/// 为什么自己起TCP 而不加 mock 依赖：`chat_stream` 是**私有**实现方法，要测它就必须真发一次
/// HTTP；引`wiremock`/`httpmock` 会给这个"协议层零依赖"的 crate 增加一批仅测试期依赖，
/// 而一个只写固定响应头的裸 HTTP 服务端足够（无状态、单请求、单响应）。
struct MockUpstream {
    api_base: String,
    handle: tokio::task::JoinHandle<()>,
}

impl MockUpstream {
    /// `body` 原样作为响应体；`content_type` 决定走 SSE 还是非流式回退路径。
    async fn start(body: String, content_type: &'static str) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定 mock 端口");
        let addr = listener.local_addr().expect("读取 mock 地址");
        let handle = tokio::spawn(async move {
            // 单连接单请求：足以覆盖一个 `chat_stream` 调用
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            // 读掉请求头（到 `\r\n\r\n` 为止）；请求体（JSON input[]）无需解析
            let mut buf = Vec::new();
            let mut tmp = [0u8; 1024];
            loop {
                match sock.read(&mut tmp).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                }
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.flush().await;
        });
        Self {
            api_base: format!("http://{addr}/v1/responses"),
            handle,
        }
    }

    fn client(&self, stream: bool) -> LlmClient {
        LlmClient::new(&LlmConfig {
            api_base: self.api_base.clone(),
            api_key: "test-key".into(),
            model: "gpt-4o".into(),
            stream,
            ..LlmConfig::default()
        })
    }
}

impl Drop for MockUpstream {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// 带usage + 一个工具调用的 SSE 流（打断测试的"上一轮已经产生的副作用"）。
fn sse_with_tool_call_and_usage() -> String {
    concat!(
        // 一段文本（打断前已出声）
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"好的，\"}\n\n",
        // 一个工具调用：控制设备音量——不可回滚的外部副作用
        "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"set_volume\",\"arguments\":\"\"}}\n\n",
        "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":0,\"delta\":\"{\\\"level\\\":80}\"}\n\n",
        // usage：钱已经花了，取消也必须上报
        "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":30,\"output_tokens\":7,\"input_tokens_details\":{\"cached_tokens\":10}}}}\n\n",
    )
    .to_string()
}

/// **核心回归测试（SSE 路径）**：打断后上一轮已累积的 `tool_calls` **不得**泄漏给下游。
///
/// 断言三件事（缺一不可）：
/// 1. `tool_calls` 为空——MCP `tools/call` 的输入没了，设备不会被唤醒执行上一轮的指令；
/// 2. `cancelled == true`——下游能区分"正常说完"与"被用户掐了"；
/// 3. `usage` 与部分文本仍在——统计与观测不能因为打断而丢。
///
/// 同时断言**回调侧**没收到 `ToolCall`/`Done`：这两者语义上属"有效意图"，
/// 打断后不得再向调用方声明"本轮意图是调用 set_volume"。
#[tokio::test]
async fn cancelled_turn_drops_accumulated_tool_calls() {
    let up = MockUpstream::start(sse_with_tool_call_and_usage(), "text/event-stream").await;
    let client = up.client(true);

    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = events.clone();
    let out: LlmTurnResult = client
        .chat_stream_inner(
            &[],
            "把音量调到 80",
            &TurnPrompt::default(),
            None,
            Box::new(move |ev| {
                match &ev {
                    LlmEvent::Text(t) => sink.lock().unwrap().push(format!("text:{t}")),
                    LlmEvent::ToolCall(c) => {
                        sink.lock().unwrap().push(format!("tool:{}", c.name))
                    }
                    LlmEvent::Done => sink.lock().unwrap().push("done".into()),
                }
                // 模拟设备打断：回调拒绝后续事件（producer.rs 的真实行为）
                false
            }),
        )
        .await
        .expect("取消不是错误，应返回 Ok");

    // ① 打断前累积的工具调用**必须丢弃**
    assert!(
        out.tool_calls.is_empty(),
        "打断后 tool_calls 泄漏：{:?}（会导致 MCP 继续执行上一轮指令）",
        out.tool_calls
    );
    // ② 下游可区分
    assert!(out.cancelled, "取消轮必须置cancelled = true");
    // ③ 用量与部分文本照常保留
    let usage = out.usage.expect("取消轮也必须上报 usage（钱已花）");
    assert_eq!(usage.input, 20, "未命中缓存的输入 = 30 - 10");
    assert_eq!(usage.cache_read, 10);
    assert_eq!(usage.output, 7);
    assert_eq!(out.text, "好的，", "已累积的部分文本要保留（观测用）");

    let seen = events.lock().unwrap().clone();
    assert!(
        !seen.iter().any(|e| e.starts_with("tool:")),
        "打断后不得再向调用方声明 ToolCall 事件，实际收到 {seen:?}"
    );
    assert!(
        !seen.iter().any(|e| e == "done"),
        "打断后不得发 Done（流没正常走完），实际收到 {seen:?}"
    );
    assert!(
        seen.iter().any(|e| e.starts_with("text:")),
        "打断前已交付的文本不应被追回，实际收到 {seen:?}"
    );
}

/// **非取消对照**：正常走完时 `tool_calls` 必须照常交付（防止修复过头把功能改坏）。
#[tokio::test]
async fn normal_turn_still_delivers_tool_calls() {
    let up = MockUpstream::start(sse_with_tool_call_and_usage(), "text/event-stream").await;
    let client = up.client(true);

    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = seen.clone();
    let out = client
        .chat_stream_inner(&[], "把音量调到 80", &TurnPrompt::default(), None, {
            Box::new(move |ev| {
                match &ev {
                    LlmEvent::Text(t) => sink.lock().unwrap().push(format!("text:{t}")),
                    LlmEvent::ToolCall(c) => {
                        sink.lock().unwrap().push(format!("tool:{}", c.name))
                    }
                    LlmEvent::Done => sink.lock().unwrap().push("done".into()),
                }
                true
            })
        })
        .await
        .expect("正常轮不应是错误");

    assert!(!out.cancelled, "正常轮cancelled 必须为 false");
    assert_eq!(out.tool_calls.len(), 1, "正常轮必须交付工具调用");
    assert_eq!(out.tool_calls[0].name, "set_volume");
    assert_eq!(out.tool_calls[0].arguments, "{\"level\":80}");
    let events = seen.lock().unwrap().clone();
    assert!(events.iter().any(|e| e == "tool:set_volume"));
    assert!(events.iter().any(|e| e == "done"), "正常轮必须发 Done");
}

/// **非 SSE 回退路径的取消语义必须与 SSE 路径一致**。
///
/// 该路径此前**完全丢弃** `on_event` 的返回值：调用方打断后，整段工具调用照样通过
/// `on_event(ToolCall)` 和返回值双路交付——两条路径语义分裂正是缺陷根因，故一并覆盖。
#[tokio::test]
async fn cancelled_non_sse_turn_drops_tool_calls() {
    let body = json!({
        "id": "resp_3",
        "status": "completed",
        "output": [
            { "type": "message", "role": "assistant",
              "content": [ { "type": "output_text", "text": "好的" } ] },
            { "type": "function_call", "call_id": "call_9", "name": "play_music",
              "arguments": "{\"song\":\"晴天\"}" }
        ],
        "usage": { "input_tokens": 11, "output_tokens": 4 }
    })
    .to_string();
    let up = MockUpstream::start(body, "application/json").await;
    //配置 stream=false 且服务端非 event-stream → 走非流式回退分支
    let client = up.client(false);

    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = seen.clone();
    let out = client
        .chat_stream_inner(&[], "放首歌", &TurnPrompt::default(), None, {
            Box::new(move |ev| {
                match &ev {
                    LlmEvent::Text(t) => sink.lock().unwrap().push(format!("text:{t}")),
                    LlmEvent::ToolCall(c) => {
                        sink.lock().unwrap().push(format!("tool:{}", c.name))
                    }
                    LlmEvent::Done => sink.lock().unwrap().push("done".into()),
                }
                false
            })
        })
        .await
        .expect("取消不是错误");

    assert!(
        out.tool_calls.is_empty(),
        "非 SSE 路径取消后 tool_calls 泄漏：{:?}",
        out.tool_calls
    );
    assert!(out.cancelled, "非 SSE 路径也必须置 cancelled = true");
    assert_eq!(out.usage.expect("取消轮也上报 usage").input, 11);
    let events = seen.lock().unwrap().clone();
    assert!(
        !events.iter().any(|e| e == "tool:play_music"),
        "非 SSE 路径取消后不得再发 ToolCall，实际 {events:?}"
    );
    assert!(
        !events.iter().any(|e| e == "done"),
        "非 SSE 路径取消后不得发 Done，实际 {events:?}"
    );
}

/// 非取消的非 SSE 对照：正常交付（含 `Done`），防止上一条把功能改坏。
#[tokio::test]
async fn normal_non_sse_turn_delivers_tool_calls() {
    let body = json!({
        "output": [ { "type": "function_call", "call_id": "c1", "name": "light_on",
                      "arguments": "{}" } ],
        "usage": { "input_tokens": 3, "output_tokens": 1 }
    })
    .to_string();
    let up = MockUpstream::start(body, "application/json").await;
    let client = up.client(false);
    let out = client
        .chat_stream_inner(&[], "开灯", &TurnPrompt::default(), None, Box::new(|_| true))
        .await
        .expect("正常轮不应是错误");
    assert!(!out.cancelled);
    assert_eq!(out.tool_calls.len(), 1);
    assert_eq!(out.tool_calls[0].name, "light_on");
}

/// **空回复仍然归类为可重试失败**——取消契约不能把"什么都没说"也放过去。
///
/// 这是修复前`if !cancelled && …` 那个守卫的等价行为：空回复判失败，空**取消**判取消。
#[tokio::test]
async fn empty_response_is_still_a_failure() {
    let up = MockUpstream::start(
        "data: {\"type\":\"response.completed\",\"response\":{}}\n\n".to_string(),
        "text/event-stream",
    )
    .await;
    let client = up.client(true);
    let err = client
        .chat_stream_inner(&[], "在吗", &TurnPrompt::default(), None, Box::new(|_| true))
        .await
        .expect_err("200 但既无文本也无工具调用 = 空回复失败");
    let failure = LlmFailure::classify(&err);
    assert!(
        matches!(failure, LlmFailure::EmptyResponse),
        "空回复应归类为 EmptyResponse，实得 {:?}",
        failure.category()
    );
}

/// **取消轮即便没有工具调用、只有一段文本，也不该被判成空回复失败**。
///
/// 回归点：修复前是`if !cancelled && out.text.trim().is_empty() && out.calls.is_empty()`，
/// 整个判据被 `cancelled` 短路。保留该守卫本身没错（空回复仍须失败，见上一条测试），
/// 但它**不能**反过来让"取消"被吞成失败——取消的下游语义（不写历史、不执行工具）
/// 与"上游故障"完全不同，混淆会让打断看起来像AI 坏了。
///
/// 注意前提：**取消只能通过 `on_event` 拒绝来观察**。若上游一个文本增量都没给，
/// 回调从不被调用，也就无所谓"取消"——那种情况判`EmptyResponse` 是正确的
/// （见 `empty_response_is_still_a_failure`）。
#[tokio::test]
async fn cancelled_turn_with_text_only_is_not_empty_response_failure() {
    let up = MockUpstream::start(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"嗯\"}\n\n\
         data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":5,\"output_tokens\":1}}}\n\n"
            .to_string(),
        "text/event-stream",
    )
    .await;
    let client = up.client(true);
    let out = client
        .chat_stream_inner(&[], "在吗", &TurnPrompt::default(), None, Box::new(|_| false))
        .await
        .expect("取消不是错误，不该返回 Err");
    assert!(out.cancelled, "应标记为取消而非失败");
    assert_eq!(out.text, "嗯");
    assert!(out.tool_calls.is_empty());
    assert_eq!(out.usage.expect("取消轮也上报 usage").output, 1);
}

/// **取消优先于错误**：触发取消的那一块里同时带 `error` 事件时，不能报 `Protocol`。
///
/// 真实场景：网络抖动与用户打断常同时发生。此时会话若收到`Protocol` 会走
/// "上游故障 → 错误提示/兜底话术"路径，而用户明明只是自己闭了嘴。
#[tokio::test]
async fn cancellation_takes_precedence_over_stream_error() {
    let up = MockUpstream::start(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"半句\"}\n\n\
         data: {\"type\":\"error\",\"message\":\"upstream socket closed\"}\n\n"
            .to_string(),
        "text/event-stream",
    )
    .await;
    let client = up.client(true);
    let out = client
        .chat_stream_inner(&[], "在吗", &TurnPrompt::default(), None, Box::new(|_| false))
        .await
        .expect("取消不是错误：同一块里的流式错误不得把打断变成上游故障");
    assert!(out.cancelled, "应按取消处理");
    assert_eq!(out.text, "半句");
}

/// 非取消轮里的 `error` 事件仍必须报 `Protocol`（守住上一条测试没有把护栏改松）。
#[tokio::test]
async fn stream_error_without_cancellation_is_still_protocol_failure() {
    let up = MockUpstream::start(
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"半句\"}\n\n\
         data: {\"type\":\"error\",\"message\":\"boom\"}\n\n"
            .to_string(),
        "text/event-stream",
    )
    .await;
    let client = up.client(true);
    let err = client
        .chat_stream_inner(&[], "在吗", &TurnPrompt::default(), None, Box::new(|_| true))
        .await
        .expect_err("未取消时的流式错误应报 Protocol");
    let failure = LlmFailure::classify(&err);
    assert!(
        matches!(failure, LlmFailure::Protocol(_)),
        "未取消的流式错误应归类 Protocol，实得 {}",
        failure.category()
    );
}

/// 401 应按 `Status` 归类且**不可重试**（分类路由护栏的既有行为，顺带守住重试可判据）。
#[tokio::test]
async fn non_error_status_returns_status_failure() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
        let mut tmp = [0u8; 1024];
        let _ = sock.read(&mut tmp).await;
        let body = "no key";
        let resp = format!(
            "HTTP/1.1 401 Unauthorized\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        let _ = sock.flush().await;
    });
    let client = LlmClient::new(&LlmConfig {
        api_base: format!("http://{addr}/v1/responses"),
        api_key: "bad".into(),
        stream: true,
        ..LlmConfig::default()
    });
    let err = client
        .chat_stream_inner(&[], "你好", &TurnPrompt::default(), None, Box::new(|_| true))
        .await
        .expect_err("401 应是错误");
    let failure = LlmFailure::classify(&err);
    assert!(
        !failure.retryable(),
        "401 属客户端错误，不可重试（实测 category={}）",
        failure.category()
    );
}