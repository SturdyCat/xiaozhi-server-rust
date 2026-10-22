//! `server/src/app/mcp.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（`AGENTS.md` §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。
//!
//! 报文样例**逐字取自固件** `main/mcp_server.cc`（`ReplyResult` / `ReplyError`）与
//! `main/mcp_server.h`（`ReturnValue` → `content[]` 的包装），不是臆造的形态。

use super::*;

/// 构造上行信封（固件 `Protocol::SendMcpMessage` 的产物）。
fn envelope(payload: Value) -> String {
    json!({ "session_id": "s1", "type": "mcp", "payload": payload }).to_string()
}

/// 测试用别名：`take_reply_for` 已并入 `take_response_for`（信封解析只留一个出口，
/// 生产代码里不需要第二个没人调用的包装），测试沿用旧名以免为改名重写一批断言。
fn take_reply_for(text: &str, id: u64) -> Option<McpReply> {
    take_response_for(text, id).map(McpResponse::into_reply)
}

#[test]
fn tools_call_shape_matches_firmware_expectations() {
    let req = tools_call(7, "self.audio_listener.stop", json!({}));
    assert_eq!(req["jsonrpc"], JSONRPC_VERSION);
    assert_eq!(req["method"], "tools/call");
    assert_eq!(req["params"]["name"], "self.audio_listener.stop");
    assert_eq!(req["params"]["arguments"], json!({}));
    // 固件要求 id 是**数字**（`mcp_server.cc:409`），序列化出来必须是 number 而非字符串。
    assert_eq!(req["id"], 7);
    assert!(req["id"].is_u64(), "id 必须是数字，字符串会被固件整条丢弃");
}

#[test]
fn parse_reply_reads_bool_tool_result() {
    // 固件：工具返回 true → {"content":[{"type":"text","text":"true"}],"isError":false}
    let reply = parse_reply(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": { "content": [{ "type": "text", "text": "true" }], "isError": false }
    }))
    .expect("合法响应应被解析");
    assert_eq!(reply.id, 1);
    assert_eq!(
        reply.outcome,
        McpOutcome::Ok {
            text: Some("true".into()),
            is_error: false
        }
    );
    assert!(reply.succeeded());
    assert!(reply.summary().contains("ok"));
}

#[test]
fn parse_reply_reads_int_tool_result() {
    let reply = parse_reply(&json!({
        "jsonrpc": "2.0",
        "id": 9,
        "result": { "content": [{ "type": "text", "text": "50" }], "isError": false }
    }))
    .unwrap();
    assert_eq!(reply.id, 9);
    assert_eq!(
        reply.outcome,
        McpOutcome::Ok {
            text: Some("50".into()),
            is_error: false
        }
    );
}

#[test]
fn parse_reply_reads_error_branch() {
    // 固件：未知工具 → ReplyError(-32602, "Unknown tool: …")
    let reply = parse_reply(&json!({
        "jsonrpc": "2.0",
        "id": 3,
        "error": { "code": -32602, "message": "Unknown tool: self.audio_listener.stop" }
    }))
    .unwrap();
    assert_eq!(reply.id, 3);
    assert_eq!(
        reply.outcome,
        McpOutcome::Err {
            code: -32602,
            message: "Unknown tool: self.audio_listener.stop".into()
        }
    );
    assert!(!reply.succeeded(), "error 分支不算执行成功");
    assert!(reply.summary().contains("-32602"), "{}", reply.summary());
}

#[test]
fn parse_reply_image_result_has_no_text() {
    let reply = parse_reply(&json!({
        "jsonrpc": "2.0",
        "id": 4,
        "result": { "content": [{ "type": "image", "image": "{}" }], "isError": false }
    }))
    .unwrap();
    assert_eq!(
        reply.outcome,
        McpOutcome::Ok {
            text: None,
            is_error: false
        }
    );
    assert!(reply.succeeded(), "图片结果也是成功执行");
}

#[test]
fn parse_reply_is_error_true_is_not_success() {
    let reply = parse_reply(&json!({
        "jsonrpc": "2.0",
        "id": 5,
        "result": { "content": [{ "type": "text", "text": "boom" }], "isError": true }
    }))
    .unwrap();
    assert!(!reply.succeeded(), "isError=true 不能算成功");
    assert!(reply.summary().contains("isError"), "{}", reply.summary());
}

/// 解析必须**收得很紧**：畸形报文返回 `None`（继续等），绝不能误判成"工具调用成功"。
#[test]
fn parse_reply_rejects_malformed_payloads() {
    // 非对象
    assert!(parse_reply(&json!("tools/call")).is_none());
    // jsonrpc 缺失 / 版本不对（固件自己也严格校验 2.0）
    assert!(parse_reply(&json!({"id": 1, "result": {}})).is_none());
    assert!(parse_reply(&json!({"jsonrpc": "1.0", "id": 1, "result": {}})).is_none());
    // id 必须是数字：字符串 id 一律不认（固件根本不会这么发）
    assert!(parse_reply(&json!({"jsonrpc": "2.0", "id": "1", "result": {}})).is_none());
    assert!(parse_reply(&json!({"jsonrpc": "2.0", "result": {}})).is_none());
    // 既无 result 也无 error：不是响应（例如设备主动通知）
    assert!(parse_reply(&json!({"jsonrpc": "2.0", "id": 1})).is_none());
}

#[test]
fn take_reply_for_unwraps_envelope_and_matches_id() {
    let text = envelope(json!({
        "jsonrpc": "2.0",
        "id": 42,
        "result": { "content": [{ "type": "text", "text": "true" }], "isError": false }
    }));
    let reply = take_reply_for(&text, 42).expect("信封 + id 匹配应取出响应");
    assert_eq!(reply.id, 42);
    assert!(reply.succeeded());
}

#[test]
fn take_reply_for_ignores_other_ids_and_other_types() {
    // 迟到的上一次响应：id 不匹配 → 必须继续等，不能当成本次结果
    let late = envelope(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": { "content": [{ "type": "text", "text": "true" }], "isError": false }
    }));
    assert!(take_reply_for(&late, 2).is_none(), "id 不匹配不能配对");

    // 非 mcp 消息（listen / abort / stt）：与工具调用无关
    assert!(take_reply_for(r#"{"session_id":"s1","type":"listen","state":"stop"}"#, 2).is_none());
    assert!(take_reply_for(r#"{"session_id":"s1","type":"abort","reason":"none"}"#, 2).is_none());
    // 畸形 JSON 不能让会话炸掉
    assert!(take_reply_for("not json at all", 2).is_none());
    // type=mcp 但 payload 不是合法响应
    assert!(take_reply_for(&envelope(json!({"jsonrpc": "2.0", "id": 2})), 2).is_none());
}

#[test]
fn stop_listening_tool_name_is_the_firmware_contract() {
    // 改名必须两边同步（固件 `main/mcp_server.cc` 的 AddUserOnlyTool）。
    // 这条断言是"改错一边就会红"的护栏，不要因为"只是字符串"而删掉。
    assert_eq!(STOP_LISTENING_TOOL, "self.audio_listener.stop");
    assert!(
        DEFAULT_CALL_TIMEOUT >= Duration::from_millis(500)
            && DEFAULT_CALL_TIMEOUT <= Duration::from_secs(5),
        "超时要远大于设备本地执行耗时、又远小于用户可察觉的卡顿"
    );
}

// ------------------------- 原始 result 视图（initialize / tools/list 用） -------------------------

#[test]
fn parse_response_keeps_the_raw_result() {
    // `tools/call` 压成 content[] 文本会丢信息；能力发现要读 result 里的字段，故保留原始 JSON。
    let res = parse_response(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "bread-compact-wifi", "version": "1.6.4" }
        }
    }))
    .expect("合法响应应被解析");
    assert_eq!(res.id(), 2);
    match res {
        McpResponse::Ok { result, .. } => {
            assert_eq!(result["serverInfo"]["name"], "bread-compact-wifi");
            assert_eq!(result["protocolVersion"], "2024-11-05");
        }
        other => panic!("应为 Ok，实际 {other:?}"),
    }
}

#[test]
fn parse_response_and_parse_reply_share_one_envelope_rule() {
    // 两个视图必须对"什么算一条响应"给出完全一致的判断——否则会出现
    // "tools/call 认、tools/list 不认"这种只有某条路径坏掉的诡异故障。
    let good = json!({ "jsonrpc": "2.0", "id": 1, "result": {} });
    assert!(parse_response(&good).is_some());
    assert!(parse_reply(&good).is_some());

    for bad in [
        json!("tools/list"),
        json!({ "id": 1, "result": {} }),
        json!({ "jsonrpc": "1.0", "id": 1, "result": {} }),
        json!({ "jsonrpc": "2.0", "id": "1", "result": {} }),
        json!({ "jsonrpc": "2.0", "id": 1 }),
    ] {
        assert!(parse_response(&bad).is_none(), "{bad} 不该被当成响应");
        assert!(parse_reply(&bad).is_none(), "{bad} 不该被当成响应");
    }
}

#[test]
fn parse_response_error_branch_has_no_result() {
    match parse_response(&json!({
        "jsonrpc": "2.0",
        "id": 9,
        "error": { "code": -32601, "message": "Method not implemented: tools/list" }
    }))
    .unwrap()
    {
        McpResponse::Err { code, message, .. } => {
            assert_eq!(code, -32601);
            assert_eq!(message, "Method not implemented: tools/list");
        }
        other => panic!("应为 Err，实际 {other:?}"),
    }
    // 缺 code 的畸形 error 按"内部错误"处理而不是丢弃：丢弃会让调用方一直等到超时，
    // 把"设备明确报错"误报成"设备没回应"（两种故障的排查方向完全不同）。
    match parse_response(&json!({
        "jsonrpc": "2.0", "id": 9, "error": { "message": "boom" }
    }))
    .unwrap()
    {
        McpResponse::Err { code, .. } => assert_eq!(code, -32603),
        other => panic!("应为 Err，实际 {other:?}"),
    }
}

#[test]
fn into_reply_maps_both_branches() {
    let ok = parse_response(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": { "content": [{ "type": "text", "text": "50" }], "isError": false }
    }))
    .unwrap()
    .into_reply();
    assert_eq!(ok.id, 1);
    assert!(ok.succeeded());

    let err = parse_response(&json!({
        "jsonrpc": "2.0", "id": 2, "error": { "code": -32602, "message": "Unknown tool" }
    }))
    .unwrap()
    .into_reply();
    assert_eq!(err.id, 2);
    assert!(!err.succeeded());
}

#[test]
fn take_response_for_unwraps_envelope_and_matches_id() {
    let text = envelope(json!({
        "jsonrpc": "2.0",
        "id": 42,
        "result": { "tools": [{ "name": "self.lamp.turn_on", "description": "d", "inputSchema": {} }] }
    }));
    let res = take_response_for(&text, 42).expect("信封 + id 匹配应取出响应");
    assert_eq!(res.id(), 42);
    // 迟到的上一次响应（id=1）不能当成本次（id=2）的结果
    assert!(take_response_for(&text, 7).is_none());
    assert!(take_response_for(r#"{"type":"listen","state":"stop"}"#, 42).is_none());
    assert!(take_response_for("not json", 42).is_none());
}
