//! `server/src/app/usage.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::*;

fn usage(input: u64, output: u64) -> Option<TokenUsage> {
    Some(TokenUsage {
        input,
        output,
        ..Default::default()
    })
}

#[test]
fn accumulates_globally_and_per_session() {
    let m = UsageMeter::new();
    m.record("s1", usage(100, 20));
    m.record("s1", usage(50, 10));
    m.record("s2", usage(7, 3));

    let g = m.global();
    assert_eq!(g.turns, 3);
    assert_eq!(g.usage.pressure(), 157);
    assert_eq!(g.usage.output, 33);

    let s1 = m.session("s1").expect("s1 应有记录");
    assert_eq!(s1.turns, 2);
    assert_eq!(s1.usage.pressure(), 150);
    assert_eq!(m.session("s2").unwrap().usage.total(), 10);
    assert!(m.session("nope").is_none());
}

/// 上游不给 usage 时：只计轮次，不污染 token 累计（不能把"未知"当 0 混进去）。
#[test]
fn missing_usage_counts_turn_only() {
    let m = UsageMeter::new();
    m.record("s1", None);
    let g = m.global();
    assert_eq!(g.turns, 0, "没有 usage 的轮次不计入已计量轮次");
    assert_eq!(g.usage.total(), 0);
    assert_eq!(m.session("s1").unwrap().turns, 0);
}

/// 会话数有上界（长时间运行不会无界增长）。
#[test]
fn session_entries_are_bounded() {
    let m = UsageMeter::new();
    for i in 0..(MAX_SESSIONS + 10) {
        m.record(&format!("s{i}"), usage(1, 1));
    }
    let json = m.to_json();
    assert_eq!(json["sessions"].as_array().unwrap().len(), MAX_SESSIONS);
    // 最旧的被淘汰，最新的还在
    assert!(m.session("s0").is_none());
    assert!(m.session(&format!("s{}", MAX_SESSIONS + 9)).is_some());
    // 全局累计不受淘汰影响
    assert_eq!(m.global().turns as usize, MAX_SESSIONS + 10);
}

#[test]
fn json_shape_exposes_pressure() {
    let m = UsageMeter::new();
    m.record(
        "s1",
        Some(TokenUsage {
            input: 10,
            output: 4,
            cache_read: 90,
            cache_write: 0,
        }),
    );
    let v = m.to_json();
    assert_eq!(
        v["global"]["usage"]["pressure"], 100,
        "压力=输入侧总量，不含输出"
    );
    assert_eq!(v["global"]["usage"]["output"], 4);
    assert_eq!(v["sessions"][0]["session_id"], "s1");
}
