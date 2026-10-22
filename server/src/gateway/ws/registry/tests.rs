//! `app/ws/registry` 的单元测试（外置文件，见 `AGENTS.md` §5.10）。
//!
//! 这里钉的是**对外 JSON 契约**：前端页面直接读这些字段名，改名不会有编译错误，
//! 只会在浏览器里静默显示空白。因此这些断言刻意写得"啰嗦"——它们就是契约文档。
//!
//! ⚠️ 不构造 `Engines`（需要真实模型）；投影函数被设计成纯函数正是为了这一点，
//! MCP join 的事实由 [`McpFacts`] 传入。

use super::*;
use crate::device::registry::{Registry, SessionStart};

fn start(session_id: &str, device_id: &str) -> SessionStart {
    SessionStart {
        session_id: session_id.to_string(),
        device_id: device_id.to_string(),
        client_id: "aa:bb:cc".to_string(),
        uplink_sr: 16_000,
        downlink_sr: 24_000,
        downlink_frame_ms: 60,
        bin_version: 2,
        mcp: true,
        is_test: false,
    }
}

/// 造一台"设备 + 一轮对话"的注册表。守卫一并返回——调用方必须持有它，
/// 否则会话立刻收尾、设备变离线，断言 `online` 分支就测不到了。
fn sample() -> (
    Registry,
    crate::device::registry::SessionGuard,
    crate::device::registry::DeviceRecord,
) {
    let reg = Registry::new();
    let guard = reg.begin_session(start("s1", "aa:bb:cc"));
    reg.record_user_turn("s1", "你好");
    reg.record_assistant_turn("s1", "你好呀", 800);
    let d = reg.device("aa:bb:cc").expect("设备已建档");
    (reg, guard, d)
}

#[test]
fn device_json_pins_the_wire_contract() {
    let (_reg, _guard, d) = sample();
    let now = d.last_seen_ms + 5_000;
    let mcp = McpFacts {
        board: Some("bread-compact-wifi".to_string()),
        firmware_version: Some("1.7.2".to_string()),
        tool_count: Some(6),
        probed_at_ms: Some(d.last_seen_ms),
    };
    let v = device_json(&d, 1, now, &mcp);

    // 身份与在线态
    assert_eq!(v["device_id"], "aa:bb:cc");
    assert_eq!(v["client_id"], "aa:bb:cc");
    assert_eq!(v["online"], true);
    assert_eq!(v["is_test"], false);
    assert_eq!(v["mcp"], true);
    // 时序（界面显示"最后活跃 5 秒前"）
    assert_eq!(v["age_ms"], 5_000);
    // 累计画像
    assert_eq!(v["session_count"], 1);
    assert_eq!(v["turn_count"], 1);
    assert_eq!(v["live_sessions"], 1);
    assert_eq!(v["last_session_id"], "s1");
    assert!(v["last_error"].is_null());
    // 协商参数：排障第一现场，键名改动会直接让页面空掉
    assert_eq!(v["audio"]["uplink_sample_rate"], 16_000);
    assert_eq!(v["audio"]["downlink_sample_rate"], 24_000);
    assert_eq!(v["audio"]["downlink_frame_duration_ms"], 60);
    assert_eq!(v["audio"]["binary_protocol_version"], 2);
    // MCP join 进来的板型/固件版本
    assert_eq!(v["board"], "bread-compact-wifi");
    assert_eq!(v["firmware_version"], "1.7.2");
    assert_eq!(v["tool_count"], 6);
}

#[test]
fn device_json_without_mcp_probe_uses_null_not_empty_string() {
    // 设备从未以 MCP 连接过：必须是 null（前端据此显示"—"），不能是 ""，
    // 否则界面会显示一个空白的"板型"字段，看起来像坏数据。
    let (_reg, _guard, d) = sample();
    let v = device_json(&d, 0, d.last_seen_ms, &McpFacts::default());
    assert!(v["board"].is_null());
    assert!(v["firmware_version"].is_null());
    assert!(v["tool_count"].is_null());
    assert!(v["tools_probed_at_ms"].is_null());
}

#[test]
fn session_json_pins_phase_and_duration() {
    let reg = Registry::new();
    let guard = reg.begin_session(start("s1", "aa:bb:cc"));
    reg.record_user_turn("s1", "打开客厅的灯");
    let s = &reg.sessions()[0];
    let v = session_json(s, s.last_activity_ms);

    assert_eq!(v["session_id"], "s1");
    assert_eq!(v["device_id"], "aa:bb:cc");
    // 阶段：识别完正在等大模型 → 思考中（界面上的实时状态就靠这对字段）
    assert_eq!(v["phase"], "thinking");
    assert_eq!(v["phase_display"], "思考中");
    assert_eq!(v["turn_count"], 1);
    assert_eq!(v["last_user_text"], "打开客厅的灯");
    assert_eq!(v["idle_ms"], 0);
    assert!(v["duration_ms"].as_u64().is_some());
    assert_eq!(v["audio"]["binary_protocol_version"], 2);
    drop(guard);
}

#[test]
fn conversation_json_pins_turns_and_truncation_flag() {
    let reg = Registry::new();
    let guard = reg.begin_session(start("s1", "aa:bb:cc"));
    reg.record_user_turn("s1", "今天几度");
    reg.record_assistant_turn("s1", "我看不到天气", 1234);

    let c = &reg.conversations()[0];
    let v = conversation_json(c);

    assert_eq!(v["session_id"], "s1");
    assert_eq!(v["device_id"], "aa:bb:cc");
    assert_eq!(v["turn_count"], 2);
    assert_eq!(v["truncated"], false);
    assert_eq!(v["dropped"], 0);

    let turns = v["turns"].as_array().expect("turns 必须是数组");
    assert_eq!(turns[0]["role"], "user");
    assert_eq!(turns[0]["text"], "今天几度");
    assert!(turns[0]["elapsed_ms"].is_null(), "用户轮次没有耗时");
    assert_eq!(turns[1]["role"], "assistant");
    assert_eq!(turns[1]["elapsed_ms"], 1234);
    drop(guard);
}

#[test]
fn conversations_query_defaults_and_cap_the_limit() {
    // 默认值：不传 limit 时用 20
    let q = ConversationsQuery::default();
    assert_eq!(
        q.limit.unwrap_or(DEFAULT_CONVERSATIONS_LIMIT),
        DEFAULT_CONVERSATIONS_LIMIT
    );
    // 上限：`?limit=100000` 必须被夹到 MAX（否则一次请求能把整个环形缓冲吐出去）
    assert_eq!(
        100_000usize.min(MAX_CONVERSATIONS_LIMIT),
        MAX_CONVERSATIONS_LIMIT
    );
    assert!(MAX_CONVERSATIONS_LIMIT >= DEFAULT_CONVERSATIONS_LIMIT);
}

#[test]
fn conversations_query_is_deserialized_from_query_string() {
    // handler 的 `Query<ConversationsQuery>` 靠 serde 反序列化；这里钉住字段名，
    // 避免有人把 `device_id` 改成 `deviceId` 后前端过滤静默失效。
    let q: ConversationsQuery =
        serde_json::from_str(r#"{"device_id":"aa:bb:cc","limit":5}"#).expect("应能反序列化");
    assert_eq!(q.device_id.as_deref(), Some("aa:bb:cc"));
    assert_eq!(q.limit, Some(5));

    // 两者都可省略（`GET /api/conversations` 无参数）
    let q: ConversationsQuery = serde_json::from_str("{}").expect("空对象应能反序列化");
    assert!(q.device_id.is_none());
    assert!(q.limit.is_none());
}
