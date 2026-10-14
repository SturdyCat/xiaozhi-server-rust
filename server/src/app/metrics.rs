//! 延迟/事件聚合（P1-1）：事件总线的**常驻订阅者** + `GET /api/metrics` 只读投影。
//!
//! ## 定位
//!
//! 这是 P0-5 事件总线的第一个订阅者：会话只管 `emit`，本模块独立消费并聚合——
//! "新增观测维度只改事件结构 + 订阅者，不动 session"的解耦目标由本模块兑现。
//!
//! - **全内存态、进程重启清零**（口径与 `app/registry` 一致）；前端页面必须注明。
//! - 整轮延迟样本 = 助手轮次的 `elapsed_ms`（事件里已带），环形窗口最近 [`TURN_WINDOW`] 轮。
//! - **分段延迟**（缺口②）：`TurnLatency` 事件的各阶段偏移各自入窗（同窗口大小），
//!   `/api/metrics` 的 `latency` 节只增不改（老字段原样保留）。
//!
//! ## 计数口径：轮次，不是事件、也不是分段
//!
//! 本订阅者**不按事件条数、也不按分段条数**计数，一律**按轮次**：
//!
//! - `turns` 与整轮耗时窗口只由 `MessageAdded{role == "assistant"}` 驱动（每轮恰好一条）；
//! - 一条 `TurnLatency` 带 5 个阶段，但它们**分属 5 个独立窗口**，每轮每窗口只 +1。
//!
//! 加 `turn_id` 字段不改变以上任何一条：`turn_id` 只用于把分段归因到具体某轮（诊断与日志
//! 串联），**不参与**任何计数或入窗。回归测试见 `tests.rs`。
//!
//! ## 订阅时序
//!
//! `main.rs` 在 `Engines::new` 之后、`axum::serve` 之前 spawn [`subscriber`]——进程最早
//! 的订阅者，避免 broadcast 环形缓冲丢早期事件（`LAGGED` 只告警不退出）。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::broadcast;

use crate::app::events::{ANCHOR_INJECT, ANCHOR_SPEECH_END, ServerEvent};

/// 整轮延迟样本窗口（p50/p95 的计算样本数；超出丢最旧）。
const TURN_WINDOW: usize = 200;

/// 各分段延迟样本的窗口：与整轮同尺寸（最近 [`TURN_WINDOW`] 轮，超出丢最旧）。
fn push_window(w: &mut VecDeque<u64>, v: u64) {
    w.push_back(v);
    while w.len() > TURN_WINDOW {
        w.pop_front();
    }
}

/// 分段延迟窗口（缺口②）：`TurnLatency` 事件的各阶段样本。
/// 各窗口独立（某阶段 `None` 的轮次不进该阶段的窗）。
#[derive(Default)]
struct StageWindows {
    asr_end_ms: VecDeque<u64>,
    llm_first_token_ms: VecDeque<u64>,
    sentence_ready_ms: VecDeque<u64>,
    tts_first_pcm_ms: VecDeque<u64>,
    first_audio_ms: VecDeque<u64>,
}

/// 一个阶段的分位投影：p50/p90/p95/p99/max/samples（与整轮的口径一致）。
fn stage_stats_json(w: &VecDeque<u64>) -> serde_json::Value {
    let mut samples: Vec<u64> = w.iter().copied().collect();
    samples.sort_unstable();
    // 分位取"向上取整位"（p95 of 100 样本 = 第 95 个）；空样本一律 0。
    let pick = |q: f64| -> u64 {
        if samples.is_empty() {
            0
        } else {
            let idx =
                (((samples.len() as f64) * q).ceil() as usize).clamp(1, samples.len()) - 1;
            samples[idx]
        }
    };
    serde_json::json!({
        "p50_ms": pick(0.50),
        "p90_ms": pick(0.90),
        "p95_ms": pick(0.95),
        "p99_ms": pick(0.99),
        "max_ms": samples.last().copied().unwrap_or(0),
        "samples": samples.len(),
    })
}

#[derive(Default)]
struct Inner {
    sessions_started: u64,
    sessions_ended: u64,
    /// 助手轮次总数（含被中断的轮次——发生了的轮就是轮）。
    turns: u64,
    /// 整轮耗时样本（环形，最近 [`TURN_WINDOW`] 轮）。
    turn_elapsed_ms: VecDeque<u64>,
    tts_rounds: u64,
    tts_frames: u64,
    tts_audio_ms: u64,
    /// 被打断（barge-in）的 TTS 轮次。
    interrupted: u64,
    errors: u64,
    last_error: Option<String>,
    /// 分段延迟窗口（缺口②）。
    stages: StageWindows,
}

/// 进程级聚合器。挂在 [`crate::engine::Engines`] 上（与 registry/usage 同性质的只读观测面）。
#[derive(Clone)]
pub struct Metrics {
    inner: Arc<Mutex<Inner>>,
    started: Instant,
}

impl Metrics {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(Inner::default())), started: Instant::now() }
    }

    /// 消费一条事件并更新聚合（订阅者唯一入口；绝不允许 panic 外溢）。
    pub fn observe(&self, e: &ServerEvent) {
        let Ok(mut g) = self.inner.lock() else { return };
        match e {
            ServerEvent::SessionStart { .. } => g.sessions_started += 1,
            ServerEvent::SessionEnd { .. } => g.sessions_ended += 1,
            ServerEvent::MessageAdded { role, elapsed_ms, .. } => {
                // 轮次计数与整轮耗时样本的**唯一来源**：只有 `role == "assistant"` 的
                // MessageAdded 才算"一轮"（用户那条只是同一轮的另一半，见 `turn_id`）。
                // `TurnLatency` **绝不**参与这两个计数——否则一轮会被记成多轮。
                if role == "assistant" {
                    g.turns += 1;
                    if let Some(ms) = elapsed_ms {
                        g.turn_elapsed_ms.push_back(*ms);
                        while g.turn_elapsed_ms.len() > TURN_WINDOW {
                            g.turn_elapsed_ms.pop_front();
                        }
                    }
                }
            }
            ServerEvent::TtsRound { frames, audio_ms, interrupted, .. } => {
                g.tts_rounds += 1;
                g.tts_frames += u64::from(*frames);
                g.tts_audio_ms += *audio_ms;
                if *interrupted {
                    g.interrupted += 1;
                }
            }
            ServerEvent::Error { message, .. } => {
                g.errors += 1;
                g.last_error = Some(message.clone());
            }
            ServerEvent::TurnLatency { stages, .. } => {
                // ⚠️ **按轮次计数，不是按分段计数**（本任务最高回归风险处）：
                // 一轮 = 恰好一条 `TurnLatency`，它带的 5 个阶段是同一轮的 5 次采样。
                // 下面每个 `Some(ms)` 各推 **1 个**样本进**各自的**窗口——一轮共推 5 个
                // 样本，但分属 5 个不同窗口，因此**每个窗口每轮只 +1**。
                // 若改成"一个事件往每个窗口推 5 次"或"按分段条数累加 turns"，同一轮就会
                // 被自身重复加权，分位数彻底失真。回归测试见 `tests.rs` 的
                // `stages_of_one_turn_count_as_one_sample_per_window`。
                //
                // 分窗口径（QA Round 1 拍板）：仅语音/注入轮入窗——canned（兜底/问候/
                // 告别，常走缓存零合成）与 aiui（云端闭环，本地无中间里程碑）与
                // 正常语音轮的延迟结构完全不同，混窗会系统性拉低语音轮分位。
                // anchor 在此消费（事件负载字段，非 dead code）。
                if !matches!(stages.anchor, ANCHOR_SPEECH_END | ANCHOR_INJECT) {
                    return;
                }
                let s = &mut g.stages;
                // 各阶段 Some(ms) 分别入窗（None = 本轮未发生，不进窗——否则会拉低分位）
                if let Some(v) = stages.asr_end_ms {
                    push_window(&mut s.asr_end_ms, v);
                }
                if let Some(v) = stages.llm_first_token_ms {
                    push_window(&mut s.llm_first_token_ms, v);
                }
                if let Some(v) = stages.sentence_ready_ms {
                    push_window(&mut s.sentence_ready_ms, v);
                }
                if let Some(v) = stages.tts_first_pcm_ms {
                    push_window(&mut s.tts_first_pcm_ms, v);
                }
                if let Some(v) = stages.first_audio_ms {
                    push_window(&mut s.first_audio_ms, v);
                }
            }
        }
    }

    /// `GET /api/metrics` 响应体（手写 wire format，契约稳定可单测）。
    pub fn snapshot_json(&self) -> serde_json::Value {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut samples: Vec<u64> = g.turn_elapsed_ms.iter().copied().collect();
        samples.sort_unstable();
        // 分位取"向上取整位"（p95 of 100 样本 = 第 95 个）；空样本一律 0。
        let pick = |q: f64| -> u64 {
            if samples.is_empty() {
                0
            } else {
                let idx = (((samples.len() as f64) * q).ceil() as usize).clamp(1, samples.len()) - 1;
                samples[idx]
            }
        };
        serde_json::json!({
            "note": "内存态，进程重启清零；整轮延迟样本窗口为最近 200 轮",
            "uptime_ms": self.started.elapsed().as_millis() as u64,
            "sessions": { "started": g.sessions_started, "ended": g.sessions_ended },
            "turns": {
                "total": g.turns,
                "elapsed_p50_ms": pick(0.50),
                "elapsed_p90_ms": pick(0.90),
                "elapsed_p95_ms": pick(0.95),
                "elapsed_p99_ms": pick(0.99),
                "elapsed_max_ms": samples.last().copied().unwrap_or(0),
                "samples": samples.len(),
            },
            "tts": {
                "rounds": g.tts_rounds,
                "frames": g.tts_frames,
                "audio_ms": g.tts_audio_ms,
                "interrupted": g.interrupted,
            },
            "errors": { "count": g.errors, "last": g.last_error },
            // 缺口②（additive）：各阶段相对锚点的毫秒偏移，仅统计**语音/注入轮**
            //（canned=兜底问候告别常走缓存、aiui=云端闭环，延迟结构与语音轮不同，
            // 混窗会拉低分位，故不入窗——见 observe() 的过滤）。缓存命中轮
            // tts_first_pcm_ms 为空（零合成无从谈首 PCM）。
            "latency": {
                "note": "各阶段相对锚点的毫秒偏移；仅统计 speech_end（语音）/ inject（注入）轮，canned/aiui 轮不入窗；缓存命中轮 tts_first_pcm 为空",
                "asr_end_ms": stage_stats_json(&g.stages.asr_end_ms),
                "llm_first_token_ms": stage_stats_json(&g.stages.llm_first_token_ms),
                "sentence_ready_ms": stage_stats_json(&g.stages.sentence_ready_ms),
                "tts_first_pcm_ms": stage_stats_json(&g.stages.tts_first_pcm_ms),
                "first_audio_ms": stage_stats_json(&g.stages.first_audio_ms),
            },
        })
    }
}

/// 常驻订阅任务：进程启动时 spawn（早于任何会话），`LAGGED` 只告警不退出。
pub async fn subscriber(metrics: Metrics, mut rx: broadcast::Receiver<ServerEvent>) {
    loop {
        match rx.recv().await {
            Ok(e) => metrics.observe(&e),
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("metrics 订阅者消费落后，丢弃 {n} 条事件（聚合口径为近似值）");
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

#[cfg(test)]
mod tests;
