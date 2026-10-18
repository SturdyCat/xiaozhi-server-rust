//! `server/src/app/mcp/discovery.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（`AGENTS.md` §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。
//!
//! 报文样例**逐字取自固件**：`initialize` 回包取 `main/mcp_server.cc` 的 `ParseMessage`
//! （`"{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{\"tools\":{}},\"serverInfo\":{...}}"`），
//! 工具项取 `main/mcp_server.h` 的 `McpTool::to_json`，不是臆造的形态。

use super::*;

/// 固件 `initialize` 回包的 `result`（原样照抄 `mcp_server.cc` 的拼接结果）。
fn firmware_initialize_result() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "bread-compact-wifi", "version": "1.6.4" }
    })
}

#[test]
fn initialize_is_spec_shaped_and_firmware_compatible() {
    let req = initialize(3, json!({}));
    assert_eq!(req["jsonrpc"], JSONRPC_VERSION);
    assert_eq!(req["method"], "initialize");
    assert_eq!(req["id"], 3);
    assert!(req["id"].is_u64(), "id 必须是数字，字符串会被固件整条丢弃");
    // 固件只读 `params.capabilities.vision`，但要求 `params` **是对象**（否则整条丢弃并打 log）
    assert!(req["params"].is_object(), "params 必须是对象");
    assert!(req["params"]["capabilities"].is_object());
    assert_eq!(req["params"]["protocolVersion"], PROTOCOL_VERSION);
    // clientInfo 固件不读，但按 MCP 规范该带（换实现时不会被当成缺步骤）
    assert_eq!(req["params"]["clientInfo"]["name"], env!("CARGO_PKG_NAME"));
}

#[test]
fn initialized_notification_is_a_notification() {
    let n = initialized_notification();
    assert_eq!(n["method"], "notifications/initialized");
    // 通知**没有 id**（JSON-RPC 2.0 语义：不期待响应）；固件对 notifications/* 直接 return
    assert!(n.get("id").is_none(), "通知不能带 id：{n}");
    assert_eq!(n["jsonrpc"], JSONRPC_VERSION);
}

#[test]
fn tools_list_carries_cursor_and_user_tools_flag() {
    let first = tools_list(4, "", true);
    assert_eq!(first["method"], "tools/list");
    assert_eq!(first["params"]["cursor"], "");
    assert_eq!(first["params"]["withUserTools"], true);
    assert!(
        first["params"]["withUserTools"].is_boolean(),
        "固件只认 bool"
    );
    assert_eq!(first["id"], 4);

    let second = tools_list(5, "self.lamp.turn_on", false);
    assert_eq!(second["params"]["cursor"], "self.lamp.turn_on");
    assert_eq!(second["params"]["withUserTools"], false);
}

#[test]
fn parse_server_info_reads_board_and_firmware_version() {
    let info = parse_server_info(&firmware_initialize_result());
    assert_eq!(info.protocol_version, "2024-11-05");
    // 固件 serverInfo.name = BOARD_NAME 宏（板型名）——管理界面显示"这是什么板子"
    assert_eq!(info.name, "bread-compact-wifi");
    // 固件 serverInfo.version = esp_app_get_description()->version（固件版本）
    assert_eq!(info.version, "1.6.4");
    assert!(info.tools_capability, "capabilities.tools 存在");
    assert!(
        info.summary().contains("bread-compact-wifi"),
        "{}",
        info.summary()
    );
}

#[test]
fn parse_server_info_tolerates_missing_fields() {
    // 宽容读取是刻意的：拿不到板型/版本只是少一行信息，工具清单才是目的
    let info = parse_server_info(&json!({}));
    assert_eq!(info, McpServerInfo::default());
    assert!(!info.tools_capability);
    assert!(info.summary().contains("未报"), "{}", info.summary());
}

#[test]
fn parse_tools_reads_firmware_tool_shape() {
    // 固件 `McpTool::to_json`：{name, description, inputSchema}（+ user_only 时带 annotations）
    let page = parse_tools(&json!({
        "tools": [
            {
                "name": "self.audio_speaker.set_volume",
                "description": "Set the volume of the audio speaker",
                "inputSchema": {
                    "type": "object",
                    "properties": { "volume": { "type": "integer", "description": "0-100" } },
                    "required": ["volume"]
                }
            },
            {
                "name": "self.audio_listener.stop",
                "description": "Stop listening",
                "inputSchema": { "type": "object", "properties": {} },
                "annotations": { "audience": ["user"] }
            }
        ]
    }));
    assert_eq!(page.tools.len(), 2);
    assert_eq!(page.tools[0].name, "self.audio_speaker.set_volume");
    assert_eq!(
        page.tools[0].description,
        "Set the volume of the audio speaker"
    );
    assert_eq!(page.tools[0].input_schema["required"][0], "volume");
    assert!(!page.tools[0].user_only, "无 annotations 的工具对 AI 可见");
    // user_only 的唯一可靠信号是 annotations.audience 含 "user"
    assert!(
        page.tools[1].user_only,
        "带 audience=[user] 的必须标成 user_only"
    );
    assert_eq!(page.next_cursor, None, "没有 nextCursor 就是最后一页");
    assert_eq!(page.skipped, 0);
}

#[test]
fn parse_tools_reports_pagination_and_skips_malformed_entries() {
    // 固件 `GetToolsList`：nextCursor = **下一页第一个工具的名字**（包含语义）
    let page = parse_tools(&json!({
        "tools": [
            { "name": "a", "description": "", "inputSchema": {} },
            { "description": "缺 name 的条目无法调用" },
            { "name": "   ", "description": "只有空白也不算" }
        ],
        "nextCursor": "self.lamp.turn_on"
    }));
    assert_eq!(page.tools.len(), 1);
    assert_eq!(page.tools[0].name, "a");
    assert_eq!(page.next_cursor.as_deref(), Some("self.lamp.turn_on"));
    // 跳过畸形条目必须**回报**（静默丢工具会让界面显示的能力与设备实际不符）
    assert_eq!(page.skipped, 2);
}

#[test]
fn parse_tools_ignores_empty_cursor_and_missing_tools() {
    // 空串 nextCursor = 没有下一页（固件只在还有剩余时写该字段，但不排除空串）
    let page = parse_tools(&json!({ "tools": [], "nextCursor": "  " }));
    assert!(page.tools.is_empty());
    assert_eq!(page.next_cursor, None);
    // `tools` 缺失/类型不对：不该 panic，返回空页（设备畸形报文由上层记 warn）
    assert!(parse_tools(&json!({})).tools.is_empty());
    assert!(parse_tools(&json!({ "tools": "oops" })).tools.is_empty());
}

#[test]
fn tool_json_keeps_description_and_schema_for_the_ui() {
    let page = parse_tools(&json!({
        "tools": [{ "name": "x", "description": "d", "inputSchema": { "type": "object" } }]
    }));
    let v = page.tools[0].to_json();
    assert_eq!(v["name"], "x");
    assert_eq!(v["description"], "d");
    assert_eq!(v["user_only"], false);
    assert_eq!(v["input_schema"]["type"], "object");
    // 无参数工具也要给一句明确的话（客户端不该为"空"另写分支）
    assert_eq!(v["signature"], "（无参数）");
    assert!(v["params"].as_array().unwrap().is_empty());
}

#[test]
fn schema_params_read_the_firmware_property_keywords() {
    // 固件 `Property::to_json` 的全部键（mcp_server.h:173-205）：type / default /
    // minimum / maximum（integer）/ maxLength（string）——没有参数级 description。
    let schema = json!({
        "type": "object",
        "properties": {
            "volume": { "type": "integer", "default": 50, "minimum": 0, "maximum": 100 },
            "text": { "type": "string", "maxLength": 32 },
            "force": { "type": "boolean", "default": false },
            "silent": { "type": "boolean" }
        },
        "required": ["text", "silent"]
    });
    let params = schema_params(&schema);
    assert_eq!(params.len(), 4, "字母序：force / silent / text / volume");
    let by = |n: &str| params.iter().find(|p| p.name == n).unwrap();

    assert_eq!(by("volume").ty, "integer");
    assert!(
        !by("volume").required,
        "有 default 即非必填（固件 GetRequired 口径）"
    );
    assert_eq!(by("volume").default, Some(json!(50)));
    assert_eq!(by("volume").minimum, Some(0));
    assert_eq!(by("volume").maximum, Some(100));
    assert_eq!(
        by("volume").signature(),
        "volume: integer（默认 50，0~100）"
    );

    assert_eq!(by("text").max_length, Some(32));
    assert!(by("text").required, "required 数组里有它");
    assert_eq!(by("text").signature(), "text: string（必填，最长 32 字）");

    assert_eq!(by("force").signature(), "force: boolean（默认 false）");
    assert_eq!(by("silent").signature(), "silent: boolean（必填）");
}

#[test]
fn schema_params_are_tolerant_of_odd_schemas() {
    // 没有 properties / properties 类型不对 / 只有 required：都不该 panic，返回空表
    assert!(schema_params(&json!({})).is_empty());
    assert!(schema_params(&json!({ "type": "object" })).is_empty());
    assert!(schema_params(&json!({ "properties": [] })).is_empty());
    // 参数缺 type：给 "any" 而不是空串（界面能显示"未声明类型"）
    let p = schema_params(&json!({ "properties": { "x": {} } }));
    assert_eq!(p[0].ty, "any");
    assert_eq!(p[0].signature(), "x: any");
    // 只声明下界 / 只声明上界
    let p = schema_params(&json!({
        "properties": { "a": { "type": "integer", "minimum": 1 }, "b": { "type": "integer", "maximum": 9 } }
    }));
    assert_eq!(
        p.iter().find(|x| x.name == "a").unwrap().signature(),
        "a: integer（≥1）"
    );
    assert_eq!(
        p.iter().find(|x| x.name == "b").unwrap().signature(),
        "b: integer（≤9）"
    );
}

#[test]
fn tool_json_exposes_a_one_line_signature_for_curl_and_the_ui() {
    let page = parse_tools(&json!({
        "tools": [{
            "name": "self.audio_speaker.set_volume",
            "description": "Set the volume of the audio speaker",
            "inputSchema": {
                "type": "object",
                "properties": { "volume": { "type": "integer", "minimum": 0, "maximum": 100 } },
                "required": ["volume"]
            }
        }]
    }));
    let v = page.tools[0].to_json();
    assert_eq!(v["signature"], "volume: integer（必填，0~100）");
    assert_eq!(v["params"][0]["name"], "volume");
    assert_eq!(v["params"][0]["required"], true);
    // 缺的约束一律给 null（客户端不必区分"缺失"与"空"）
    assert!(v["params"][0]["default"].is_null());
    assert!(v["params"][0]["max_length"].is_null());
}

#[test]
fn string_defaults_are_quoted_in_the_signature() {
    // 空串默认值必须看得见是"空串"，否则会被读成"没有默认值"
    let p = schema_params(&json!({
        "properties": { "reply": { "type": "string", "default": "" } }
    }));
    assert_eq!(p[0].signature(), "reply: string（默认 \"\"）");
}

#[test]
fn max_pages_is_a_small_sane_bound() {
    // 真机上全部工具（含 user_only）只有个位数、8000 字节一页 → 正常 1~2 页；
    // 这个上界只为防"设备回 nextCursor 却不推进"的死循环，不该小到截断正常设备。
    assert!((2..=32).contains(&MAX_PAGES), "MAX_PAGES={MAX_PAGES}");
}
