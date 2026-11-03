//! voice 层纯函数单测：mock `DeviceRow` 与 DP 表，**无网络、无库**。
//!
//! 覆盖：名称匹配三级（alias 精确 / name 精确 / 双向包含）与优先级、query 过短
//! 拒识、歧义与上限、离线设备参与匹配、开关 DP 优先级与缺失、话术组装与失败
//! 三归类映射（架构 §2/§3/§5）。

use super::voice::{
    device_label, failure_speech, find_switch_dp, match_devices, normalize_name, speech_ambiguous,
    speech_not_found, speech_success, NameMatch,
};
use crate::device::tuya::store::DeviceRow;
use serde_json::json;
use std::collections::HashMap;

fn row(id: &str, name: &str, alias: &str, online: bool) -> DeviceRow {
    DeviceRow {
        device_id: id.into(),
        local_key: "key".into(),
        name: name.into(),
        model: String::new(),
        product_id: String::new(),
        ip: "192.168.1.10".into(),
        protocol_version: "3.3".into(),
        online,
        last_seen_ms: None,
        alias: alias.into(),
        updated_at: 0,
    }
}

fn unique(matched: NameMatch) -> DeviceRow {
    match matched {
        NameMatch::Unique(row) => row,
        other => panic!("expected unique hit, got {other:?}"),
    }
}

fn ambiguous(matched: NameMatch) -> Vec<DeviceRow> {
    match matched {
        NameMatch::Ambiguous(rows) => rows,
        other => panic!("expected ambiguous, got {other:?}"),
    }
}

#[test]
fn exact_alias_wins_over_everything() {
    let devices = vec![
        row("d1", "客厅风扇", "客厅", true),
        row("d2", "客厅", "", true),
    ];
    // alias 精确命中即短路：即便 name 精确也能命中 d2，也不看。
    assert_eq!(unique(match_devices("客厅", &devices)).device_id, "d1");
}

#[test]
fn exact_name_matches_when_alias_misses() {
    let devices = vec![row("d1", "客厅插座", "", true), row("d2", "卧室灯", "台灯", true)];
    assert_eq!(unique(match_devices("客厅插座", &devices)).device_id, "d1");
    // 别名精确
    assert_eq!(unique(match_devices("台灯", &devices)).device_id, "d2");
}

#[test]
fn bidirectional_contains_hits_short_and_long_queries() {
    let devices = vec![row("d1", "客厅插座", "", true)];
    // 话短名长：query ⊆ name
    assert_eq!(unique(match_devices("客厅", &devices)).device_id, "d1");
    // 话长名短（含「的」）：规范化后 name ⊆ query
    assert_eq!(unique(match_devices("客厅的插座", &devices)).device_id, "d1");
    assert_eq!(unique(match_devices("客厅 插座", &devices)).device_id, "d1");
    // 无交集
    assert_eq!(match_devices("卧室", &devices), NameMatch::NotFound);
}

#[test]
fn short_or_particle_queries_are_rejected() {
    let devices = vec![row("d1", "客厅插座", "的", true)];
    assert_eq!(match_devices("", &devices), NameMatch::NotFound);
    assert_eq!(match_devices("  ", &devices), NameMatch::NotFound);
    // 规范化后不足 2 字符的碎词直接拒识，防误命中。
    assert_eq!(match_devices("的", &devices), NameMatch::NotFound);
    assert_eq!(match_devices("开", &devices), NameMatch::NotFound);
}

#[test]
fn contains_multi_hits_are_ambiguous_and_offline_devices_participate() {
    let devices = vec![
        row("d1", "客厅插座", "", false),
        row("d2", "客厅灯", "吊灯", true),
    ];
    let rows = ambiguous(match_devices("客厅", &devices));
    assert_eq!(rows.len(), 2);
    // 离线设备参与匹配（PRD 用户故事 4：提示离线而非未找到）。
    assert!(rows.iter().any(|r| r.device_id == "d1" && !r.online));
    // 歧义话术列出标签且提示具体些。
    let speech = speech_ambiguous("客厅", &rows);
    assert!(speech.contains("客厅插座") && speech.contains("吊灯"));
    assert!(speech.contains("更具体"));
    assert!(speech.contains("「客厅」"));
}

#[test]
fn ambiguous_speech_lists_at_most_three_candidates() {
    let devices: Vec<DeviceRow> = (1..=5)
        .map(|i| row(&format!("d{i}"), &format!("客厅设备{i}"), "", true))
        .collect();
    let rows = ambiguous(match_devices("客厅设备", &devices));
    assert_eq!(rows.len(), 5);
    let speech = speech_ambiguous("客厅设备", &rows);
    assert!(!speech.contains("客厅设备4"), "只列前 3 个候选: {speech}");
}

#[test]
fn label_prefers_alias_then_cloud_name() {
    assert_eq!(device_label(&row("d1", "云名称", "我的别名", true)), "我的别名");
    assert_eq!(device_label(&row("d1", "云名称", "  ", true)), "云名称");
}

#[test]
fn switch_dp_prefers_id_one_then_any_bool_deterministically() {
    let mut dps = HashMap::new();
    assert_eq!(find_switch_dp(&dps), None, "空缓存无 DP");
    dps.insert("2".into(), json!(30));
    assert_eq!(find_switch_dp(&dps), None, "无 bool DP 归不支持");
    dps.insert("2".into(), json!(true));
    assert_eq!(find_switch_dp(&dps).as_deref(), Some("2"));
    // id=="1" 优先（哪怕还另有 bool DP）。
    dps.insert("1".into(), json!(false));
    dps.insert("5".into(), json!(true));
    assert_eq!(find_switch_dp(&dps).as_deref(), Some("1"));
}

#[test]
fn speech_assembly_covers_success_and_failure_kinds() {
    assert_eq!(speech_success(true, "客厅插座"), "已打开客厅插座。");
    assert_eq!(speech_success(false, "客厅插座"), "已关闭客厅插座。");
    let not_found = speech_not_found(" 存在的插座 ");
    assert!(not_found.contains("存在的插座") && not_found.contains("管理台"));
    // 三归类一一映射（架构 §5 后三行），不透传引擎内部 advice。
    assert_eq!(failure_speech("offline", "客厅插座"), "客厅插座现在离线，请检查设备供电和网络后再试。");
    assert_eq!(failure_speech("timeout", "客厅插座"), "客厅插座没有响应，请稍后再试。");
    assert_eq!(
        failure_speech("unsupported", "客厅插座"),
        "客厅插座不是可开关的设备，暂时不支持语音开关。"
    );
    // 未知 kind 归 offline 兜底（三类是完整口径，防御性收口）。
    assert_eq!(failure_speech("whatever", "客厅插座"), failure_speech("offline", "客厅插座"));
}

#[test]
fn normalization_removes_whitespace_and_particle() {
    assert_eq!(normalize_name("  客厅 的 插座 "), "客厅插座");
    assert_eq!(normalize_name("客厅插座"), "客厅插座");
    assert!(normalize_name("的").is_empty());
}
