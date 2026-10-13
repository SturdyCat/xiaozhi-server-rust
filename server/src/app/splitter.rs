//! LLM 流式文本按句切分器。
//!
//! [`SentenceSplitter`] 把 LLM 流式下发的文本增量按句切分，使 TTS 能「首句即播」
//! （见 [`crate::app::session`] 流水线）。命中句末标点即出句（标点保留在句尾）；流结束
//! `flush` 残句；无标点超长按 [`MAX_SENTENCE_CHARS`] 兜底硬切（对齐多字节字符边界）。
//!
//! ## 定时 flush（缺口③）
//!
//! `last_feed` 记录上次喂入时刻；[`SentenceSplitter::take_expired`] 在"距上一 token
//! 静默 ≥ `[llm].sentence_flush_ms` 且残句非空"时倒出残句——由独立的
//! [`spawn_flush_watchdog`] 驱动（回调只在 token 到达时执行，"停顿"期间没有代码在跑，
//! 物理上无法在回调内自判定，见 `session/producer.rs` 模块注释的方案取舍）。
//!
//! 已知取舍：英文缩写 `Mr.`/`Dr.` 会被误切——第一版接受（对齐方案文档 §3.2）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 无标点兜底句长（字符数）：LLM 长输出无标点时按此长度强制切句，保证首句延迟有上界。
pub const MAX_SENTENCE_CHARS: usize = 50;

/// LLM 流式文本按句切分器。
///
/// 命中句末标点（`。！？；` 及 ASCII `! ? ; \n`）即出句（标点保留在句尾）；
/// 流结束 `flush` 残句；无标点超长按 [`MAX_SENTENCE_CHARS`] 兜底硬切。
/// 已知取舍：英文缩写 `Mr.`/`Dr.` 会被误切——第一版接受（对齐方案文档 §3.2）。
pub struct SentenceSplitter {
    buf: String,
    /// 上次 feed 时刻（定时 flush 判定；`None` = 还没喂过）。
    last_feed: Option<Instant>,
}

impl SentenceSplitter {
    pub fn new() -> Self {
        Self { buf: String::new(), last_feed: None }
    }

    /// 喂入文本增量，返回本次切出的完整句（可能 0~N 句）。
    /// 每次喂入刷新 [`Self::last_feed`]（定时 flush 的静默计时起点）。
    pub fn feed(&mut self, delta: &str) -> Vec<String> {
        self.buf.push_str(delta);
        self.last_feed = Some(Instant::now());
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
        self.last_feed = None; // 残句已倒空：定时判定随之时效
        let mut out = Vec::new();
        push_sentence(&mut out, &rest);
        out
    }

    /// 定时 flush 判定（缺口③，纯函数可单测）：静默 ≥ `max_wait` 且残句非空 →
    /// 取走残句（trim 后非空才返回 `Some`），并刷新 `last_feed`（残句清空）。
    /// 未到期 / 残句为空 → `None`（watchdog 重算 deadline 继续睡）。
    pub fn take_expired(&mut self, now: Instant, max_wait: Duration) -> Option<String> {
        let last = self.last_feed?;
        if now.duration_since(last) < max_wait || self.buf.trim().is_empty() {
            return None;
        }
        let rest = std::mem::take(&mut self.buf);
        self.last_feed = Some(now); // 残句已取走：下一轮静默计时从现在起算
        let s = rest.trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    /// 下一次到期时刻（缺口③）：`last_feed + max_wait`；残句空 / 没喂过 → `None`
    ///（watchdog 只等 notify / 兜底轮询）。
    pub fn flush_deadline(&self, max_wait: Duration) -> Option<Instant> {
        let last = self.last_feed?;
        if self.buf.trim().is_empty() {
            return None;
        }
        Some(last + max_wait)
    }
}

/// 残句看门狗（缺口③）：producer 侧 spawn，到期把残句推入句子通道。
///
/// `on_flush`：残句 **send 成功后**调用的通知钩子（`Arc<dyn Fn() + Send + Sync>`，
/// 贴合 `prefetch` 的 `first_pcm` hook 范式）——调用方（producer）用它 mark
/// `sentence_ready`（定时出的残句也是"首个句子送 TTS"的合法来源，否则
/// 缓存轮之外的思考型停顿会让该阶段恒为 None）。本模块不 import events 类型，
/// 保持通用分句器与观测解耦。
///
/// 退出三条件（**先查再取**，顺序保证 abort 后不追加残句）：
/// ① `done`（producer 收工，含 notify 唤醒）；② `abort` 置位；③ `tx` 关闭
///（消费者已放弃）。锁内无 await；Notify 与检查之间的微小竞态窗口最坏多睡一个
/// `max_wait` 周期（≤ 上限 60s 内、默认 1.5s）后自行退出，可接受。
#[allow(clippy::too_many_arguments)] // 退出信号三件套 + 钩子 + 时长，与 producer 侧接线一一对应
pub(crate) fn spawn_flush_watchdog(
    shared: Arc<Mutex<SentenceSplitter>>,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
    abort: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    finished: Arc<tokio::sync::Notify>,
    on_flush: Arc<dyn Fn() + Send + Sync>,
    max_wait: Duration,
) {
    tokio::spawn(async move {
        // 兜底轮询周期：即使 notify 丢失（竞态窗口），最坏延迟一个周期也能感知收工。
        let fallback = max_wait.min(Duration::from_millis(500));
        loop {
            if done.load(Ordering::Acquire) || abort.load(Ordering::Relaxed) || tx.is_closed() {
                return;
            }
            // 下一次到期时刻（残句空 / 没喂过 → None：只等 notify 或兜底轮询）
            let deadline = shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .flush_deadline(max_wait);
            let sleep = match deadline {
                Some(d) => d.saturating_duration_since(Instant::now()).min(fallback),
                None => fallback,
            };
            tokio::select! {
                _ = tokio::time::sleep(sleep) => {}
                _ = finished.notified() => {
                    // producer 收工：先复查 done 再决定是否取残句（回调侧 flush 已倒过）
                    if done.load(Ordering::Acquire) {
                        return;
                    }
                }
            }
            // 醒来复查三个退出条件（sleep 竞争失败也可能错过一次 notify）
            if done.load(Ordering::Acquire) || abort.load(Ordering::Relaxed) || tx.is_closed() {
                return;
            }
            let now = Instant::now();
            let expired = shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take_expired(now, max_wait);
            if let Some(sentence) = expired {
                tracing::debug!("残句定时出句（静默 ≥ {}ms）：{:?}", max_wait.as_millis(), sentence);
                if tx.send(sentence).is_err() {
                    return; // 消费者已退出
                }
                // send 成功才通知（失败 = 残句没送出去，不该被记成"句子已就绪"）
                on_flush();
            }
            // 误醒无害：新 token 已刷新 last_feed，下一轮 deadline 重算继续睡
        }
    });
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
