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
mod tests;
