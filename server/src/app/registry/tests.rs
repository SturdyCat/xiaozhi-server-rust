//! `app/registry` 的单元测试（外置文件，见 `AGENTS.md` §5.10）。
//!
//! 覆盖的都是"错了界面就会骗人"的语义：在线状态翻转、重连竞态、上界淘汰、轮次环形截断。
//! JSON 投影不在这里测（那是 `app/ws/registry.rs` 的职责）。

use super::*;

/// 造一条会话登记请求（测试里只关心少数几个字段）。
fn start(session_id: &str, device_id: &str) -> SessionStart {
    SessionStart {
        session_id: session_id.to_string(),
        device_id: device_id.to_string(),
        client_id: "client-1".to_string(),
        uplink_sr: 16_000,
        downlink_sr: 24_000,
        downlink_frame_ms: 60,
        bin_version: 1,
        mcp: true,
        is_test: false,
    }
}

#[test]
fn begin_session_registers_device_and_live_session() {
    let reg = Registry::new();
    let _guard = reg.begin_session(start("s1", "dev-a"));

    let d = reg.device("dev-a").expect("设备应已建档");
    assert!(d.online, "会话开始后设备应在在线");
    assert_eq!(d.session_count, 1);
    assert_eq!(d.last_session_id, "s1");
    assert_eq!(d.uplink_sr, 16_000);
    assert_eq!(d.downlink_sr, 24_000);

    let sessions = reg.sessions();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "s1");
    assert_eq!(sessions[0].phase, Phase::Idle);
    assert_eq!(sessions[0].turn_count, 0);
}

#[test]
fn guard_drop_removes_session_and_marks_device_offline() {
    let reg = Registry::new();
    {
        let _guard = reg.begin_session(start("s1", "dev-a"));
        assert_eq!(reg.session_count(), 1);
    }
    // guard 出作用域：会话摘除、设备离线
    assert_eq!(reg.session_count(), 0, "守卫 Drop 后活会话必须为 0");
    let d = reg.device("dev-a").unwrap();
    assert!(!d.online, "最后一个会话结束后设备必须离线");
    assert_eq!(d.session_count, 1, "累计会话数不该被收尾抹掉");
}

#[test]
fn reconnect_is_not_marked_offline_by_old_session() {
    // 设备重连：旧会话的收尾**不能**把新会话的在线状态抹掉（否则界面显示假离线）。
    let reg = Registry::new();
    let old = reg.begin_session(start("s1", "dev-a"));
    let _new = reg.begin_session(start("s2", "dev-a"));

    drop(old); // 旧会话结束

    let d = reg.device("dev-a").unwrap();
    assert!(d.online, "新会话还活着，设备必须仍显示在线");
    assert_eq!(d.last_session_id, "s2");
    assert_eq!(d.session_count, 2, "重连应累计会话数");
    assert_eq!(reg.session_count(), 1);
}

#[test]
fn close_session_is_idempotent() {
    let reg = Registry::new();
    let _guard = reg.begin_session(start("s1", "dev-a"));
    reg.close_session("s1");
    reg.close_session("s1"); // 重复调用不得 panic、不得影响设备画像
    reg.close_session("never-existed");
    assert_eq!(reg.session_count(), 0);
    assert_eq!(reg.device("dev-a").unwrap().session_count, 1);
}

#[test]
fn turns_land_in_conversation_and_counts() {
    let reg = Registry::new();
    let _guard = reg.begin_session(start("s1", "dev-a"));

    reg.record_user_turn("s1", "今天天气怎么样");
    // 识别完成 → 交给大模型，阶段应为"思考中"
    assert_eq!(reg.sessions()[0].phase, Phase::Thinking);
    assert_eq!(reg.sessions()[0].turn_count, 1);
    assert_eq!(
        reg.sessions()[0].last_user_text.as_deref(),
        Some("今天天气怎么样")
    );

    reg.record_assistant_turn("s1", "我看不到天气，你手机上看一眼？", 1234);
    assert_eq!(reg.sessions()[0].phase, Phase::Idle);

    let conv = &reg.conversations()[0];
    assert_eq!(conv.turns.len(), 2);
    assert_eq!(conv.turns[0].role, "user");
    assert_eq!(conv.turns[0].text, "今天天气怎么样");
    assert_eq!(conv.turns[0].elapsed_ms, None);
    assert_eq!(conv.turns[1].role, "assistant");
    assert_eq!(conv.turns[1].elapsed_ms, Some(1234));
    assert!(!conv.truncated());

    // 设备画像也累计了轮次
    assert_eq!(reg.device("dev-a").unwrap().turn_count, 1);
}

#[test]
fn turn_ring_truncates_and_reports_dropped() {
    let reg = Registry::new();
    let _guard = reg.begin_session(start("s1", "dev-a"));

    let extra = 5;
    for i in 0..(MAX_TURNS_PER_SESSION + extra) {
        reg.record_user_turn("s1", &format!("提问 {i}"));
    }

    let conv = &reg.conversations()[0];
    assert_eq!(
        conv.turns.len(),
        MAX_TURNS_PER_SESSION,
        "轮次必须被环形截断"
    );
    assert!(conv.truncated());
    assert_eq!(conv.dropped, extra as u64, "丢掉的条数要如实记下");
    // 保留的是**最近**的：最后一条应是最后写入的那句
    assert_eq!(
        conv.turns.last().unwrap().text,
        format!("提问 {}", MAX_TURNS_PER_SESSION + extra - 1)
    );
}

#[test]
fn devices_are_evicted_at_the_upper_bound() {
    let reg = Registry::new();
    // 每条会话立刻收尾：只压设备表，不压会话表
    for i in 0..(MAX_DEVICES + 3) {
        drop(reg.begin_session(start(&format!("s{i}"), &format!("dev-{i}"))));
    }
    assert_eq!(reg.device_count(), MAX_DEVICES, "设备表必须有上界");
    // 最旧的被淘汰、最新的还在
    assert!(reg.device("dev-0").is_none());
    assert!(reg.device(&format!("dev-{}", MAX_DEVICES + 2)).is_some());
}

#[test]
fn conversations_are_evicted_at_the_upper_bound() {
    let reg = Registry::new();
    for i in 0..(MAX_CONVERSATION_SESSIONS + 4) {
        let guard = reg.begin_session(start(&format!("s{i}"), "dev-a"));
        reg.record_user_turn(&format!("s{i}"), &format!("第 {i} 句"));
        drop(guard);
    }
    assert_eq!(
        reg.conversations().len(),
        MAX_CONVERSATION_SESSIONS,
        "保留对话的会话数必须有上界"
    );
    // 最近的在最前
    let first = &reg.conversations()[0];
    assert_eq!(
        first.session_id,
        format!("s{}", MAX_CONVERSATION_SESSIONS + 3)
    );
}

#[test]
fn counts_distinguish_devices_from_sessions() {
    let reg = Registry::new();
    let _a1 = reg.begin_session(start("s1", "dev-a"));
    let _a2 = reg.begin_session(start("s2", "dev-a")); // 同设备两条连接
    let _b = reg.begin_session(start("s3", "dev-b"));

    let c = reg.counts();
    assert_eq!(c.devices, 2);
    assert_eq!(c.sessions, 3);
    assert_eq!(c.online, 2, "在线数按**设备**去重，不是连接数");
    assert_eq!(c.test_sessions, 0);
    assert_eq!(c.conversations, 3);
}

#[test]
fn test_bench_sessions_are_flagged_not_counted_as_devices() {
    let reg = Registry::new();
    let mut s = start("s1", "bench");
    s.is_test = true;
    let _g = reg.begin_session(s);

    let c = reg.counts();
    assert_eq!(c.test_sessions, 1);
    // 测试台连接仍然是"一条会话"，但界面可据此区分（它不是设备）
    let row = &reg.sessions()[0];
    assert!(row.is_test);
}

#[test]
fn sessions_of_and_conversations_of_filter_by_device() {
    let reg = Registry::new();
    let _a = reg.begin_session(start("s1", "dev-a"));
    let _b = reg.begin_session(start("s2", "dev-b"));
    reg.record_user_turn("s1", "a 说的");
    reg.record_user_turn("s2", "b 说的");

    assert_eq!(reg.sessions_of("dev-a").len(), 1);
    assert_eq!(reg.sessions_of("dev-a")[0].session_id, "s1");

    let convs = reg.conversations_of("dev-b", 10);
    assert_eq!(convs.len(), 1);
    assert_eq!(convs[0].turns[0].text, "b 说的");

    // limit 生效
    assert_eq!(reg.conversations_of("dev-b", 0).len(), 0);
}

#[test]
fn error_is_attached_to_session_then_device() {
    let reg = Registry::new();
    let guard = reg.begin_session(start("s1", "dev-a"));
    reg.record_error("s1", "LLM 调用失败（上游 401）");
    assert_eq!(
        reg.sessions()[0].last_error.as_deref(),
        Some("LLM 调用失败（上游 401）")
    );
    drop(guard);
    assert_eq!(
        reg.device("dev-a").unwrap().last_error.as_deref(),
        Some("LLM 调用失败（上游 401）"),
        "会话结束应把异常带到设备画像上（断电后仍看得到原因）"
    );
}

#[test]
fn clean_session_end_clears_previous_error() {
    let reg = Registry::new();
    // 第一次会话**异常**结束 → 设备画像带上原因
    let g1 = reg.begin_session(start("s1", "dev-a"));
    reg.record_error("s1", "LLM 调用失败（上游 401）");
    drop(g1);
    assert_eq!(
        reg.device("dev-a").unwrap().last_error.as_deref(),
        Some("LLM 调用失败（上游 401）")
    );

    // 新会话开始：上一条会话的异常**仍然可见**——"上次为什么断"正是重连时最该看到的信息
    let g2 = reg.begin_session(start("s2", "dev-a"));
    assert!(
        reg.device("dev-a").unwrap().last_error.is_some(),
        "新会话开始时不该抹掉上一条会话的异常原因"
    );

    // 干净结束 → 清空（修好之后错误提示不该永远挂在设备上）
    drop(g2);
    assert!(
        reg.device("dev-a").unwrap().last_error.is_none(),
        "会话干净结束必须清掉历史异常"
    );
}

#[test]
fn ordering_is_deterministic_within_the_same_millisecond() {
    // 回归测试：一轮快循环里所有记录都落在同一毫秒。若用时间戳当排序键，顺序取决于
    // `HashMap` 的**随机迭代顺序**（实测确实会乱），管理页的"最近活跃在前"就会随机跳。
    // 排序键必须是单调序号。
    let reg = Registry::new();
    for i in 0..8 {
        drop(reg.begin_session(start(&format!("s{i}"), &format!("dev-{i}"))));
    }

    let devices: Vec<String> = reg.devices().iter().map(|d| d.device_id.clone()).collect();
    assert_eq!(
        devices,
        vec!["dev-7", "dev-6", "dev-5", "dev-4", "dev-3", "dev-2", "dev-1", "dev-0"],
        "同毫秒内设备顺序必须确定（最近活跃在前）"
    );

    let convs: Vec<String> = reg
        .conversations()
        .iter()
        .map(|c| c.session_id.clone())
        .collect();
    assert_eq!(convs[0], "s7", "最近建立的会话必须排在最前，且顺序确定");
}

#[test]
fn set_phase_ignores_unknown_session() {
    let reg = Registry::new();
    // 不该 panic，也不该凭空造出会话
    reg.set_phase("nope", Phase::Speaking);
    reg.record_error("nope", "x");
    assert_eq!(reg.session_count(), 0);
}

#[test]
fn phase_labels_are_stable() {
    // 这几个字符串是对外 JSON 契约（前端/OtaState 之类的展示依赖它们），改动即破坏接口
    assert_eq!(Phase::Idle.as_str(), "idle");
    assert_eq!(Phase::Listening.as_str(), "listening");
    assert_eq!(Phase::Thinking.as_str(), "thinking");
    assert_eq!(Phase::Speaking.as_str(), "speaking");
    assert_eq!(Phase::Listening.display(), "聆听中");
}
