//! `server/src/app/mcp/registry.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（`AGENTS.md` §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。
//!
//! 重点锚住两条**容易静默出错**的规则：
//! ① 探测失败必须保留上次的好数据（否则界面会从"有 5 个工具"变成"什么都没有"，像设备被清空了）；
//! ② `mark_offline` 不能被**旧会话**收尾误伤（设备重连后新会话已探测过）。

use super::*;
use crate::workflow::mcp::discovery::McpToolInfo;

fn tool(name: &str, user_only: bool) -> McpToolInfo {
    McpToolInfo {
        name: name.to_string(),
        description: format!("{name} 的说明"),
        input_schema: json!({ "type": "object" }),
        user_only,
    }
}

fn probe(device_id: &str, session_id: &str, at: u64, tools: Vec<McpToolInfo>) -> ProbeResult {
    ProbeResult {
        device_id: device_id.to_string(),
        session_id: session_id.to_string(),
        probed_at_ms: at,
        board: "bread-compact-wifi".into(),
        firmware_version: "1.6.4".into(),
        protocol_version: "2024-11-05".into(),
        tools,
        error: None,
    }
}

#[test]
fn record_stores_tools_and_projects_counts() {
    let reg = McpRegistry::new();
    reg.record(probe(
        "aa:bb",
        "s1",
        1_000,
        vec![
            tool("self.audio_speaker.set_volume", false),
            tool("self.reboot", true),
        ],
    ));
    let d = reg.get("aa:bb").expect("登记后应能查到");
    assert_eq!(d.board, "bread-compact-wifi");
    assert_eq!(d.firmware_version, "1.6.4");
    assert!(d.online, "刚探测完这一刻设备在线");
    assert_eq!(d.error, None);

    let v = d.to_json(1_500);
    assert_eq!(v["tool_count"], 2);
    assert_eq!(v["user_tool_count"], 1, "user_only 工具数");
    assert_eq!(v["ai_tool_count"], 1, "对大模型可见的工具数");
    assert_eq!(v["age_ms"], 500, "age 在序列化时算，注册表只存事实时间戳");
    assert_eq!(v["tools"][0]["name"], "self.audio_speaker.set_volume");
}

#[test]
fn failed_probe_keeps_last_good_tools() {
    // 这是本模块的核心规则：探测失败（一个工具都没取到 + 带 error）**不能**把上次结果抹掉，
    // 否则界面会从"5 个工具"变成"什么都没有"，看起来像设备被清空了。
    let reg = McpRegistry::new();
    reg.record(probe(
        "aa:bb",
        "s1",
        1_000,
        vec![tool("self.lamp.turn_on", false)],
    ));
    let mut failed = probe("aa:bb", "s2", 2_000, vec![]);
    failed.error = Some("initialize 未完成: 等待设备 MCP 响应超时（800ms）".into());
    failed.board = String::new();
    failed.firmware_version = String::new();
    reg.record(failed);

    let d = reg.get("aa:bb").unwrap();
    assert_eq!(d.tools.len(), 1, "上次的工具清单必须保留");
    assert_eq!(d.board, "bread-compact-wifi", "板型也保留");
    assert_eq!(d.firmware_version, "1.6.4", "固件版本也保留");
    assert!(d.error.is_some(), "失败原因要显示出来");
    assert_eq!(d.probed_at_ms, 2_000, "时间戳更新为本次尝试");
    assert_eq!(d.session_id, "s2");
    assert!(d.online);
}

#[test]
fn partial_success_replaces_tools_and_keeps_the_reason() {
    // 取到了工具但第 2 页超时：按新结果整体替换（已取到的是真数据），同时保留原因提示不完整
    let reg = McpRegistry::new();
    reg.record(probe("aa:bb", "s1", 1_000, vec![tool("old", false)]));
    let mut partial = probe("aa:bb", "s2", 2_000, vec![tool("new", false)]);
    partial.error = Some("tools/list 未完成（已取到 1 个工具）: 超时".into());
    reg.record(partial);

    let d = reg.get("aa:bb").unwrap();
    assert_eq!(d.tools.len(), 1);
    assert_eq!(d.tools[0].name, "new", "取到数据就替换，不做合并");
    assert!(d.error.is_some());
}

#[test]
fn failed_probe_without_previous_record_still_reports_the_error() {
    let reg = McpRegistry::new();
    let mut failed = probe("cc:dd", "s1", 5_000, vec![]);
    failed.error = Some("设备拒绝 initialize（code=-32601）: Method not implemented".into());
    reg.record(failed);
    let d = reg
        .get("cc:dd")
        .expect("失败也要登记：界面才能显示'这台设备探测失败了'");
    assert!(d.tools.is_empty());
    assert!(d.error.unwrap().contains("-32601"));
}

#[test]
fn mark_offline_only_applies_to_the_probing_session() {
    let reg = McpRegistry::new();
    reg.record(probe("aa:bb", "s1", 1_000, vec![]));
    // 旧会话收尾（s1）在新会话（s2）已探测之后到达：不能把"在线"抹掉
    reg.record(probe("aa:bb", "s2", 2_000, vec![]));
    reg.mark_offline("aa:bb", "s1");
    assert!(
        reg.get("aa:bb").unwrap().online,
        "旧会话不得把新会话标成离线"
    );

    reg.mark_offline("aa:bb", "s2");
    assert!(!reg.get("aa:bb").unwrap().online);
    // 未登记的设备：静默无操作（会话收尾会对测试台等非设备会话调用它）
    reg.mark_offline("never-seen", "s9");
    assert_eq!(reg.len(), 1);
}

#[test]
fn registry_is_bounded() {
    let reg = McpRegistry::new();
    for i in 0..(MAX_DEVICES + 5) {
        reg.record(probe(&format!("dev-{i}"), "s", i as u64, vec![]));
    }
    assert_eq!(reg.len(), MAX_DEVICES, "长时间运行不得无界增长");
    assert!(reg.get("dev-0").is_none(), "淘汰最旧的一条");
    assert!(reg.get("dev-4").is_none());
    assert!(
        reg.get(&format!("dev-{}", MAX_DEVICES + 4)).is_some(),
        "最新的必须在"
    );
}

#[test]
fn to_json_orders_most_recently_probed_first() {
    let reg = McpRegistry::new();
    reg.record(probe("old", "s1", 1_000, vec![]));
    reg.record(probe("new", "s2", 2_000, vec![]));
    let v = reg.to_json();
    assert_eq!(v["count"], 2);
    assert_eq!(v["devices"][0]["device_id"], "new", "最近探测的在前面");
    assert_eq!(v["devices"][1]["device_id"], "old");
    // 空注册表也要给出可行动的说明（"刚部署 / 设备还没唤醒"是正常现象，不是错误）
    assert!(McpRegistry::new().to_json()["devices"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(v["note"].as_str().unwrap().contains("会话开始时"));
}

/// P1-10：`has_tool` 三态——None=清单未知（未登记/探测失败），Some=清单在手（有/没有）。
#[test]
fn has_tool_distinguishes_unknown_from_absent() {
    let reg = McpRegistry::new();
    // 未登记 → 未知
    assert_eq!(reg.has_tool("no-device", "self.audio_listener.stop"), None);

    // 探测成功且含 stop → Some(true)
    reg.record(probe(
        "dev-ok",
        "s1",
        1_000,
        vec![tool("self.audio_listener.stop", true)],
    ));
    assert_eq!(
        reg.has_tool("dev-ok", "self.audio_listener.stop"),
        Some(true)
    );
    assert_eq!(
        reg.has_tool("dev-ok", "self.reboot"),
        Some(false),
        "清单在手但无此工具"
    );

    // 探测失败（零工具 + error）→ 清单未知（保持试探路径，不误判为"没有"）
    let mut bad = probe("dev-bad", "s2", 2_000, vec![]);
    bad.error = Some("第 2 页超时".into());
    reg.record(bad);
    assert_eq!(
        reg.has_tool("dev-bad", "self.audio_listener.stop"),
        None,
        "探测失败 = 没问出来，不是没有"
    );

    // 探测成功但清单为空 → 设备明确没有工具 → Some(false)（不是 None）
    reg.record(probe("dev-empty", "s3", 3_000, vec![]));
    assert_eq!(
        reg.has_tool("dev-empty", "self.audio_listener.stop"),
        Some(false)
    );
}

/// P1-10：`command_gate` 三态标记（管理台「指令闸门兼容」列的数据源）。
#[test]
fn command_gate_json_marks_compatibility() {
    let reg = McpRegistry::new();
    reg.record(probe(
        "dev-ok",
        "s1",
        1_000,
        vec![tool("self.audio_listener.stop", true)],
    ));
    reg.record(probe(
        "dev-old",
        "s2",
        1_000,
        vec![tool("self.reboot", true)],
    ));
    let mut bad = probe("dev-bad", "s3", 1_000, vec![]);
    bad.error = Some("超时".into());
    reg.record(bad);

    let v = reg.to_json();
    let find = |id: &str| {
        v["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["device_id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(find("dev-ok")["command_gate"], "compatible");
    assert_eq!(find("dev-old")["command_gate"], "will_degrade");
    assert_eq!(
        find("dev-bad")["command_gate"],
        "unknown",
        "探测失败 = 未知"
    );
}
