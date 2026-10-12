//! 延迟/事件聚合（P1-1）：事件总线的**常驻订阅者** + `GET /api/metrics` 只读投影。
//!
//! ## 定位
//!
//! 这是 P0-5 事件总线的第一个订阅者：会话只管 `emit`，本模块独立消费并聚合——
//! "新增观测维度只改事件结构 + 订阅者，不动 session"的解耦目标由本模块兑现。
//!
//! - **全内存态、进程重启清零**（口径与 `app/registry` 一致）；前端页面必须注明。
//! - 整轮延迟样本 = 助手轮次的 `elapsed_ms`（事件里已带），环形窗口最近 [`TURN_WINDOW`] 轮。
//! - **每模块被动首包延迟暂未纳入**：正常会话中 VAD/ASR/LLM/TTS 的分模块首包目前只在
//!   测试台（`bench.rs`）与内联测试（`bench_api.rs`）测量；后续若要被动采集，在
//!   `reply.rs` 的流式回调处补事件字段即可（订阅者无需改 session）。
//!
//! ## 订阅时序
//!
//! `main.rs` 在 `Engines::new` 之后、`axum::serve` 之前 spawn [`subscriber`]——进程最早
//! 的订阅者，避免 broadcast 环形缓冲丢早期事件（`LAGGED` 只告警不退出）。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::broadcast;

use crate::app::events::ServerEvent;

/// 整轮延迟样本窗口（p50/p95 的计算样本数；超出丢最旧）。
const TURN_WINDOW: usize = 200;

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
                "elapsed_p95_ms": pick(0.95),
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
