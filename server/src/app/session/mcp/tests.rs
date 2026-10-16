//! `server/src/app/session/mcp.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（`AGENTS.md` §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。
//!
//! 这里只测**等待泵**（`await_tool_reply`）：`Session` 需要真实 VAD 模型才能构造
//! （`engine.rs` 的必需能力），所以泵被刻意写成不依赖 `Session` 的自由函数。
//! 协议报文本身的构造/解析在 `app/mcp/tests.rs`。

use super::*;

/// 内存传输：预置上行帧 + 记录下行文本。
///
/// `hang=true` 时"没有预置帧"表现为**永不就绪**（模拟设备收下请求却不回包），
/// 用于触发超时分支——这与"承载断开"（`Closed`）是两种必须区分开的失败。
struct MockTransport {
    frames: Vec<IncomingFrame>,
    hang: bool,
    sent: Vec<String>,
}

impl MockTransport {
    fn new(frames: Vec<IncomingFrame>) -> Self {
        Self {
            frames,
            hang: false,
            sent: Vec::new(),
        }
    }

    fn hanging() -> Self {
        Self {
            frames: Vec::new(),
            hang: true,
            sent: Vec::new(),
        }
    }
}

impl Transport for MockTransport {
    async fn send_text(&mut self, json: String) -> anyhow::Result<()> {
        self.sent.push(json);
        Ok(())
    }
    async fn send_binary(&mut self, _data: Vec<u8>) -> anyhow::Result<()> {
        Ok(())
    }
    async fn recv(&mut self) -> IncomingFrame {
        if self.frames.is_empty() {
            if self.hang {
                std::future::pending::<()>().await;
            }
            return IncomingFrame::Closed;
        }
        self.frames.remove(0)
    }
    fn try_recv(&mut self) -> Option<IncomingFrame> {
        (!self.frames.is_empty()).then(|| self.frames.remove(0))
    }
    async fn keepalive(&mut self) {}
}

/// 设备侧成功响应（固件 `ReplyResult` 形态）包进上行信封。
fn reply_frame(id: u64) -> IncomingFrame {
    IncomingFrame::Text(
        serde_json::json!({
            "session_id": "s1",
            "type": "mcp",
            "payload": {
                "jsonrpc": "2.0",
                "id": id,
                "result": { "content": [{ "type": "text", "text": "true" }], "isError": false }
            }
        })
        .to_string(),
    )
}

fn text_frame(raw: &str) -> IncomingFrame {
    IncomingFrame::Text(raw.to_string())
}

#[tokio::test]
async fn reply_returns_immediately_without_deferring() {
    let mut t = MockTransport::new(vec![reply_frame(1)]);
    let (result, deferred) = await_tool_reply(&mut t, 1, Duration::from_millis(200)).await;
    let reply = result.expect("配对响应应返回 Ok");
    assert_eq!(reply.id, 1);
    assert!(reply.succeeded());
    assert!(deferred.is_empty(), "没有其他消息时不应有遗留帧");
}

#[tokio::test]
async fn unrelated_text_frames_are_deferred_in_order() {
    // 真机时序：设备先回 `listen stop`（结束聆听的自报），其间还可能来一条 abort，最后才是响应。
    let mut t = MockTransport::new(vec![
        text_frame(r#"{"session_id":"s1","type":"listen","state":"stop"}"#),
        text_frame(r#"{"session_id":"s1","type":"abort","reason":"none"}"#),
        reply_frame(7),
    ]);
    let (result, deferred) = await_tool_reply(&mut t, 7, Duration::from_millis(200)).await;
    assert_eq!(result.unwrap().id, 7);
    assert_eq!(
        deferred.len(),
        2,
        "非本次响应的文本帧必须原样交回调用方，不能丢"
    );
    assert!(deferred[0].contains("listen"), "{}", deferred[0]);
    assert!(deferred[1].contains("abort"), "{}", deferred[1]);
}

#[tokio::test]
async fn wrong_id_is_deferred_and_the_matching_one_wins() {
    // 迟到的上一次响应（id=1）不能当成本次（id=2）的结果——否则会"用旧结果报成功"。
    let mut t = MockTransport::new(vec![reply_frame(1), reply_frame(2)]);
    let (result, deferred) = await_tool_reply(&mut t, 2, Duration::from_millis(200)).await;
    assert_eq!(result.unwrap().id, 2);
    assert_eq!(
        deferred.len(),
        1,
        "id 不匹配的响应应被留下交调用方按普通消息处理"
    );
    assert!(deferred[0].contains("\"id\":1"), "{}", deferred[0]);
}

#[tokio::test]
async fn binary_frames_are_dropped_not_deferred() {
    // 等待期间的上行音频：用户已要求退下，直接丢弃（不回灌 VAD、不留给调用方）。
    let mut t = MockTransport::new(vec![
        IncomingFrame::Binary(vec![1, 2, 3]),
        IncomingFrame::Binary(vec![4]),
        reply_frame(1),
    ]);
    let (result, deferred) = await_tool_reply(&mut t, 1, Duration::from_millis(200)).await;
    assert!(result.is_ok());
    assert!(deferred.is_empty(), "音频帧不该进遗留列表");
}

#[tokio::test]
async fn silent_device_times_out() {
    let mut t = MockTransport::hanging();
    let timeout = Duration::from_millis(30);
    let (result, deferred) = await_tool_reply(&mut t, 1, timeout).await;
    assert_eq!(result, Err(McpCallError::Timeout(timeout)));
    assert!(deferred.is_empty());
    // 错误文案要说清"等了多久"（现场定位用）
    assert!(McpCallError::Timeout(timeout).to_string().contains("30ms"));
}

#[tokio::test]
async fn closed_transport_is_reported_as_closed_not_timeout() {
    let mut t = MockTransport::new(vec![]);
    let (result, _deferred) = await_tool_reply(&mut t, 1, Duration::from_millis(200)).await;
    assert_eq!(
        result,
        Err(McpCallError::Closed),
        "断开与超时是两种失败，日志要能区分"
    );
}

#[tokio::test]
async fn device_error_reply_is_a_normal_result() {
    // 老固件没有该工具 → `-32602 Unknown tool`。这是**设备明确拒绝**，不是传输失败：
    // 返回 Ok，由调用方看 succeeded() 决定降级（两者日志口径不同）。
    let mut t = MockTransport::new(vec![IncomingFrame::Text(
        serde_json::json!({
            "session_id": "s1",
            "type": "mcp",
            "payload": {
                "jsonrpc": "2.0",
                "id": 5,
                "error": { "code": -32602, "message": "Unknown tool: self.audio_listener.stop" }
            }
        })
        .to_string(),
    )]);
    let (result, _deferred) = await_tool_reply(&mut t, 5, Duration::from_millis(200)).await;
    let reply = result.expect("设备拒绝是正常响应，不是传输错误");
    assert_eq!(reply.id, 5);
    assert!(!reply.succeeded());
    assert!(
        reply.summary().contains("Unknown tool"),
        "{}",
        reply.summary()
    );
}

#[tokio::test]
async fn error_texts_say_what_to_do_next() {
    // 降级路径的日志文案要能直接回答"为什么退下没走 MCP"。
    // （`Session` 需要真实 VAD 模型才能构造，故这里只锚住文案；"先判开关再发"的分支
    //   由 `mcp_call_tool` 开头的 `if !self.mcp_enabled` 保证，零网络开销。）
    let unsupported = McpCallError::Unsupported.to_string();
    assert!(unsupported.contains("features.mcp"), "{unsupported}");
    let send = McpCallError::Send("boom".into()).to_string();
    assert!(send.contains("boom"), "{send}");
}

// ===================== 能力发现（initialize + tools/list） =====================

/// 设备侧带 `result` 的响应帧（`initialize` / `tools/list` 用）。
fn result_frame(id: u64, result: serde_json::Value) -> IncomingFrame {
    IncomingFrame::Text(
        serde_json::json!({
            "session_id": "s1",
            "type": "mcp",
            "payload": { "jsonrpc": "2.0", "id": id, "result": result }
        })
        .to_string(),
    )
}

/// 设备侧 `error` 响应帧。
fn error_frame(id: u64, code: i64, message: &str) -> IncomingFrame {
    IncomingFrame::Text(
        serde_json::json!({
            "session_id": "s1",
            "type": "mcp",
            "payload": { "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }
        })
        .to_string(),
    )
}

/// 固件 `initialize` 回包（照抄 `mcp_server.cc` 的拼接结果）。
fn server_info() -> serde_json::Value {
    serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "bread-compact-wifi", "version": "1.6.4" }
    })
}

/// 一页 `tools/list`（`next_cursor` 非空时带上 `nextCursor`）。
fn tools_page(names: &[&str], next_cursor: Option<&str>) -> serde_json::Value {
    let tools: Vec<serde_json::Value> = names
        .iter()
        .map(|n| {
            serde_json::json!({
                "name": n,
                "description": format!("{n} 的说明"),
                "inputSchema": { "type": "object" }
            })
        })
        .collect();
    match next_cursor {
        Some(c) => serde_json::json!({ "tools": tools, "nextCursor": c }),
        None => serde_json::json!({ "tools": tools }),
    }
}

/// 取出下行 MCP 报文的 `payload`（顺带断言外层信封是 `type:"mcp"`）。
fn sent_payloads(t: &MockTransport) -> Vec<serde_json::Value> {
    t.sent
        .iter()
        .map(|s| {
            let v: serde_json::Value = serde_json::from_str(s).expect("下行必须是 JSON");
            assert_eq!(v["type"], "mcp", "MCP 报文必须套 type:mcp 信封: {s}");
            v["payload"].clone()
        })
        .collect()
}

#[tokio::test]
async fn discovery_walks_initialize_then_pages_of_tools_list() {
    let mut t = MockTransport::new(vec![
        result_frame(1, server_info()),
        result_frame(
            2,
            tools_page(
                &["self.audio_speaker.set_volume"],
                Some("self.lamp.turn_on"),
            ),
        ),
        result_frame(3, tools_page(&["self.lamp.turn_on", "self.reboot"], None)),
    ]);
    let mut next_id = 1;
    let (found, deferred) =
        discover_tools(&mut t, "s1", &mut next_id, Duration::from_millis(500)).await;

    assert_eq!(found.error, None, "三帧都齐 → 完整成功");
    assert_eq!(found.server.name, "bread-compact-wifi");
    assert_eq!(found.server.version, "1.6.4");
    assert_eq!(found.tools.len(), 3, "两页的工具要合并");
    assert_eq!(found.tools[1].name, "self.lamp.turn_on");
    assert_eq!(found.pages, 2);
    assert!(deferred.is_empty(), "全是配对响应，没有遗留帧");
    // 报文顺序：initialize → notifications/initialized → tools/list(游标) → tools/list(下一页游标)
    let sent = sent_payloads(&t);
    assert_eq!(sent.len(), 4, "{sent:?}");
    assert_eq!(sent[0]["method"], "initialize");
    assert_eq!(sent[1]["method"], "notifications/initialized");
    assert!(sent[1].get("id").is_none(), "通知不能带 id");
    assert_eq!(sent[2]["method"], "tools/list");
    assert_eq!(sent[2]["params"]["cursor"], "", "第一页游标为空");
    assert_eq!(
        sent[2]["params"]["withUserTools"], true,
        "管理界面要看全量（含 user_only），靠 annotations.audience 标注"
    );
    assert_eq!(
        sent[3]["params"]["cursor"], "self.lamp.turn_on",
        "下一页游标 = 上一页的 nextCursor（包含语义，不能自行 +1）"
    );
    // id 连续自增（配对靠它，回绕/重置都会让后续响应配错）
    assert_eq!(sent[0]["id"], 1);
    assert_eq!(sent[2]["id"], 2);
    assert_eq!(sent[3]["id"], 3);
    assert_eq!(next_id, 4, "取号必须交给调用方持有并自增");
}

#[tokio::test]
async fn discovery_buffers_every_frame_and_hands_it_back_in_order() {
    // 真机时序：设备建连后立刻上行音频 + listen start，探测请求就夹在这些帧中间。
    // 这些帧**一帧都不能丢**（丢了就是掐掉用户第一句话的开头），必须原样交回补派发。
    let mut t = MockTransport::new(vec![
        IncomingFrame::Binary(vec![1, 2, 3]),
        text_frame(r#"{"session_id":"s1","type":"listen","state":"start"}"#),
        IncomingFrame::Binary(vec![4, 5]),
        result_frame(1, server_info()),
        IncomingFrame::Binary(vec![6]),
        result_frame(2, tools_page(&["self.lamp.turn_on"], None)),
        IncomingFrame::Binary(vec![7]),
    ]);
    let mut next_id = 1;
    let (found, deferred) =
        discover_tools(&mut t, "s1", &mut next_id, Duration::from_millis(500)).await;

    assert_eq!(found.error, None);
    assert_eq!(found.tools.len(), 1);
    // 探测只读到"最后一页响应"为止，**不多读**：响应之后到达的帧留在承载里，由主循环正常取走
    // （过读会把帧堆在内存里、还可能吃掉后来的 abort）。
    assert_eq!(deferred.len(), 4, "音频 3 帧 + listen start 1 帧都要交回");
    let kinds: Vec<&str> = deferred
        .iter()
        .map(|f| match f {
            IncomingFrame::Text(_) => "text",
            IncomingFrame::Binary(_) => "binary",
            IncomingFrame::Closed => "closed",
        })
        .collect();
    assert_eq!(kinds, vec!["binary", "text", "binary", "binary"]);
    // 顺序必须保持（VAD 依赖帧序）：按字节校验
    let bytes: Vec<&Vec<u8>> = deferred
        .iter()
        .filter_map(|f| match f {
            IncomingFrame::Binary(b) => Some(b),
            _ => None,
        })
        .collect();
    assert_eq!(bytes, vec![&vec![1, 2, 3], &vec![4, 5], &vec![6]]);
    match &deferred[1] {
        IncomingFrame::Text(s) => assert!(s.contains("listen")),
        other => panic!("第 2 帧应是 listen start，实际 {other:?}"),
    }
    match t.try_recv() {
        Some(IncomingFrame::Binary(b)) => assert_eq!(b, vec![7], "响应之后的帧必须留在承载里"),
        other => panic!("承载里应还剩最后一帧音频，实际 {other:?}"),
    }
}

#[tokio::test]
async fn discovery_stops_when_device_refuses_initialize() {
    // 老固件/非标准设备：initialize 回 -32601。这是"设备明确不支持"，不是传输失败——
    // 立刻停手（不再发 tools/list），把原因交上去让界面能解释"为什么这台设备没有工具"。
    let mut t = MockTransport::new(vec![error_frame(
        1,
        -32601,
        "Method not implemented: initialize",
    )]);
    let mut next_id = 1;
    let (found, deferred) =
        discover_tools(&mut t, "s1", &mut next_id, Duration::from_millis(500)).await;

    let err = found.error.expect("必须回报原因");
    assert!(err.contains("-32601"), "{err}");
    assert!(found.tools.is_empty());
    assert_eq!(found.pages, 0);
    assert_eq!(sent_payloads(&t).len(), 1, "被拒绝后不该再发 tools/list");
    assert!(deferred.is_empty());
}

#[tokio::test]
async fn discovery_keeps_partial_tools_when_a_later_page_times_out() {
    // 第 1 页拿到了、第 2 页设备不回：保留已取到的工具 + 说明原因（不静默截断，也不算全失败）。
    let mut t = MockTransport {
        frames: vec![
            result_frame(1, server_info()),
            result_frame(2, tools_page(&["self.lamp.turn_on"], Some("self.reboot"))),
        ],
        hang: true, // 帧用完后表现为"永不就绪"（设备收下请求却不回包）
        sent: Vec::new(),
    };
    let mut next_id = 1;
    let (found, _deferred) =
        discover_tools(&mut t, "s1", &mut next_id, Duration::from_millis(80)).await;

    let err = found.error.expect("第 2 页超时要回报原因");
    assert!(err.contains("tools/list 未完成"), "{err}");
    assert!(err.contains("已取到 1 个工具"), "原因里要有进度: {err}");
    assert_eq!(found.tools.len(), 1, "已取到的工具必须保留");
    assert_eq!(found.pages, 1);
}

#[tokio::test]
async fn discovery_reports_exhausted_budget_instead_of_a_zero_ms_timeout() {
    // 预算为 0（或已耗尽）：给一句看得懂的说明，而不是下游"超时 0ms"这种没法行动的数字。
    let mut t = MockTransport::new(vec![result_frame(1, server_info())]);
    let mut next_id = 1;
    let (found, _deferred) = discover_tools(&mut t, "s1", &mut next_id, Duration::ZERO).await;
    let err = found.error.expect("预算为 0 必须回报");
    assert!(err.contains("预算"), "{err}");
    assert!(t.sent.is_empty(), "预算为 0 时一个报文都不该发");
    assert_eq!(next_id, 1, "没发请求就不该消耗 id");
}

#[tokio::test]
async fn discovery_stops_at_the_page_cap_and_says_so() {
    // 防"设备回 nextCursor 却不推进"的死循环：触顶保留已取到的工具并把原因写清楚。
    let mut frames = vec![result_frame(1, server_info())];
    for i in 0..(discovery::MAX_PAGES + 3) {
        frames.push(result_frame(
            (i + 2) as u64,
            tools_page(&[&format!("tool-{i}")], Some(&format!("tool-{i}"))),
        ));
    }
    let mut t = MockTransport::new(frames);
    let mut next_id = 1;
    let (found, _deferred) =
        discover_tools(&mut t, "s1", &mut next_id, Duration::from_millis(500)).await;

    assert_eq!(found.pages, discovery::MAX_PAGES);
    assert_eq!(found.tools.len(), discovery::MAX_PAGES);
    let err = found.error.expect("触顶必须回报（不静默截断）");
    assert!(err.contains("上限"), "{err}");
}

#[tokio::test]
async fn discovery_ignores_stale_ids_and_unrelated_frames() {
    // 迟到的上一次响应（id=99）不能当成本次结果；非 mcp 消息原样留下。
    let mut t = MockTransport::new(vec![
        text_frame(r#"{"session_id":"s1","type":"listen","state":"start"}"#),
        result_frame(99, tools_page(&["stale"], None)),
        result_frame(1, server_info()),
        result_frame(2, tools_page(&["self.lamp.turn_on"], None)),
    ]);
    let mut next_id = 1;
    let (found, deferred) =
        discover_tools(&mut t, "s1", &mut next_id, Duration::from_millis(500)).await;

    assert_eq!(found.error, None);
    assert_eq!(found.tools.len(), 1);
    assert_eq!(
        found.tools[0].name, "self.lamp.turn_on",
        "不能采信迟到的 id=99"
    );
    assert_eq!(deferred.len(), 2, "listen start 与迟到响应都要交回");
    match &deferred[1] {
        IncomingFrame::Text(s) => assert!(s.contains("99"), "迟到的响应按普通消息交回: {s}"),
        other => panic!("第 2 帧应是迟到的响应，实际 {other:?}"),
    }
}

#[tokio::test]
async fn discovery_never_returns_closed_as_a_partial_success() {
    // 设备在 initialize 之前就断开：报"断开"而不是"0 个工具的成功结果"。
    let mut t = MockTransport::new(vec![]);
    let mut next_id = 1;
    let (found, deferred) =
        discover_tools(&mut t, "s1", &mut next_id, Duration::from_millis(200)).await;
    let err = found.error.expect("断开必须回报");
    assert!(err.contains("initialize 未完成"), "{err}");
    assert!(err.contains("断开"), "文案要点明是断开而不是超时: {err}");
    assert!(found.tools.is_empty());
    assert!(deferred.is_empty());
}

// ============ 端到端（内存传输）：探测 → 注册表 → `GET /api/mcp/tools` 的 JSON ============

#[tokio::test]
async fn discovery_to_registry_to_endpoint_json_is_one_chain() {
    // 这条测试跨三层（协议 → 探测泵 → 注册表投影），断言的是**管理界面真正会看到的东西**。
    // 真机端到端（烧录固件 + 唤醒）在 CI 里做不到，这是能自动化到的最远一环：
    // 唯一没被覆盖的是 axum handler 本身（5 行包装）与真实 WebSocket 承载。
    let mut t = MockTransport::new(vec![
        // 真机时序：探测请求夹在音频帧中间（必须缓冲后补派发）
        IncomingFrame::Binary(vec![0xAA]),
        result_frame(1, server_info()),
        result_frame(
            2,
            serde_json::json!({
                "tools": [
                    {
                        "name": "self.audio_speaker.set_volume",
                        "description": "Set the volume of the audio speaker",
                        "inputSchema": {
                            "type": "object",
                            "properties": { "volume": { "type": "integer", "minimum": 0, "maximum": 100 } },
                            "required": ["volume"]
                        }
                    },
                    {
                        "name": "self.audio_listener.stop",
                        "description": "Stop listening",
                        "inputSchema": { "type": "object", "properties": {} },
                        "annotations": { "audience": ["user"] }
                    }
                ],
                "nextCursor": "self.reboot"
            }),
        ),
        result_frame(
            3,
            serde_json::json!({
                "tools": [
                    {
                        "name": "self.reboot",
                        "description": "Reboot the device",
                        "inputSchema": { "type": "object", "properties": {} },
                        "annotations": { "audience": ["user"] }
                    }
                ]
            }),
        ),
    ]);
    let mut next_id = 1;
    let (found, deferred) =
        discover_tools(&mut t, "s1", &mut next_id, Duration::from_millis(500)).await;
    assert_eq!(found.error, None);
    assert_eq!(deferred.len(), 1, "音频帧必须交回补派发");

    // 会话侧本来要做的事（`Session::discover_mcp_tools` 里的那一段）
    let reg = registry::McpRegistry::new();
    reg.record(ProbeResult {
        device_id: "aa:bb:cc".into(),
        session_id: "s1".into(),
        probed_at_ms: registry::now_ms(),
        board: found.server.name,
        firmware_version: found.server.version,
        protocol_version: found.server.protocol_version,
        tools: found.tools,
        error: found.error,
    });

    // 管理界面拿到的 JSON
    let v = reg.to_json();
    assert_eq!(v["ok"], serde_json::Value::Null, "ok 由端点补，注册表不管");
    assert_eq!(v["count"], 1);
    let d = &v["devices"][0];
    assert_eq!(d["device_id"], "aa:bb:cc");
    assert_eq!(d["board"], "bread-compact-wifi");
    assert_eq!(d["firmware_version"], "1.6.4");
    assert_eq!(d["protocol_version"], "2024-11-05");
    assert_eq!(d["online"], true);
    assert_eq!(d["tool_count"], 3, "两页合并");
    assert_eq!(d["ai_tool_count"], 1, "只有 set_volume 对 AI 可见");
    assert_eq!(
        d["user_tool_count"], 2,
        "audio_listener.stop 与 reboot 是 user_only"
    );
    assert!(
        d["age_ms"].as_u64().unwrap() < 5_000,
        "刚探测完，age 应该很小"
    );
    assert_eq!(d["error"], serde_json::Value::Null);
    // 工具本身：界面要显示的 name / signature / description 都在
    let tools = d["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 3);
    let volume = tools
        .iter()
        .find(|t| t["name"] == "self.audio_speaker.set_volume")
        .unwrap();
    assert_eq!(volume["user_only"], false);
    assert_eq!(volume["signature"], "volume: integer（必填，0~100）");
    assert_eq!(volume["description"], "Set the volume of the audio speaker");
    let stop = tools
        .iter()
        .find(|t| t["name"] == "self.audio_listener.stop")
        .unwrap();
    assert_eq!(stop["user_only"], true);
    assert_eq!(stop["signature"], "（无参数）");

    // 会话结束 → 界面从"在线"变"已断开"（保留工具清单）
    reg.mark_offline("aa:bb:cc", "s1");
    let after = reg.to_json();
    assert_eq!(after["devices"][0]["online"], false);
    assert_eq!(after["devices"][0]["tool_count"], 3, "断开不清空工具清单");
}
