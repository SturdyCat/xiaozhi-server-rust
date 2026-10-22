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
//!
//! ## 取消后不倒残句
//!
//! 回调返 `false`（打断）后，流结束时**不再**把 splitter 里的残句推进通道——与
//! watchdog 的"abort 后不再出句"同一条纪律。否则被打断的这一轮会继续占用 TTS 合成，
//! 并把 `sentence_ready` 记成一个并不存在的时刻（分段延迟分位数失真）。
//! 判据用**回调拒绝**的记录位而非 `abort` 本身：`abort` 会被下一轮复位，
//! 用它判断会把上一轮的残句误当成新一轮的。
//!
//! ## 首 token / 首句的日志判据
//!
//! 三条出句路径（标点出句、定时 flush 残句、流结束残句）都用 `mark_sentence_ready()`
//! 的**返回值**决定是否打日志，而不是另设一个原子标志——判据与 first-only 语义同源，
//! 不会出现"日志说首句到了、快照里该阶段却是 None"的自相矛盾观测。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::observability::events::TurnTrace;
use crate::plugins::llm::{Llm, LlmEvent, LlmTurnResult};
use crate::plugins::prompt::TurnPrompt;
use crate::workflow::pipeline::splitter::{spawn_flush_watchdog, SentenceSplitter};

/// producer 的对外产物：句子接收端 + LLM 汇总结果的等待句柄。
pub(super) struct LlmProducer {
    /// 已切分的句子流（unbounded：文本量级 KB，无需反压）。
    pub sentences: tokio::sync::mpsc::UnboundedReceiver<String>,
    /// LLM 流结束后的汇总（文本/工具调用/令牌用量），调用方 `.await` 取。
    pub result: tokio::task::JoinHandle<anyhow::Result<LlmTurnResult>>,
}

/// producer 服务的**那一轮**的上下文：`trace`（mark 用）+ `session_id`（仅日志用）。
///
/// 为什么要打包成一个参数而不是给 `spawn_llm_producer` 加第 8 个位置参数：
/// 两者本就同属"轮次上下文"，而函数签名越过 clippy 的参数上限（7）后每次改动都要多背一条
/// lint。`session_id` 只是为了让轮次内的日志行带上会话上下文——`turn=` 本身进程内唯一，
/// 但运维排查常常是**先按会话过滤**再看轮次，缺了它就得反向查。
#[derive(Clone)]
pub(super) struct ProducerTurn {
    /// 本轮的 trace（mark 与 `turn_id` 的唯一来源）。
    pub trace: Arc<TurnTrace>,
    /// 会话 id：只进日志文案，不参与任何判定。
    pub session_id: String,
    /// 本轮可用的技能工具（`execute_skill`，来自技能目录）；`None` = 纯聊天不声明工具。
    pub tools: Option<Vec<crate::plugins::llm::ToolSpec>>,
}

/// 读 LLM 流 → 按句切分（含定时 flush watchdog）→ unbounded channel。
///
/// 回调内：abort 轮询（既有语义）+ 首 token / 首句 trace 埋点（缺口②）。
/// `flush_wait = 0` 时不 spawn watchdog（回退缺口③之前的现行为）。
///
/// 轮次内的首 token / 首句就绪日志都带 `turn=`（取自 `turn.trace`）与 session 前缀，
/// 因此 `grep turn=N` 能一次捞出整轮的日志行。
pub(super) fn spawn_llm_producer(
    llm: Llm,
    history: Vec<(String, String)>,
    user: String,
    prompt: TurnPrompt,
    abort: Arc<AtomicBool>,
    turn_ctx: ProducerTurn,
    flush_wait: Duration,
) -> LlmProducer {
    let ProducerTurn {
        trace,
        session_id,
        tools,
    } = turn_ctx;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    // 共享 splitter：回调 feed（切句）与 watchdog take_expired（定时出残句）共用。
    let shared = Arc::new(Mutex::new(SentenceSplitter::new()));
    // producer 收工信号：回调返回 false / 流结束后置位，watchdog 据此退出。
    let done = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(tokio::sync::Notify::new());
    let turn = trace.turn_id();
    // 定时 flush（缺口③）：wait > 0 才启用。
    // on_flush：残句送出后 mark sentence_ready——定时出的残句也是"首个句子送 TTS"
    // 的合法来源（思考型停顿轮若不 mark，该阶段恒为 None，分位数失真）。
    if !flush_wait.is_zero() {
        let trace_watch = trace.clone();
        let sid = session_id.clone();
        let on_flush: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            // 只在**真正首次** mark 时记日志：判据取自 mark 的返回值，与 first-only
            // 语义同源（不会打出"第 7 次首句就绪"这种自相矛盾的观测）。
            if trace_watch.mark_sentence_ready() {
                tracing::info!(turn, "session {sid} 首句就绪（定时 flush 残句）");
            }
        });
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
        let sid = session_id.clone();
        // 本轮是否被调用方取消（回调返过false）：与 `abort` 分开记录，因为**回调拒绝**才是
        // 引擎视角的取消信号（契约见 `plugins::llm::LlmTurnResult`），而 `abort` 可能已被
        // 下一轮复位——用它判断会把上一轮的残句误当成新一轮的。
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_cb = cancelled.clone();
        let result = llm
            .chat_stream(
                &history,
                &user,
                &prompt,
                tools.as_deref(),
                Box::new(move |event| {
                    if abort.load(Ordering::Relaxed) {
                        cancelled_cb.store(true, Ordering::Release);
                        return false; // 设备打断：停止读流
                    }
                    if let LlmEvent::Text(delta) = event {
                        if trace_cb.mark_llm_first_token() {
                            tracing::info!(turn, "session {sid} LLM 首 token 到达");
                        }
                        let mut sp = shared_cb.lock().unwrap_or_else(|e| e.into_inner());
                        let sentences = sp.feed(&delta);
                        if !sentences.is_empty() {
                            if trace_cb.mark_sentence_ready() {
                                tracing::info!(turn, "session {sid} 首句就绪（标点/硬切出句）");
                            }
                            for s in sentences {
                                let _ = tx_cb.send(s);
                            }
                        }
                    }
                    true
                }),
            )
            .await;
        // 流结束：倒出残句（含 watchdog 可能刚取走后的空残句，无副作用）。
        // ⚠️ **取消轮不倒**（与 watchdog 的"abort 后不再出句"同一条纪律）：
        // 打断发生后再把半句推进通道，等于让已经被用户否决的这一轮继续占用合成资源，
        // 并把 `sentence_ready` 记成一个并不存在的时刻（污染分段延迟分位数）。
        if !cancelled.load(Ordering::Acquire) {
            let mut sp = shared.lock().unwrap_or_else(|e| e.into_inner());
            for s in sp.flush() {
                if trace.mark_sentence_ready() {
                    tracing::info!(turn, "session {session_id} 首句就绪（流结束残句）");
                }
                let _ = tx.send(s);
            }
        }
        // 收工：置位 + 唤醒 watchdog（它会立即退出，不再出残句）
        done.store(true, Ordering::Release);
        finished.notify_waiters();
        result
    });
    LlmProducer {
        sentences: rx,
        result,
    }
}

#[cfg(test)]
mod tests;
