//! `memory/rag.rs`（P1-9 本地文档 RAG 后端）的端到端测试：**真 SQLite + 真临时目录**，
//! 零 LLM（rag 构造不触碰 LLM——这本身就是被测的结构保证）。
//!
//! 对齐 PRD P1-9 验收：1 命中含来源、2 零知识零影响、4 重建幂等、5 超预算静默、
//! 7 graph ⇄ rag 切换数据互不删。

use std::sync::Arc;

use super::*;
use crate::plugins::llm::{LlmEvent, LlmProvider, LlmTurnResult, ToolSpec, TurnPrompt};
use crate::plugins::memory::{CompletedTurn, GraphMemory, MemoryProvider};

/// graph 阶段用的假 LLM（rag 零 LLM，仅切换测试需要）。
struct FakeOkLlm;

impl LlmProvider for FakeOkLlm {
    fn chat_stream<'a>(
        &'a self,
        _history: &'a [(String, String)],
        _user_text: &'a str,
        _prompt: &'a TurnPrompt,
        _tools: Option<&'a [ToolSpec]>,
        mut on_event: Box<dyn FnMut(LlmEvent) -> bool + Send + 'a>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<LlmTurnResult>> + Send + 'a>>
    {
        Box::pin(async move {
            let text = "```json\n{\"summary\":\"测试摘要\",\"outcome\":\"information\"}\n```".to_string();
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

fn rag_cfg(db: &str, docs_dir: &str) -> MemoryConfig {
    MemoryConfig {
        enabled: true,
        db_path: db.into(),
        rag: super::super::RagConfig {
            docs_dir: docs_dir.into(),
            ..super::super::RagConfig::default()
        },
        ..MemoryConfig::default()
    }
}

fn temp_dir(tag: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!(
        "xiaozhi-rag-mem-{}-{}-{}",
        std::process::id(),
        tag,
        seq
    ));
    std::fs::create_dir_all(&p).unwrap();
    p.to_string_lossy().into_owned()
}

#[tokio::test]
async fn ingest_rebuild_then_recall_returns_chunk_with_source() {
    let docs = temp_dir("hit");
    std::fs::write(
        std::path::Path::new(&docs).join("售后.md"),
        "# 售后服务\n\n售后服务电话是 400-123-4567，工作日九点到十八点。\n",
    )
    .unwrap();

    let mem = RagMemory::new(&rag_cfg(":memory:", &docs)).unwrap();
    // 构造时索引为空 + 目录存在 → 后台首次构建；等待其完成
    for _ in 0..100 {
        if !mem.stats().rag.as_ref().unwrap().building {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let rag = mem.stats().rag.unwrap();
    assert!(!rag.building);
    assert_eq!(rag.docs, 1, "首次自建应扫到 1 份文档");
    assert!(rag.chunks >= 1);
    assert!(rag.last_build_at_ms.is_some());

    // 验收 1：换了说法的问题仍能召回，注入正文含号码与来源文件名
    let r = mem.recall(Some("xiaozhi:dev1"), "售后服务电话是多少").await;
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(r.hits >= 1);
    let block = r.block.expect("应注入正文");
    assert!(block.contains("400-123-4567"), "{block}");
    assert!(block.contains("售后"), "来源可解释: {block}");

    // 验收 5：预算压到 1 → 超预算/放不下 → 静默降级，不阻塞
    let mut cfg = rag_cfg(":memory:", &docs);
    cfg.recall_max_tokens = 1;
    let tight = RagMemory::new(&cfg).unwrap();
    let r = tight.recall(Some("xiaozhi:dev1"), "售后电话").await;
    assert!(r.block.is_none() || r.block.as_deref() == Some(""));

    let _ = std::fs::remove_dir_all(&docs);
}

#[tokio::test]
async fn zero_knowledge_is_zero_impact() {
    // docs_dir 指向不存在的目录：对话完全正常、recall 空、stats 0 文档（验收 2）
    let mem = RagMemory::new(&rag_cfg(":memory:", "/nonexistent/rag-docs-never")).unwrap();
    let stats = mem.stats();
    let rag = stats.rag.as_ref().unwrap();
    assert_eq!(rag.docs, 0);
    assert!(!rag.building, "目录不存在不触发构建");
    mem.record(turn("xiaozhi:dev1", "我下周三要去杭州出差", "记得带充电器"))
        .await
        .unwrap();
    let stats = mem.stats();
    assert_eq!(stats.stored, 2, "对话沉淀照常（零知识不影响对话）");
    let r = mem.recall(Some("xiaozhi:dev1"), "随便问点什么").await;
    assert!(r.block.is_none() && r.hits == 0);
}

#[tokio::test]
async fn record_stores_and_reindex_is_idempotent() {
    let docs = temp_dir("idem");
    std::fs::write(
        std::path::Path::new(&docs).join("a.md"),
        "第一份文档 一些内容要足够长 产生词项",
    )
    .unwrap();
    let mem = RagMemory::new(&rag_cfg(":memory:", &docs)).unwrap();
    wait_idle(&mem).await;
    let c1 = mem.stats().rag.unwrap().chunks;

    // 对话照常落 stored（验收：知识问答与对话沉淀不互相废弃）
    mem.record(turn("xiaozhi:dev1", "问一", "答一")).await.unwrap();
    mem.record(turn("xiaozhi:dev1", "问二", "答二")).await.unwrap();
    let stats = mem.stats();
    assert_eq!(stats.stored, 4);
    assert_eq!(stats.turns, 2);
    assert_eq!(stats.rag.as_ref().unwrap().chunks, c1, "record 不动文档索引");

    // 验收 4：手动触发重建，分块数一致；重复触发防重入
    assert!(mem.reindex(), "空闲时应触发成功");
    assert!(!mem.reindex(), "构建中再次触发应被防重入挡下");
    wait_idle(&mem).await;
    let after = mem.stats().rag.unwrap();
    assert_eq!(after.chunks, c1);
    let _ = std::fs::remove_dir_all(&docs);
}

/// 维护：保留策略对对话原文生效、文档索引不删（store 同款口径）。
#[tokio::test]
async fn maintain_applies_retention_to_turns_not_docs() {
    let docs = temp_dir("maint");
    std::fs::write(std::path::Path::new(&docs).join("k.md"), "知识内容 需要足够长 产生词项").unwrap();
    let mut cfg = rag_cfg(":memory:", &docs);
    cfg.retention.keep = "recent".into();
    cfg.retention.recent_turns = 1;
    let mem = RagMemory::new(&cfg).unwrap();
    wait_idle(&mem).await;
    let docs_before = mem.stats().rag.unwrap().docs;
    assert_eq!(docs_before, 1);

    mem.record(turn("xiaozhi:dev1", "第一轮", "答")).await.unwrap();
    mem.record(turn("xiaozhi:dev1", "第二轮", "答")).await.unwrap();
    let out = mem.maintain(MaintainOpts::forced(false)).await.unwrap();
    assert!(out.ran);
    let stats = mem.stats();
    assert_eq!(stats.turns, 1, "只留最近 1 轮");
    assert_eq!(stats.rag.unwrap().docs, 1, "文档索引不受维护影响");
    let _ = std::fs::remove_dir_all(&docs);
}

/// graph ⇄ rag 切换（同一 db 文件）：图记忆数据与 RAG 索引各自保留（验收 7）。
#[tokio::test]
async fn switch_graph_rag_graph_preserves_both_sides() {
    let docs = temp_dir("switch");
    std::fs::write(
        std::path::Path::new(&docs).join("k.md"),
        "售后服务电话是 400-123-4567 内容足够长",
    )
    .unwrap();
    let db = format!("{}/memory.db", temp_dir("switchdb"));

    // 阶段 1：graph 抽取 1 条记忆
    let gcfg = MemoryConfig {
        enabled: true,
        db_path: db.clone(),
        ..MemoryConfig::default()
    };
    let graph = GraphMemory::with_llm(&gcfg, Arc::new(FakeOkLlm)).unwrap();
    graph
        .record(turn("xiaozhi:dev1", "我喜欢喝美式咖啡", "好的，记住了"))
        .await
        .unwrap();
    let out = graph.extract_pending(1).await;
    assert_eq!((out.attempted, out.succeeded), (1, 1));
    drop(graph);

    // 阶段 2：rag 建索引 + 继续落对话
    let mem = RagMemory::new(&rag_cfg(&db, &docs)).unwrap();
    wait_idle(&mem).await;
    mem.record(turn("xiaozhi:dev1", "问", "答")).await.unwrap();
    let stats = mem.stats();
    assert_eq!(stats.memories, 1, "graph 阶段的记忆保留");
    assert_eq!(stats.stored, 2, "rag 阶段落的原文");
    assert!(stats.rag.unwrap().docs >= 1);
    let r = mem.recall(Some("xiaozhi:dev1"), "售后电话").await;
    assert!(r.hits >= 1, "rag 检索只查文档表仍命中");
    drop(mem);

    // 阶段 3：切回 graph——记忆与原文都在，stored 行不进抽取队列
    let graph2 = GraphMemory::with_llm(&gcfg, Arc::new(FakeOkLlm)).unwrap();
    let stats = graph2.stats();
    assert_eq!(stats.memories, 1, "图记忆不丢");
    assert_eq!(stats.pending, 0, "stored 行对抽取队列不可见");
    assert_eq!(stats.turns, 2);
    let _ = std::fs::remove_dir_all(&docs);
    let _ = std::fs::remove_file(&db);
}

/// 等 rag 的后台构建结束（测试基建：轮询 stats.rag.building）。
async fn wait_idle(mem: &RagMemory) {
    for _ in 0..200 {
        if !mem.index_snapshot().building {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("rag 后台构建超时未结束");
}
