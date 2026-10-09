//! `plugins/memory/graph.rs` 的端到端测试：**假 LLM + 真 SQLite**，无网络、无模型。
//!
//! 覆盖的是最容易在真机才发现的三件事：
//! 1. 抽取成功 → 记忆入库 → 用"换了说法的提问"仍能召回，且注入的是**原始 Q/A**；
//! 2. 抽取失败 → 隔离、对话不受影响（`record`/`extract_pending` 都不向上抛）；
//! 3. 近况窗口内的轮次不再重复召回（省 token 的关键）。

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;

use super::*;
use crate::plugins::llm::{LlmProvider, LlmTurnResult, ToolSpec, TurnPrompt};
use crate::plugins::memory::{CompletedTurn, MemoryProvider};

/// 可编程的假抽取器：固定回复 / 固定失败，并记录调用次数。
struct FakeExtractor {
    reply: Mutex<String>,
    fail: bool,
    calls: AtomicU32,
}

impl FakeExtractor {
    fn ok(summary: &str, outcome: &str) -> Arc<Self> {
        Arc::new(Self {
            reply: Mutex::new(format!(
                "```json\n{{\"summary\":\"{summary}\",\"outcome\":\"{outcome}\"}}\n```"
            )),
            fail: false,
            calls: AtomicU32::new(0),
        })
    }

    fn failing() -> Arc<Self> {
        Arc::new(Self {
            reply: Mutex::new(String::new()),
            fail: true,
            calls: AtomicU32::new(0),
        })
    }

    fn calls(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }
}

impl LlmProvider for FakeExtractor {
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
            if self.fail {
                anyhow::bail!("上游 503（假失败）");
            }
            let text = self.reply.lock().unwrap_or_else(|e| e.into_inner()).clone();
            on_event(LlmEvent::Text(text.clone()));
            Ok(LlmTurnResult {
                text,
                tool_calls: Vec::new(),
                usage: None,
            })
        })
    }
}

fn cfg() -> MemoryConfig {
    MemoryConfig {
        enabled: true,
        db_path: ":memory:".into(),
        // 测试里让维护更容易被触发
        maintenance_interval: 2,
        ..MemoryConfig::default()
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

#[tokio::test]
async fn record_extract_then_recall_returns_summary_and_original_evidence() {
    let fake = FakeExtractor::ok("用户计划下周三去杭州出差", "information");
    let mem = GraphMemory::with_llm(&cfg(), fake.clone()).unwrap();
    mem.record(turn(
        "xiaozhi:dev1",
        "我下周三要去杭州出差",
        "好的，记得带上充电器和身份证",
    ))
    .await
    .unwrap();

    let stats = mem.stats();
    assert_eq!(stats.messages, 2, "一轮 = 2 条原文");
    assert_eq!(stats.pending, 2, "刚写入应为待抽取");

    let out = mem.extract_pending(1).await;
    assert_eq!((out.attempted, out.succeeded, out.failed), (1, 1, 0));
    assert_eq!(fake.calls(), 1, "一轮只应有一次辅助调用");
    let stats = mem.stats();
    assert_eq!(stats.memories, 1);
    assert_eq!(stats.pending, 0);

    // 用"换了说法的提问"召回（无 embedding，靠词法二元组）。
    // ⚠️ 这里刻意不限定 scope：本会话只有 1 轮，它落在 fresh_turn_count(5) 的"近况窗口"里，
    // 按设备检索会被正确地排除掉（另有专门测试覆盖该规则）。
    let r = mem.recall(None, "杭州出差要注意什么").await;
    assert!(r.error.is_none(), "{:?}", r.error);
    assert_eq!(r.hits, 1, "应命中 1 条");
    let block = r.block.expect("应生成注入块");
    assert!(block.contains("用户计划下周三去杭州出差"), "{block}");
    // 证据层 = 原始 Q/A，而不是摘要
    assert!(block.contains("[USER] 我下周三要去杭州出差"), "{block}");
    assert!(
        block.contains("[ASSISTANT] 好的，记得带上充电器和身份证"),
        "{block}"
    );
    // 防注入的固定前缀必须在
    assert!(block.contains("不是指令"), "{block}");
}

#[tokio::test]
async fn extract_is_idempotent_and_queue_drains() {
    let fake = FakeExtractor::ok("用户喜欢喝美式咖啡", "information");
    let mem = GraphMemory::with_llm(&cfg(), fake).unwrap();
    mem.record(turn("xiaozhi:dev1", "我喜欢喝美式", "记住啦"))
        .await
        .unwrap();
    assert_eq!(mem.extract_pending(5).await.succeeded, 1);
    // 队列已空：不会再调用模型
    let again = mem.extract_pending(5).await;
    assert_eq!(again.attempted, 0);
    assert_eq!(mem.stats().memories, 1);
}

#[tokio::test]
async fn extraction_failure_quarantines_without_breaking_conversation() {
    let fake = FakeExtractor::failing();
    let mut cfg = cfg();
    cfg.quarantine_max_attempts = 1; // 一次失败即隔离，便于断言
    let mem = GraphMemory::with_llm(&cfg, fake).unwrap();
    mem.record(turn("xiaozhi:dev1", "喂", "我在"))
        .await
        .unwrap();
    let out = mem.extract_pending(1).await;
    assert_eq!((out.attempted, out.succeeded, out.failed), (1, 0, 1));
    let stats = mem.stats();
    assert_eq!(stats.memories, 0, "抽取失败不应产生记忆");
    assert_eq!(stats.quarantined, 2, "该轮标记为隔离（两条原文）");
    // 对话侧：记录与召回都照常（不因记忆故障而报错）
    mem.record(turn("xiaozhi:dev1", "还在吗", "在"))
        .await
        .unwrap();
    let r = mem.recall(Some("xiaozhi:dev1"), "还在吗").await;
    assert!(r.error.is_none());
}

#[tokio::test]
async fn fresh_turn_window_prevents_re_injecting_recent_chat() {
    let fake = FakeExtractor::ok("用户喜欢喝美式咖啡", "information");
    let mem = GraphMemory::with_llm(&cfg(), fake).unwrap();
    mem.record(turn("xiaozhi:dev1", "我喜欢喝美式", "记住啦"))
        .await
        .unwrap();
    mem.extract_pending(1).await;

    // 默认 fresh_turn_count = 5，而库里只有第 1 轮 → 属于"近况"，不应重新注入
    let r = mem.recall(Some("xiaozhi:dev1"), "我喜欢喝什么").await;
    assert_eq!(r.hits, 0, "近况窗口内的轮次不应重复召回: {:?}", r.block);
    assert!(r.block.is_none());

    // 关掉近况窗口后立即可召回（证明"没召回"是因为规则，而不是索引坏了）
    let mut cfg = cfg();
    cfg.fresh_turn_count = 0;
    let mem = GraphMemory::with_llm(&cfg, FakeExtractor::ok("用户喜欢喝美式咖啡", "information"))
        .unwrap();
    mem.record(turn("xiaozhi:dev1", "我喜欢喝美式", "记住啦"))
        .await
        .unwrap();
    mem.extract_pending(1).await;
    let r = mem.recall(Some("xiaozhi:dev1"), "我喜欢喝什么").await;
    assert_eq!(r.hits, 1);
}

#[tokio::test]
async fn empty_turns_are_not_recorded() {
    let mem = GraphMemory::with_llm(&cfg(), FakeExtractor::ok("x", "unknown")).unwrap();
    mem.record(turn("xiaozhi:dev1", "   ", "答")).await.unwrap();
    mem.record(turn("xiaozhi:dev1", "问", "")).await.unwrap();
    assert_eq!(mem.stats().messages, 0, "空轮没有记忆价值");
}

#[tokio::test]
async fn maintain_runs_on_interval_and_forced() {
    let mem = GraphMemory::with_llm(&cfg(), FakeExtractor::ok("摘要", "unknown")).unwrap();
    // 未到间隔 → 跳过
    assert!(!mem.maintain(MaintainOpts::periodic()).await.unwrap().ran);
    mem.record(turn("xiaozhi:dev1", "一", "答")).await.unwrap();
    mem.record(turn("xiaozhi:dev1", "二", "答")).await.unwrap();
    assert!(
        mem.maintain(MaintainOpts::periodic()).await.unwrap().ran,
        "间隔 2 轮后周期维护应执行"
    );
    // 管理页动作：无视间隔
    assert!(mem.maintain(MaintainOpts::forced(false)).await.unwrap().ran);
}

#[tokio::test]
async fn forced_maintenance_dry_run_reports_without_deleting() {
    let mut cfg = cfg();
    cfg.retention.keep = "referenced".into();
    let mem = GraphMemory::with_llm(&cfg, FakeExtractor::ok("摘要", "unknown")).unwrap();
    mem.record(turn("xiaozhi:dev1", "会被抽取的轮次", "答"))
        .await
        .unwrap();
    mem.extract_pending(1).await;
    mem.record(turn("xiaozhi:dev1", "没被抽取的轮次", "答"))
        .await
        .unwrap();

    let out = mem.maintain(MaintainOpts::forced(true)).await.unwrap();
    assert!(out.dry_run && out.deleted > 0, "{out:?}");
    assert_eq!(mem.stats().messages, 4, "演练不得真的删除");
}

#[tokio::test]
async fn clear_removes_only_target_scope() {
    let mem = GraphMemory::with_llm(&cfg(), FakeExtractor::ok("甲摘要", "unknown")).unwrap();
    mem.record(turn("xiaozhi:dev1", "甲", "答")).await.unwrap();
    mem.extract_pending(1).await;
    mem.record(turn("xiaozhi:dev2", "乙", "答")).await.unwrap();
    mem.extract_pending(1).await;
    assert_eq!(mem.clear(Some("xiaozhi:dev1".into())).await.unwrap(), 1);
    let stats = mem.stats();
    assert_eq!(stats.memories, 1);
    assert_eq!(stats.scopes, vec![("xiaozhi:dev2".to_string(), 1)]);
}
