//! `store/rag_index.rs` 的单元测试：真 SQLite（`:memory:`），无网络。
//!
//! 重点锚住 PRD P1-9 的两条容易静默出错的规则：
//! ① 全量重建幂等（同一批文档重建两次分块数一致、无重复，验收 4）；
//! ② `docs_dir` 不存在 = 清空索引返回零文档（零知识零影响，验收 2）。

use super::*;
use crate::plugins::memory::{schema, MemoryConfig};

fn cfg() -> MemoryConfig {
    MemoryConfig {
        enabled: true,
        db_path: ":memory:".into(),
        ..MemoryConfig::default()
    }
}

fn write_doc(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, content).unwrap();
    p
}

#[test]
fn chunk_text_splits_paragraphs_and_windows_long_ones() {
    // 短段直接成块
    let chunks = chunk_text("第一段内容。\n\n第二段内容。", 512, 64);
    assert_eq!(chunks.len(), 2);

    // 超长段按滑窗切，且相邻块有重叠（跨块边界语义保住）
    let long = "甲".repeat(300) + "。" + &"乙".repeat(300);
    let chunks = chunk_text(&long, 100, 20);
    assert!(chunks.len() >= 6, "300+301 字符按 100/步长 80 至少 6 块: {}", chunks.len());
    // 重叠：第 2 块的开头应与第 1 块的结尾部分重叠
    assert!(chunks[1].chars().count() <= 100);

    // chunk_size=0 时步长兜底为 1（不 panic）
    let chunks = chunk_text("abc", 1, 5);
    assert_eq!(chunks.len(), 3, "overlap 被钳到 size-1=0，步长 1");
}

#[test]
fn rebuild_is_idempotent_and_search_hits_content() {
    let store = crate::plugins::memory::store::open(&cfg()).unwrap();
    let dir = std::env::temp_dir().join(format!("xiaozhi-rag-t1-{}-{}", std::process::id(), schema::now_ms()));
    std::fs::create_dir_all(&dir).unwrap();
    write_doc(&dir, "售后.md", "# 售后服务\n\n售后服务电话是 400-123-4567，工作日 9 点到 18 点。\n\n保修期为两年。");

    let s1 = store.rebuild_docs(dir.to_str().unwrap(), 512, 64).unwrap();
    assert_eq!(s1.docs, 1);
    assert!(s1.chunks >= 1);

    // 幂等：连续两次全量重建，分块总数一致（验收 4）
    let s2 = store.rebuild_docs(dir.to_str().unwrap(), 512, 64).unwrap();
    assert_eq!(s1.chunks, s2.chunks);
    let counts = store.rag_counts().unwrap();
    assert_eq!(counts.docs, 1);
    assert_eq!(counts.chunks as u32, s2.chunks);

    // 检索命中（换了说法仍能召回，同 graph 词法口径）
    let terms = terms::query_terms("售后电话是多少");
    assert!(!terms.is_empty());
    let hits = store.search_chunks(&terms, 4).unwrap();
    assert!(!hits.is_empty());
    assert!(hits[0].text.contains("400-123-4567"), "{:?}", hits[0].text);
    assert!(hits[0].source.ends_with("售后.md"), "来源可解释: {}", hits[0].source);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_docs_dir_clears_index_and_zero_docs_is_normal() {
    let store = crate::plugins::memory::store::open(&cfg()).unwrap();
    store.ensure_rag_tables().unwrap();
    // 先写入一份旧索引内容
    let dir = std::env::temp_dir().join(format!("xiaozhi-rag-t2-{}-{}", std::process::id(), schema::now_ms()));
    std::fs::create_dir_all(&dir).unwrap();
    write_doc(&dir, "a.md", "一些内容需要足够长才能产生词项 一些内容");
    store.rebuild_docs(dir.to_str().unwrap(), 512, 64).unwrap();
    assert_eq!(store.rag_counts().unwrap().docs, 1);

    // 目录被删后重建 → 清空索引、零文档、不报错（验收 2）
    std::fs::remove_dir_all(&dir).unwrap();
    let s = store.rebuild_docs(dir.to_str().unwrap(), 512, 64).unwrap();
    assert_eq!(s.docs, 0);
    assert_eq!(s.chunks, 0);
    assert_eq!(store.rag_counts().unwrap(), RagIndexCounts::default());
    // 空索引检索 → 空（不报错）
    let terms = terms::query_terms("一些内容");
    assert!(store.search_chunks(&terms, 4).unwrap().is_empty());
}

#[test]
fn rebuild_picks_up_new_and_deleted_docs() {
    let store = crate::plugins::memory::store::open(&cfg()).unwrap();
    let dir = std::env::temp_dir().join(format!("xiaozhi-rag-t3-{}-{}", std::process::id(), schema::now_ms()));
    std::fs::create_dir_all(&dir).unwrap();
    write_doc(&dir, "a.md", "菠萝披萨配凤梨 味道特别 值得一试");
    store.rebuild_docs(dir.to_str().unwrap(), 512, 64).unwrap();

    write_doc(&dir, "b.md", "第二份文档 另一些内容 也要够长");
    let _ = std::fs::remove_file(dir.join("a.md"));
    store.rebuild_docs(dir.to_str().unwrap(), 512, 64).unwrap();
    let counts = store.rag_counts().unwrap();
    assert_eq!(counts.docs, 1, "删掉的文档不应残留在索引里");

    let terms = terms::query_terms("菠萝披萨");
    assert!(
        store.search_chunks(&terms, 4).unwrap().is_empty(),
        "被删文档的内容不应再被召回"
    );
    // 反向：现存文档仍可召回
    let terms = terms::query_terms("另一些内容");
    assert!(!store.search_chunks(&terms, 4).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
