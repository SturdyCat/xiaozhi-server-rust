//! `store_only.rs`（P1-7 纯存储后端）的端到端测试：**假 LLM + 真 SQLite**。
//!
//! 覆盖设计 §1.2.8 护栏 3/4 与 PRD 验收 1/3：
//! 1. record 只落 `extraction_state='stored'`，摘要/倒排（`gm_turn_memories`）零写入；
//! 2. `extract_pending` 恒零调用、`recall` 恒空（零辅助 LLM 成本的结构保证）；
//! 3. graph ⇄ store ⇄ graph 切换（同一 db 文件）数据保留，且切回 graph 不突发抽取。
//!
//! 注意：graph/tests.rs 的 FakeExtractor 是其私有模块，此处自建极简版。

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use anyhow::Result;

use super::*;
use crate::plugins::llm::{LlmEvent, LlmProvider, LlmTurnResult, ToolSpec, TurnPrompt};
use crate::plugins::memory::{CompletedTurn, GraphMemory, MaintainOpts, MemoryProvider, RetentionConfig};

/// 极简假 LLM：固定成功回复（供 graph 阶段抽取用），记录调用次数。
struct FakeOkLlm {
    calls: AtomicU32,
}

impl FakeOkLlm {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicU32::new(0),
        })
    }

    fn calls(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }
}

impl LlmProvider for FakeOkLlm {
    fn chat_stream<'a>(
        &'a self,
        _history: &'a [(String, String)],
        _user_text: &'a str,
        _prompt: &'a TurnPrompt,
        _tools: Option<&'a [ToolSpec]>,
        mut on_event: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<LlmTurnResult>> + Send + 'a>>
    {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let text = "```json\n{\"summary\":\"测试摘要\",\"outcome\":\"information\"}\n```"
                .to_string();
            on_event(LlmEvent::Text(text.clone()));
            Ok(LlmTurnResult {
                text,
                tool_calls: Vec::new(),
                usage: None,
                cancelled: false,
            })
        })
    }
}

fn turn(scope: &str, user: &str, assistant: &str) -> CompletedTurn {
    CompletedTurn {
        scope: scope.to_string(),
        session_id: "sess-1".to_string(),
        user_text: user.to_string(),
        assistant_text: assistant.to_string(),
    }
}

/// 临时文件 db（graph ⇄ store 切换必须共享同一文件，`:memory:` 每次 open 独立）。
fn temp_db(tag: &str) -> String {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "xiaozhi-store-only-{}-{}-{}.db",
        std::process::id(),
        tag,
        SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    path.to_string_lossy().into_owned()
}

#[tokio::test]
async fn record_writes_stored_only_and_extract_recall_are_noop() {
    let cfg = MemoryConfig {
        enabled: true,
        db_path: ":memory:".into(),
        ..MemoryConfig::default()
    };
    let mem = StoreOnlyMemory::new(&cfg).unwrap();

    mem.record(turn(
        "xiaozhi:dev1",
        "我下周三要去杭州出差",
        "好的，记得带上充电器和身份证",
    ))
    .await
    .unwrap();

    let stats = mem.stats();
    assert_eq!(stats.engine, "store");
    assert!(stats.active);
    assert_eq!(stats.messages, 2, "一轮 = 2 条原文");
    assert_eq!(stats.stored, 2, "原文应标记为 stored");
    assert_eq!(stats.pending, 0, "stored 不进抽取队列");
    assert_eq!(stats.memories, 0, "摘要零写入");
    assert_eq!(stats.turns, 1);

    // 抽取恒零、召回恒空（PRD 验收 1/2 的行为面）。
    let out = mem.extract_pending(10).await;
    assert_eq!(out, ExtractOutcome::default(), "store 恒无抽取");
    let r = mem.recall(Some("xiaozhi:dev1"), "杭州出差要注意什么").await;
    assert!(r.block.is_none() && r.hits == 0, "store 恒不召回");
    let stats = mem.stats();
    assert_eq!(stats.stored, 2, "noop 操作不应改变数据");
}

#[tokio::test]
async fn switch_graph_store_graph_preserves_data_without_burst_extraction() {
    let db = temp_db("switch");
    let cfg = || MemoryConfig {
        enabled: true,
        db_path: db.clone(),
        ..MemoryConfig::default()
    };

    // 阶段 1：graph 正常抽取（产生 1 条记忆）。
    let fake = FakeOkLlm::new();
    let graph = GraphMemory::with_llm(&cfg(), fake.clone()).unwrap();
    graph
        .record(turn("xiaozhi:dev1", "我喜欢喝美式咖啡", "好的，记住了"))
        .await
        .unwrap();
    let out = graph.extract_pending(1).await;
    assert_eq!((out.attempted, out.succeeded), (1, 1));
    assert_eq!(fake.calls(), 1);
    drop(graph);

    // 阶段 2：切到 store 继续积累（stored 落同一库）。
    let store_mem = StoreOnlyMemory::new(&cfg()).unwrap();
    store_mem
        .record(turn(
            "xiaozhi:dev1",
            "我下周三要去杭州出差",
            "记得带充电器",
        ))
        .await
        .unwrap();
    let stats = store_mem.stats();
    assert_eq!(stats.stored, 2, "store 阶段写入的原文");
    assert_eq!(stats.memories, 1, "graph 阶段的记忆保留");
    drop(store_mem);

    // 阶段 3：切回 graph——stored 行不进抽取队列，无突发 LLM 调用。
    let fake2 = FakeOkLlm::new();
    let graph2 = GraphMemory::with_llm(&cfg(), fake2.clone()).unwrap();
    let stats = graph2.stats();
    assert_eq!(stats.pending, 0, "stored 行对抽取队列不可见");
    assert_eq!(stats.memories, 1, "既有记忆不丢");
    assert_eq!(stats.turns, 2, "两轮原文都在");
    let out = graph2.extract_pending(10).await;
    assert_eq!(
        out,
        ExtractOutcome::default(),
        "切回 graph 不应突发抽取（成本惊吓防护）"
    );
    assert_eq!(fake2.calls(), 0, "零辅助 LLM 调用");

    let _ = std::fs::remove_file(&db);
}

#[tokio::test]
async fn maintain_applies_retention_to_stored_turns() {
    let cfg = MemoryConfig {
        enabled: true,
        db_path: ":memory:".into(),
        maintenance_interval: 2, // 2 轮即触发 periodic 维护
        retention: RetentionConfig {
            keep: "recent".into(),
            recent_turns: 1, // 只留最近 1 轮
            ..Default::default()
        },
        ..MemoryConfig::default()
    };
    let mem = StoreOnlyMemory::new(&cfg).unwrap();

    mem.record(turn("xiaozhi:dev1", "第一轮问题", "第一轮回答"))
        .await
        .unwrap();
    mem.record(turn("xiaozhi:dev1", "第二轮问题", "第二轮回答"))
        .await
        .unwrap();
    assert_eq!(mem.stats().turns, 2);

    // 未到间隔时跳过：先记 1 轮时 turn_count=2 → 2%2==0 反而会跑，
    // 所以这里直接验证 forced dry_run 与真实执行两条路径。
    let dry = mem.maintain(MaintainOpts::forced(true)).await.unwrap();
    assert!(dry.ran && dry.dry_run, "forced 演练应执行");
    assert_eq!(mem.stats().turns, 2, "dry_run 不删数据");

    let real = mem.maintain(MaintainOpts::forced(false)).await.unwrap();
    assert!(real.ran && !real.dry_run);
    let stats = mem.stats();
    assert_eq!(
        stats.turns, 1,
        "recent=1 应只保留最近 1 轮（store 的保留策略对原文生效）"
    );
    assert_eq!(real.deleted, 2, "被删的是 1 轮 = 2 条原文");
}

#[tokio::test]
async fn clear_scoped_and_global_on_store() {
    let cfg = MemoryConfig {
        enabled: true,
        db_path: ":memory:".into(),
        ..MemoryConfig::default()
    };
    let mem = StoreOnlyMemory::new(&cfg).unwrap();
    mem.record(turn("xiaozhi:a", "A1", "R1")).await.unwrap();
    mem.record(turn("xiaozhi:b", "B1", "R1")).await.unwrap();
    assert_eq!(mem.stats().turns, 2);

    // ⚠️ 返回口径与 graph 一致 = gm_turn_memories 条数；store 库恒无记忆 → 恒 0。
    // 清空是否生效以 stats（原文）为准。
    let deleted = mem.clear(Some("xiaozhi:a".into())).await.unwrap();
    assert_eq!(deleted, 0, "store 无轮次记忆，返回口径恒 0");
    let stats = mem.stats();
    assert_eq!(stats.turns, 1, "scope 清空生效：原文只剩另一 scope");
    assert_eq!(stats.stored, 2);

    let _deleted = mem.clear(None).await.unwrap();
    assert_eq!(mem.stats().turns, 0, "全局清空生效");
}
