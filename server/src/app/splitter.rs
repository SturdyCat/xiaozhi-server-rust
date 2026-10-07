//! LLM 流式文本按句切分器。
//!
//! [`SentenceSplitter`] 把 LLM 流式下发的文本增量按句切分，使 TTS 能「首句即播」
//! （见 [`crate::app::session`] 流水线）。命中句末标点即出句（标点保留在句尾）；流结束
//! `flush` 残句；无标点超长按 [`MAX_SENTENCE_CHARS`] 兜底硬切（对齐多字节字符边界）。
//!
//! 已知取舍：英文缩写 `Mr.`/`Dr.` 会被误切——第一版接受（对齐方案文档 §3.2）。

/// 无标点兜底句长（字符数）：LLM 长输出无标点时按此长度强制切句，保证首句延迟有上界。
pub const MAX_SENTENCE_CHARS: usize = 50;

/// LLM 流式文本按句切分器。
///
/// 命中句末标点（`。！？；` 及 ASCII `! ? ; \n`）即出句（标点保留在句尾）；
/// 流结束 `flush` 残句；无标点超长按 [`MAX_SENTENCE_CHARS`] 兜底硬切。
/// 已知取舍：英文缩写 `Mr.`/`Dr.` 会被误切——第一版接受（对齐方案文档 §3.2）。
pub struct SentenceSplitter {
    buf: String,
}

impl SentenceSplitter {
    pub fn new() -> Self {
        Self { buf: String::new() }
    }

    /// 喂入文本增量，返回本次切出的完整句（可能 0~N 句）。
    pub fn feed(&mut self, delta: &str) -> Vec<String> {
        self.buf.push_str(delta);
        let mut out = Vec::new();
        while let Some(pos) = find_sentence_end(&self.buf) {
            let s: String = self.buf.drain(..pos).collect();
            push_sentence(&mut out, &s);
        }
        // 兜底：无标点且已超长 → 按最大句长硬切（对齐字符边界，避免切碎多字节字符）。
        // 循环切：单次大 delta 也全部切块，只留不足句长的尾巴。
        while self.buf.chars().count() >= MAX_SENTENCE_CHARS {
            let cut = self
                .buf
                .char_indices()
                .nth(MAX_SENTENCE_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(self.buf.len());
            let s: String = self.buf.drain(..cut).collect();
            push_sentence(&mut out, &s);
        }
        out
    }

    /// 流结束：返回剩余未成句的残句（可能为空）。
    pub fn flush(&mut self) -> Vec<String> {
        let rest = std::mem::take(&mut self.buf);
        let mut out = Vec::new();
        push_sentence(&mut out, &rest);
        out
    }
}

/// 返回第一个句末标点（含该标点）之后的字节偏移；无则 `None`。
fn find_sentence_end(buf: &str) -> Option<usize> {
    for (idx, ch) in buf.char_indices() {
        if matches!(ch, '。' | '！' | '？' | '；' | '!' | '?' | ';' | '\n') {
            return Some(idx + ch.len_utf8());
        }
    }
    None
}

/// 句子去除首尾空白后入列；空句（如纯换行）跳过。
fn push_sentence(out: &mut Vec<String>, s: &str) {
    let s = s.trim();
    if !s.is_empty() {
        out.push(s.to_string());
    }
}

#[cfg(test)]
mod tests {
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
}
