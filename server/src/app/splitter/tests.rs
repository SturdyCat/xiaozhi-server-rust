//! `server/src/app/splitter.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::{find_sentence_end, SentenceSplitter};

#[test]
fn splitter_splits_on_chinese_punctuation() {
    let mut sp = SentenceSplitter::new();
    assert!(sp.feed("今天天气不错，").is_empty(), "逗号不是句末标点");
    let out = sp.feed("适合出门。明天呢？");
    assert_eq!(out, vec!["今天天气不错，适合出门。", "明天呢？"]);
    assert!(sp.flush().is_empty());
}

#[test]
fn splitter_multiple_sentences_single_delta() {
    let mut sp = SentenceSplitter::new();
    let out = sp.feed("一。二！三？四；");
    assert_eq!(out, vec!["一。", "二！", "三？", "四；"]);
}

#[test]
fn splitter_flush_tail_without_punctuation() {
    let mut sp = SentenceSplitter::new();
    assert_eq!(sp.feed("你好。再见"), vec!["你好。"]);
    let out = sp.flush();
    assert_eq!(out, vec!["再见"]);
}

#[test]
fn splitter_fallback_max_len_no_punctuation() {
    let mut sp = SentenceSplitter::new();
    let text: String = "啊".repeat(120);
    let out = sp.feed(&text);
    // 120 字无标点 → 按 50 字兜底切出 2 句，剩 20 字留在缓冲
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|s| s.chars().count() == 50));
    assert_eq!(sp.flush(), vec!["啊".repeat(20)]);
}

#[test]
fn splitter_ascii_and_newline_boundaries() {
    let mut sp = SentenceSplitter::new();
    // \n 段去掉首尾空白后为空 → 跳过；"new line" 留在缓冲待 flush
    let out = sp.feed("ok! next? done;\nnew line");
    assert_eq!(out, vec!["ok!", "next?", "done;"]);
    assert_eq!(sp.flush(), vec!["new line"]);
}

#[test]
fn splitter_fallback_aligned_to_char_boundary() {
    // 兜底硬切必须对齐多字节字符边界（中文 3 字节），不得切碎
    let mut sp = SentenceSplitter::new();
    let text: String = "语".repeat(60);
    let out = sp.feed(&text);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].chars().count(), 50);
    assert_eq!(sp.flush(), vec!["语".repeat(10)]);
}

#[test]
fn find_sentence_end_positions() {
    assert_eq!(find_sentence_end("你好。"), Some(9)); // 3 字节 ×2 + 3 字节句号
    assert_eq!(find_sentence_end("你好"), None);
    assert_eq!(find_sentence_end(""), None);
    assert_eq!(find_sentence_end("hi!\n"), Some(3));
}
