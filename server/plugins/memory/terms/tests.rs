//! `plugins/memory/terms.rs` 的回归测试（逻辑见 `terms.rs`；测试与逻辑分文件见 `AGENTS.md` §5.10）。

use super::*;

#[test]
fn chinese_becomes_bigrams() {
    let t = index_terms("我想吃火锅");
    assert_eq!(t, vec!["我想", "想吃", "吃火", "火锅"]);
}

#[test]
fn punctuation_and_whitespace_split_runs() {
    // 标点断开 → 不产生跨标点的伪二元组
    let t = index_terms("火锅。好吃");
    assert_eq!(t, vec!["火锅", "好吃"]);
    let t = index_terms("火锅 好吃");
    assert_eq!(t, vec!["火锅", "好吃"]);
}

#[test]
fn single_char_run_produces_nothing() {
    assert!(index_terms("我").is_empty());
    // 单字 run 相邻的 ASCII 单词仍要被抽出
    assert_eq!(index_terms("我 love rust"), vec!["love", "rust"]);
}

#[test]
fn ascii_words_lowercased_and_short_ones_dropped() {
    assert_eq!(index_terms("OK, Rust-lang"), vec!["ok", "rust", "lang"]);
    // 单个字母不成词
    assert!(index_terms("a b c").is_empty());
}

#[test]
fn repeated_bigrams_are_deduplicated() {
    assert_eq!(index_terms("哈哈哈"), vec!["哈哈"]);
}

#[test]
fn mixed_language_and_digits() {
    let t = index_terms("温度 26 度");
    assert!(t.contains(&"温度".to_string()), "{t:?}");
    assert!(t.contains(&"26".to_string()), "{t:?}");
    assert!(
        !t.contains(&"26度".to_string()),
        "数字与汉字之间应断开: {t:?}"
    );
}

#[test]
fn query_terms_are_capped_and_keep_leading_context() {
    // 80 个互不相同的汉字 → 79 个不同二元组，必然触发上限
    let text: String = (0..80u32)
        .map(|i| char::from_u32(0x4E00 + i).unwrap())
        .collect();
    let q = query_terms(&text);
    assert_eq!(q.len(), MAX_QUERY_TERMS, "必须截断到上限");
    // 靠前词项被保留（关键信息通常在前半句）
    let first: String = text.chars().take(2).collect();
    assert_eq!(q.first(), Some(&first), "{q:?}");
}

#[test]
fn recall_matches_pairwise_evidence_from_differently_phrased_question() {
    // 索引一段摘要，用一个"换了说法但共享词项"的问题去查：二元组必须有交集
    let summary = index_terms("用户喜欢吃火锅，尤其爱麻辣口味");
    let query = query_terms("上次说的那个火锅店在哪");
    let hit = summary.iter().any(|s| query.contains(s));
    assert!(hit, "summary={summary:?} query={query:?}");
}
