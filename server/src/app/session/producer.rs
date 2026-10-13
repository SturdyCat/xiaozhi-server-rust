//! LLM 生产者 + 分句泵（从 `reply.rs` 抽出，缺口③④：行数上限 + 定时 flush watchdog）。
//!
//! `Session` 定义在父模块，子模块可直接访问其私有字段（AGENTS.md §5.10）。
//!
//! ## 定时 flush（缺口③）的核心取舍
//!
//! producer 是 `tokio::spawn` 的 async 任务，但**句子产出发生在同步回调里**
//!（`llm.chat_stream` 的 `Box<dyn FnMut(LlmEvent) -> bool>`）——"停顿"期间根本没有
//! 代码在跑，回调内自判物理上无法触发。因此采用**独立 watchdog 任务**：
//! [`SentenceSplitter`] 提到 `Arc<Mutex<..>>` 共享（回调与 watchdog 各自瞬时持锁、
//! 锁内无 await，`std::sync::Mutex` 足够），watchdog `sleep_until` 到期检查残句：
//!
//! - 新 token 刷新 `last_feed` → 到期重算 deadline 继续睡（误醒无害）；
//! - `take_expired` 只在"静默 ≥ max_wait 且残句非空"时给残句 → 推给 TTS
//!  （TTFA 上界收紧：思考型停顿不再无限等标点或 50 字硬切）；
//! - 退出三条件：producer 收工（done + notify）/ abort 置位 / 句子通道关闭。
//!   Notify 与检查之间的微小竞态窗口最坏多睡一个 `max_wait` 周期后自行退出，可接受。
//! - **abort 后不再出句**：watchdog 醒来先查 done/abort 再 `take_expired`。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::app::events::TurnTrace;
use crate::app::splitter::{SentenceSplitter, spawn_flush_watchdog};
use crate::plugins::llm::{Llm, LlmEvent, LlmTurnResult};
use crate::plugins::prompt::TurnPrompt;

/// producer 的对外产物：句子接收端 + LLM 汇总结果的等待句柄。
pub(super) struct LlmProducer {
    /// 已切分的句子流（unbounded：文本量级 KB，无需反压）。
    pub sentences: tokio::sync::mpsc::UnboundedReceiver<String>,
    /// LLM 流结束后的汇总（文本/工具调用/令牌用量），调用方 `.await` 取。
    pub result: tokio::task::JoinHandle<anyhow::Result<LlmTurnResult>>,
}

/// 读 LLM 流 → 按句切分（含定时 flush watchdog）→ unbounded channel。
///
/// 回调内：abort 轮询（既有语义）+ 首 token / 首句 trace 埋点（缺口②）。
/// `flush_wait = 0` 时不 spawn watchdog（回退缺口③之前的现行为）。
pub(super) fn spawn_llm_producer(
    llm: Llm,
    history: Vec<(String, String)>,
    user: String,
    prompt: TurnPrompt,
    abort: Arc<AtomicBool>,
    trace: Arc<TurnTrace>,
    flush_wait: Duration,
) -> LlmProducer {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    // 共享 splitter：回调 feed（切句）与 watchdog take_expired（定时出残句）共用。
    let shared = Arc::new(Mutex::new(SentenceSplitter::new()));
    // producer 收工信号：回调返回 false / 流结束后置位，watchdog 据此退出。
    let done = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(tokio::sync::Notify::new());
    // 定时 flush（缺口③）：wait > 0 才启用。
    // on_flush：残句送出后 mark sentence_ready——定时出的残句也是"首个句子送 TTS"
    // 的合法来源（思考型停顿轮若不 mark，该阶段恒为 None，分位数失真）。
    if !flush_wait.is_zero() {
        let trace_watch = trace.clone();
        let on_flush: Arc<dyn Fn() + Send + Sync> =
            Arc::new(move || trace_watch.mark_sentence_ready());
        spawn_flush_watchdog(
            shared.clone(),
            tx.clone(),
            abort.clone(),
            done.clone(),
            finished.clone(),
            on_flush,
            flush_wait,
        );
    }
    let result = tokio::spawn(async move {
        // 首 token / 首句埋点（缺口②）：first-only 语义在 TurnTrace 内部。
        // `shared` / `tx` 克隆进回调（回调要 `'static`），本体留给流结束后的 flush——
        // 两条路径写同一把锁/同一个通道，时序上互斥（回调先于 flush 结束）。
        let shared_cb = shared.clone();
        let tx_cb = tx.clone();
        let trace_cb = trace.clone();
        let result = llm
            .chat_stream(
                &history,
                &user,
                &prompt,
                None,
                Box::new(move |event| {
                    if abort.load(Ordering::Relaxed) {
                        return false; // 设备打断：停止读流
                    }
                    if let LlmEvent::Text(delta) = event {
                        trace_cb.mark_llm_first_token();
                        let mut sp = shared_cb.lock().unwrap_or_else(|e| e.into_inner());
                        let sentences = sp.feed(&delta);
                        if !sentences.is_empty() {
                            trace_cb.mark_sentence_ready();
                            for s in sentences {
                                let _ = tx_cb.send(s);
                            }
                        }
                    }
                    true
                }),
            )
            .await;
        // 流结束：倒出残句（含 watchdog 可能刚取走后的空残句，无副作用）
        {
            let mut sp = shared.lock().unwrap_or_else(|e| e.into_inner());
            for s in sp.flush() {
                trace.mark_sentence_ready();
                let _ = tx.send(s);
            }
        }
        // 收工：置位 + 唤醒 watchdog（它会立即退出，不再出残句）
        done.store(true, Ordering::Release);
        finished.notify_waiters();
        result
    });
    LlmProducer { sentences: rx, result }
}

#[cfg(test)]
mod tests;
