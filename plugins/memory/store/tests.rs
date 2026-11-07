//! `plugins/memory/store.rs` 的回归测试（逻辑见 `store.rs`；测试与逻辑分文件见 `AGENTS.md` §5.10）。
//!
//! 这些测试直接打 SQLite：无网络、无模型，跑得快，且能精确验证
//! "上游会踩的坑我们没踩"（尤其是保留策略误删被引用的证据）。

use super::*;
use crate::plugins::memory::MemoryConfig;

fn cfg(db: &str) -> MemoryConfig {
    MemoryConfig {
        enabled: true,
        db_path: db.to_string(),
        ..MemoryConfig::default()
    }
}

fn mem(scope: &str, sess: &str, turn: i64, summary: &str) -> NewMemory {
    NewMemory {
        scope: scope.to_string(),
        session_id: sess.to_string(),
        turn_index: turn,
        summary: summary.to_string(),
        outcome: "completed".to_string(),
        keywords: vec![],
    }
}

/// 写一轮原文 + 一条记忆（返回该轮的 message id）。
fn write_turn(
    st: &Store,
    scope: &str,
    sess: &str,
    turn: i64,
    user: &str,
    assistant: &str,
    summary: &str,
) -> Vec<String> {
    let now = 1_000i64 * turn;
    let ids = st
        .insert_turn(scope, sess, turn, user, assistant, now)
        .expect("写入原文");
    st.upsert_memory(&mem(scope, sess, turn, summary), &ids, now)
        .expect("写入记忆");
    ids
}

#[test]
fn migrations_are_idempotent_on_same_file() {
    let dir = std::env::temp_dir().join(format!("xzm-store-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("memory.db");
    let p = path.to_str().unwrap();
    let a = open(&cfg(p)).expect("首次打开");
    drop(a);
    let b = open(&cfg(p)).expect("再次打开（迁移应幂等）");
    let v: i64 = b
        .conn()
        .query_row("SELECT MAX(version) FROM _migrations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, schema::LATEST_VERSION);
    drop(b);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn insert_turn_and_search_roundtrip_with_evidence() {
    let st = open(&cfg(":memory:")).unwrap();
    write_turn(
        &st,
        "xiaozhi:dev1",
        "sess-1",
        1,
        "我下周三要去杭州出差",
        "好的，记得带上充电器",
        "用户计划下周三去杭州出差",
    );
    let hits = st
        .search(
            &crate::plugins::memory::terms::query_terms("杭州出差要注意什么"),
            Some("xiaozhi:dev1"),
            0,
            6,
        )
        .expect("召回");
    assert_eq!(hits.len(), 1, "应命中 1 条: {hits:?}");
    assert!(hits[0].summary.contains("杭州"));
    assert!(hits[0].hits >= 2, "应匹配多个词项: {}", hits[0].hits);
    // 证据层必须是**原始 Q/A**（不是摘要）
    let roles: Vec<&str> = hits[0].sources.iter().map(|(r, _)| r.as_str()).collect();
    assert_eq!(roles, vec!["user", "assistant"]);
    assert!(hits[0].sources[0].1.contains("杭州出差"));
}

/// 回归：主查询应一次性返回 summary/outcome/turn_index，且与写入值逐字一致
/// （折叠 per-hit 摘要 query_row 后的字段等价性）。
#[test]
fn search_returns_summary_fields_matching_written_values() {
    let st = open(&cfg(":memory:")).unwrap();
    let ids = st
        .insert_turn(
            "xiaozhi:dev1",
            "sess-1",
            3,
            "我下周三要去杭州出差",
            "好的，记得带上充电器",
            3_000,
        )
        .expect("写入原文");
    let m = NewMemory {
        scope: "xiaozhi:dev1".into(),
        session_id: "sess-1".into(),
        turn_index: 3,
        summary: "用户计划下周三去杭州出差，需带充电器".into(),
        outcome: "rescheduled".into(),
        keywords: vec![],
    };
    let memory_id = st.upsert_memory(&m, &ids, 3_000).expect("写入记忆");
    let hits = st
        .search(
            &crate::plugins::memory::terms::query_terms("杭州出差"),
            Some("xiaozhi:dev1"),
            0,
            6,
        )
        .expect("召回");
    assert_eq!(hits.len(), 1, "应命中 1 条: {hits:?}");
    let h = &hits[0];
    assert_eq!(h.memory_id, memory_id);
    assert_eq!(h.summary, "用户计划下周三去杭州出差，需带充电器");
    assert_eq!(h.outcome, "rescheduled");
    assert_eq!(h.turn_index, 3);
}

#[test]
fn search_without_scope_spans_all_devices() {
    let st = open(&cfg(":memory:")).unwrap();
    write_turn(
        &st,
        "xiaozhi:dev1",
        "s",
        1,
        "客厅的温度是 26 度",
        "记下了",
        "用户说客厅温度 26 度",
    );
    write_turn(
        &st,
        "xiaozhi:dev2",
        "s",
        1,
        "书房的温度是 20 度",
        "记下了",
        "用户说书房温度 20 度",
    );
    let q = crate::plugins::memory::terms::query_terms("温度是多少");
    let scoped = st.search(&q, Some("xiaozhi:dev1"), 0, 6).unwrap();
    assert_eq!(scoped.len(), 1, "按 scope 只应命中本设备");
    let all = st.search(&q, None, 0, 6).unwrap();
    assert_eq!(all.len(), 2, "不限定 scope 时跨设备命中");
}

#[test]
fn fresh_turn_window_excludes_recent_memories() {
    let st = open(&cfg(":memory:")).unwrap();
    write_turn(
        &st,
        "xiaozhi:dev1",
        "s",
        1,
        "我喜欢吃辣",
        "记住啦",
        "用户喜欢吃辣",
    );
    write_turn(
        &st,
        "xiaozhi:dev1",
        "s",
        2,
        "还是吃辣",
        "好",
        "用户又说到吃辣",
    );
    let q = crate::plugins::memory::terms::query_terms("吃辣");
    // fresh_turn_count = 0：最近轮次不排除 → 两条都召回
    assert_eq!(st.search(&q, Some("xiaozhi:dev1"), 0, 6).unwrap().len(), 2);
    // fresh_turn_count = 1：最新一轮（turn 2）已被排除 → 只剩 turn 1
    let hits = st.search(&q, Some("xiaozhi:dev1"), 1, 6).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].turn_index, 1);
}

#[test]
fn retention_never_deletes_referenced_evidence() {
    // 这是上游的已知缺陷：runMessageRetention 只看 legacy 表，会把轮次记忆引用的原文删掉。
    let st = open(&cfg(":memory:")).unwrap();
    write_turn(
        &st,
        "xiaozhi:dev1",
        "s",
        1,
        "用户在写一个 Rust 服务端",
        "加油",
        "用户在写 Rust 服务端",
    );
    // 第二轮没有抽取（没有记忆引用它）→ 可被保留策略清理
    st.insert_turn("xiaozhi:dev1", "s", 2, "随便聊聊", "嗯嗯", 2_000)
        .unwrap();

    let dry = st.retention(KeepPolicy::Referenced, 0, 0, true).unwrap();
    assert_eq!(dry, 2, "演练应报出 2 条（第二轮的两条原文）");
    assert_eq!(st.counts().unwrap().messages, 4, "演练不得真的删除");

    let deleted = st.retention(KeepPolicy::Referenced, 0, 0, false).unwrap();
    assert_eq!(deleted, 2);
    let c = st.counts().unwrap();
    assert_eq!(c.messages, 2, "只应删掉未被引用的第二轮");
    assert_eq!(c.memories, 1, "记忆本身保留");

    // 证据层仍然可召回 —— 这正是上游缺陷会造成"证据层变空"的地方
    let hits = st
        .search(
            &crate::plugins::memory::terms::query_terms("Rust 服务端"),
            None,
            0,
            6,
        )
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].sources.len(), 2, "被引用的证据必须还在");
}

#[test]
fn retention_recent_keeps_recent_unreferenced_messages() {
    let st = open(&cfg(":memory:")).unwrap();
    for turn in 1..=5 {
        st.insert_turn(
            "xiaozhi:dev1",
            "s",
            turn,
            &format!("第{turn}轮"),
            "好",
            1_000 * turn,
        )
        .unwrap();
    }
    // 保留最近 2 轮 → 删除第 1~3 轮（共 6 条）
    let deleted = st.retention(KeepPolicy::Recent, 2, 0, false).unwrap();
    assert_eq!(deleted, 6);
    assert_eq!(st.counts().unwrap().messages, 4);
}

#[test]
fn clear_only_touches_given_scope() {
    let st = open(&cfg(":memory:")).unwrap();
    write_turn(&st, "xiaozhi:dev1", "s", 1, "甲", "甲答", "甲摘要");
    write_turn(&st, "xiaozhi:dev2", "s", 1, "乙", "乙答", "乙摘要");
    let n = st.clear(Some("xiaozhi:dev1")).unwrap();
    assert_eq!(n, 1);
    assert_eq!(st.counts().unwrap().memories, 1);
    assert_eq!(st.scopes().unwrap(), vec![("xiaozhi:dev2".to_string(), 1)]);
    // 全清
    let n = st.clear(None).unwrap();
    assert_eq!(n, 1);
    assert_eq!(st.counts().unwrap().memories, 0);
}

#[test]
fn upsert_memory_is_idempotent_and_rebuilds_index() {
    let st = open(&cfg(":memory:")).unwrap();
    let ids = st
        .insert_turn("xiaozhi:dev1", "s", 1, "旧问题", "旧回答", 1)
        .unwrap();
    st.upsert_memory(&mem("xiaozhi:dev1", "s", 1, "用户提过蓝色雨伞"), &ids, 1)
        .unwrap();
    // 同一轮再次抽取（换了摘要）：应覆盖而不是新增，且词项索引重建
    st.upsert_memory(
        &mem("xiaozhi:dev1", "s", 1, "改过的摘要：杭州出差"),
        &ids,
        2,
    )
    .unwrap();
    let c = st.counts().unwrap();
    assert_eq!(c.memories, 1, "同一轮只能有一条记忆");
    let hits = st
        .search(
            &crate::plugins::memory::terms::query_terms("杭州出差"),
            None,
            0,
            6,
        )
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].summary.contains("杭州"));
    let old = st
        .search(
            &crate::plugins::memory::terms::query_terms("蓝色雨伞"),
            None,
            0,
            6,
        )
        .unwrap();
    assert!(old.is_empty(), "旧摘要的词项必须被清掉: {old:?}");
}

#[test]
fn quarantine_release_respects_attempt_limit() {
    let st = open(&cfg(":memory:")).unwrap();
    st.insert_turn("xiaozhi:dev1", "s", 1, "甲", "甲答", 1)
        .unwrap();
    st.insert_turn("xiaozhi:dev1", "s", 2, "乙", "乙答", 2)
        .unwrap();
    let key = PendingTurn {
        scope: "xiaozhi:dev1".into(),
        session_id: "s".into(),
        turn_index: 1,
    };
    // 第 1 次失败（上限 1）→ 直接隔离
    let (state, attempts) = st.mark_turn_failed(&key, "上游 503", 1, 100).unwrap();
    assert_eq!((state.as_str(), attempts), ("quarantined", 1));
    assert_eq!(
        st.counts().unwrap().quarantined,
        2,
        "该轮两条原文都标记隔离"
    );
    // attempts(1) >= max(1) → 不释放（永久隔离，等人工）
    assert_eq!(st.release_quarantine(1, 10_000).unwrap(), 0);
    // 上限放宽 → 放回队列（`maintain` 之后的轮次会重新尝试抽取）
    assert_eq!(st.release_quarantine(2, 10_000).unwrap(), 2);
    // 4 = 被释放的 turn 1 两条 + 本来就在队列里的 turn 2 两条
    assert_eq!(st.counts().unwrap().pending, 4);
    assert_eq!(st.counts().unwrap().quarantined, 0);

    // 未到上限的失败仍是 pending（安排退避重试），不会进隔离。
    // 注意 attempts 是**累计**的（释放重试不会清零）：这是刻意行为，
    // 否则"隔离 → 释放 → 再失败 → 释放"会无限循环。
    let (state, attempts) = st.mark_turn_failed(&key, "上游 503", 3, 20_000).unwrap();
    assert_eq!((state.as_str(), attempts), ("pending", 2));
    assert_eq!(st.counts().unwrap().quarantined, 0);
}

#[test]
fn turn_index_is_monotonic_per_scope() {
    let st = open(&cfg(":memory:")).unwrap();
    assert_eq!(st.next_turn_index("xiaozhi:dev1").unwrap(), 1);
    st.insert_turn("xiaozhi:dev1", "s", 1, "甲", "答", 1)
        .unwrap();
    assert_eq!(st.next_turn_index("xiaozhi:dev1").unwrap(), 2);
    // 不同 scope 各自独立
    assert_eq!(st.next_turn_index("xiaozhi:dev2").unwrap(), 1);
}

#[test]
fn stable_ids_follow_sha256_scheme_and_are_deterministic() {
    let a = schema::stable_id("tm", &["xiaozhi:dev1", "s", "1"]);
    let b = schema::stable_id("tm", &["xiaozhi:dev1", "s", "1"]);
    assert_eq!(a, b);
    assert!(a.starts_with("tm-"));
    assert_eq!(a.len(), 3 + 32);
    // 分隔符必须参与：否则 ("ab","c") 与 ("a","bc") 会撞
    assert_ne!(
        schema::stable_id("tm", &["ab", "c"]),
        schema::stable_id("tm", &["a", "bc"])
    );
}

// ============================================================
// P1-7：insert_turn_state 参数化 + 轮次计数 + store/graph 数据隔离
// ============================================================

/// `'stored'` 状态的消息**不进** graph 抽取队列（成本惊吓防护的存储层保证）。
#[test]
fn stored_state_is_invisible_to_extraction_queue() {
    let st = open(&cfg(":memory:")).unwrap();
    // graph 路径：'pending' → 进队列
    st.insert_turn("xiaozhi:dev1", "s1", 1, "问一", "答一", 1_000)
        .unwrap();
    assert!(
        st.pending_turn(2_000).unwrap().is_some(),
        "pending 状态应在抽取队列里"
    );
    // store 路径：'stored' → 队列看不见
    st.insert_turn_state("xiaozhi:dev1", "s1", 2, "问二", "答二", "stored", 2_000)
        .unwrap();
    let queue = st.pending_turn(3_000).unwrap();
    assert_eq!(
        queue.as_ref().map(|p| p.turn_index),
        Some(1),
        "队列队首应仍是 turn 1，stored 的 turn 2 不得出现: {queue:?}"
    );
    // 标记 turn 1 完成后队列即空（stored 的 turn 2 不会被补进队列）
    let done = PendingTurn {
        scope: "xiaozhi:dev1".into(),
        session_id: "s1".into(),
        turn_index: 1,
    };
    st.mark_turn(&done, "succeeded", None, None, 3_000).unwrap();
    assert!(
        st.pending_turn(4_000).unwrap().is_none(),
        "stored 轮次绝不能进入抽取队列"
    );
}

/// turn_count：按 (scope, session, turn) 去重的轮次计数（每轮 2 条消息）。
#[test]
fn turn_count_dedupes_messages_into_turns() {
    let st = open(&cfg(":memory:")).unwrap();
    assert_eq!(st.turn_count().unwrap(), 0);
    st.insert_turn("xiaozhi:dev1", "s1", 1, "甲", "答", 1)
        .unwrap();
    st.insert_turn("xiaozhi:dev1", "s1", 2, "乙", "答", 2)
        .unwrap();
    st.insert_turn_state("xiaozhi:dev2", "s1", 1, "丙", "答", "stored", 3)
        .unwrap();
    // 3 轮（跨 scope 计数），不是 6 条消息
    assert_eq!(st.turn_count().unwrap(), 3);
    // 幂等重复写入同一轮（INSERT OR IGNORE）不重复计数
    st.insert_turn("xiaozhi:dev1", "s1", 1, "甲", "答", 1)
        .unwrap();
    assert_eq!(st.turn_count().unwrap(), 3);
}

/// counts() 的新 `stored` 字段：pending/succeeded 与 stored 各自计数、互不串。
#[test]
fn counts_tracks_stored_separately() {
    let st = open(&cfg(":memory:")).unwrap();
    st.insert_turn("xiaozhi:dev1", "s1", 1, "问", "答", 1_000)
        .unwrap();
    st.insert_turn_state("xiaozhi:dev1", "s1", 2, "问二", "答二", "stored", 2_000)
        .unwrap();
    let c = st.counts().unwrap();
    assert_eq!(c.messages, 4, "共 4 条消息（两轮各 2 条）");
    assert_eq!(c.pending, 2, "graph 路径的两条是 pending");
    assert_eq!(c.stored, 2, "store 路径的两条是 stored");
    assert_eq!(st.turn_count().unwrap(), 2);
}

/// message_scope_counts：按**原文轮次**列 scope（store 状态页口径），与记忆口径分开。
#[test]
fn message_scope_counts_lists_turns_per_scope() {
    let st = open(&cfg(":memory:")).unwrap();
    st.insert_turn("xiaozhi:dev1", "s1", 1, "甲", "答", 1)
        .unwrap();
    st.insert_turn("xiaozhi:dev1", "s1", 2, "乙", "答", 2)
        .unwrap();
    st.insert_turn_state("xiaozhi:dev2", "s1", 1, "丙", "答", "stored", 3)
        .unwrap();
    let scopes = st.message_scope_counts().unwrap();
    assert_eq!(
        scopes,
        vec![
            ("xiaozhi:dev1".to_string(), 2),
            ("xiaozhi:dev2".to_string(), 1),
        ],
        "按轮次（非消息条数）计数、按数量降序: {scopes:?}"
    );
    // 空库 → 空列表
    let empty = open(&cfg(":memory:")).unwrap();
    assert!(empty.message_scope_counts().unwrap().is_empty());
}

/// graph ⇄ store 数据保留（PRD 验收 3 的存储层基础）：同一 db 下两种状态共存互不污染。
#[test]
fn stored_and_graph_rows_coexist_in_same_db() {
    let st = open(&cfg(":memory:")).unwrap();
    // graph 写入 + 抽取成功
    let ids = st
        .insert_turn("xiaozhi:dev1", "s1", 1, "杭州出差", "好的", 1_000)
        .unwrap();
    st.upsert_memory(
        &mem("xiaozhi:dev1", "s1", 1, "用户计划杭州出差"),
        &ids,
        1_000,
    )
    .unwrap();
    let done = PendingTurn {
        scope: "xiaozhi:dev1".into(),
        session_id: "s1".into(),
        turn_index: 1,
    };
    st.mark_turn(&done, "succeeded", None, None, 1_500).unwrap();
    // store 写入
    st.insert_turn_state("xiaozhi:dev1", "s2", 1, "今天天气", "晴天", "stored", 2_000)
        .unwrap();
    // graph 数据完好：召回仍命中
    let hits = st
        .search(
            &crate::plugins::memory::terms::query_terms("杭州出差"),
            None,
            0,
            6,
        )
        .unwrap();
    assert_eq!(hits.len(), 1, "graph 记忆不受 store 写入影响");
    let c = st.counts().unwrap();
    assert_eq!((c.memories, c.stored), (1, 2));
    assert_eq!(st.turn_count().unwrap(), 2);
}
