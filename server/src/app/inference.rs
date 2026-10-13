//! 推理并发闸门（缺口①）：ASR/TTS 各一把 [`Semaphore`]，包在 `spawn_blocking` 调用点外。
//!
//! ## 为什么需要
//!
//! 四大引擎的重推理（ASR 识别 / TTS 合成）是同步 CPU 密集调用，统一经 `spawn_blocking`
//! 隔离——但 `spawn_blocking` **无并发上限**（tokio blocking 池默认 512 线程）：多设备同时
//! 说话 = 多路 ASR（各 `num_threads` 线程）+ 多路 TTS 同时压上小主机 → CPU 超订、延迟雪崩。
//! `engine.rs //!` 声明的线程预算（tokio 2 + ASR 2 + TTS 4 + VAD 1）只在"每能力同时
//! **1 路**推理"时成立——闸门就是把这条隐含前提变成显式约束。
//!
//! ## 设计（范式对齐 `plugins/vad/pool.rs`）
//!
//! - 挂在 [`crate::engine::Engines`] 上（与 `vad_pool`、`tts_cache` 同性质的进程级共享资源）：
//!   引擎热切换替换的是 `Arc<dyn TtsEngine>`，**闸门不随引擎走**——限的是"并发推理路数"；
//! - 获取一律 `acquire_owned()`（future 被 drop 即取消，cancel-safe），permit **move 进
//!   `spawn_blocking` 闭包**或随作用域 RAII 归还，绝不手工 release；
//! - 许可数可配（`[asr]/[tts].max_concurrent`，默认 1）：本地引擎每路占 `num_threads` 线程，
//!   默认 1 路恰等于段预算；在线引擎（如讯飞）或算力富余可放宽，无需改代码。
//!
//! ## 与 abort 语义协同（不对称，刻意为之）
//!
//! - **TTS 侧**：等待可被 abort 打断（[`GateHandle::acquire_with_abort`]，200ms 轮询，
//!   粒度对齐 `downlink/sender.rs::POLL_MS`）——打断后不白等；
//! - **ASR 侧**：**不**响应 abort（裸 [`GateHandle::acquire`]）。`abort` 只在
//!   `stream_response` 入口复位，而 ASR 发生在它**之前**——上一轮打断遗留的 stale
//!   `abort=true` 会让"打断后立刻说的新话语"在排队时被静默丢弃（barge-in 最常见的
//!   行为序列），现有代码在 ASR 路径也刻意不查 abort，本模块保持该语义。
//!
//! ## 热切换对账
//!
//! ASR 段是 `RestartOnly`（许可启动时定死）；TTS 段是 `NextSession`——
//! [`crate::engine::Engines::refresh_tts`] 成功路径调 [`InferenceGates::reconfigure_tts`]：
//! 许可数变化就换新 Semaphore（在飞 permit 持旧 Arc，切换瞬间并发上限短暂 = 新旧之和，
//! 频率极低，接受）。

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::config::Config;

/// abort 感知获取的轮询粒度（对齐 `downlink/sender.rs::POLL_MS` 的轮询语义）。
const ABORT_POLL_MS: u64 = 200;

/// 排队 ≥ 该值打 info 日志（低于只记账进 `/api/metrics`）。
const LOUD_WAIT_MS: u64 = 100;

/// 单能力闸门的等待计数（获取成功时若等待 ≥ 1ms 记账）。
#[derive(Default)]
struct GateCounters {
    wait_events: AtomicU64,
    wait_ms_total: AtomicU64,
    wait_ms_max: AtomicU64,
}

/// 一把闸门：Semaphore（可热换）+ 当前许可数 + 等待计数。
struct Gate {
    name: &'static str,
    /// 当前 Semaphore（`reconfigure` 换新；读多写少 → RwLock）。
    sem: RwLock<Arc<Semaphore>>,
    /// 当前许可数（对账/展示用）。
    permits: AtomicUsize,
    counters: GateCounters,
}

impl Gate {
    fn new(name: &'static str, permits: u32) -> Arc<Self> {
        // clamp ≥ 1：0 许可会把该能力的推理永久堵死。
        let permits = permits.max(1) as usize;
        Arc::new(Self {
            name,
            sem: RwLock::new(Arc::new(Semaphore::new(permits))),
            permits: AtomicUsize::new(permits),
            counters: GateCounters::default(),
        })
    }

    /// 当前 Semaphore（每次获取时取一次读锁后 clone Arc，开销可忽略；
    /// `reconfigure` 之后的新获取立即走新闸门）。
    fn current(&self) -> Arc<Semaphore> {
        self.sem.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 获取成功后记账排队等待（≥1ms 才记；≥100ms 升 info）。
    fn note_wait(&self, waited: Duration) {
        let ms = waited.as_millis() as u64;
        if ms == 0 {
            return;
        }
        let c = &self.counters;
        c.wait_events.fetch_add(1, Ordering::Relaxed);
        c.wait_ms_total.fetch_add(ms, Ordering::Relaxed);
        c.wait_ms_max.fetch_max(ms, Ordering::Relaxed);
        if ms >= LOUD_WAIT_MS {
            tracing::info!(
                "推理闸门 {}：排队 {}ms 后获得许可（当前上限 {} 路并发推理）",
                self.name,
                ms,
                self.permits.load(Ordering::Relaxed)
            );
        } else {
            tracing::debug!("推理闸门 {}：排队 {}ms", self.name, ms);
        }
    }

    /// 裸获取（ASR 侧 / 无打断语义的调用方）：排队等待，cancel-safe。
    /// 返回 `None` 仅在信号量被 close 时（本进程从不 close，防御性处理）。
    async fn acquire(&self) -> Option<OwnedSemaphorePermit> {
        let t0 = Instant::now();
        let permit = self.current().acquire_owned().await.ok()?;
        self.note_wait(t0.elapsed());
        Some(permit)
    }

    /// abort 感知获取（TTS 侧）：200ms 轮询 abort，置位即放弃（返回 `None`）。
    /// 取消安全：`timeout` 竞争失败只是放弃等待，permit 未取走、不丢失。
    async fn acquire_abortable(&self, abort: &AtomicBool) -> Option<OwnedSemaphorePermit> {
        let t0 = Instant::now();
        loop {
            if abort.load(Ordering::Relaxed) {
                return None;
            }
            let sem = self.current();
            match tokio::time::timeout(Duration::from_millis(ABORT_POLL_MS), sem.acquire_owned())
                .await
            {
                Ok(Ok(permit)) => {
                    self.note_wait(t0.elapsed());
                    return Some(permit);
                }
                // 我们从不 close 信号量；真发生说明出现结构性 bug，放弃排队并留痕。
                Ok(Err(_)) => {
                    tracing::error!("推理闸门 {} 的信号量已被关闭（不应发生），放弃本次排队", self.name);
                    return None;
                }
                // 本轮超时：复查 abort 后继续排队
                Err(_elapsed) => continue,
            }
        }
    }

    /// 热切换对账：许可数变化 → 换新 Semaphore。
    /// 在飞 permit 持旧 Arc：切换瞬间并发上限短暂 = 新旧之和（见模块注释）。
    fn reconfigure(&self, permits: u32) {
        let permits = permits.max(1) as usize;
        if permits == self.permits.load(Ordering::Relaxed) {
            return;
        }
        *self.sem.write().unwrap_or_else(|e| e.into_inner()) =
            Arc::new(Semaphore::new(permits));
        self.permits.store(permits, Ordering::Relaxed);
        tracing::info!("推理闸门 {}：许可数已调整为 {permits}", self.name);
    }

    fn stats_json(&self) -> serde_json::Value {
        let c = &self.counters;
        serde_json::json!({
            "permits": self.permits.load(Ordering::Relaxed),
            "wait_events": c.wait_events.load(Ordering::Relaxed),
            "wait_ms_total": c.wait_ms_total.load(Ordering::Relaxed),
            "wait_ms_max": c.wait_ms_max.load(Ordering::Relaxed),
        })
    }
}

/// 闸门句柄：clone 开销可忽略（一个 `Arc`），调用方把它 move 进 worker task。
///
/// 统一获取入口（两个方法的差别只在要不要响应 abort，见模块注释的"与 abort 语义协同"）：
/// permit 一律 RAII 归还——随 `spawn_blocking` 闭包结束或所在作用域退出，绝不手工 release。
#[derive(Clone)]
pub struct GateHandle {
    gate: Arc<Gate>,
}

impl GateHandle {
    /// 裸获取（ASR 路径 / HTTP 内联测试等无打断语义的调用方）。
    pub async fn acquire(&self) -> Option<OwnedSemaphorePermit> {
        self.gate.acquire().await
    }

    /// abort 感知获取（TTS 路径）：排队期间 abort 置位 → 放弃等待返回 `None`。
    pub async fn acquire_with_abort(
        &self,
        abort: &AtomicBool,
    ) -> Option<OwnedSemaphorePermit> {
        self.gate.acquire_abortable(abort).await
    }
}

/// 进程级推理闸门：ASR/TTS 各一把，挂在 [`crate::engine::Engines`] 上跨会话共享。
pub struct InferenceGates {
    asr: Arc<Gate>,
    tts: Arc<Gate>,
}

impl InferenceGates {
    /// 启动时按配置构造（`[asr].max_concurrent` / `[tts].max_concurrent`，均 clamp ≥ 1）。
    pub fn new(cfg: &Config) -> Self {
        Self {
            asr: Gate::new("asr", cfg.asr.max_concurrent),
            tts: Gate::new("tts", cfg.tts.max_concurrent),
        }
    }

    /// 取当前 ASR 闸门（调用方 `acquire()` 后把 permit 带进 `spawn_blocking`）。
    pub fn asr(&self) -> GateHandle {
        GateHandle { gate: self.asr.clone() }
    }

    /// 取当前 TTS 闸门（调用方 `acquire_with_abort()`）。
    pub fn tts(&self) -> GateHandle {
        GateHandle { gate: self.tts.clone() }
    }

    /// TTS 热切换对账（[`crate::engine::Engines::refresh_tts`] 成功路径调用）：
    /// 许可数变化 → 换新 Semaphore（在飞 permit 持旧 Arc，短暂重叠可接受）。
    pub fn reconfigure_tts(&self, permits: u32) {
        self.tts.reconfigure(permits);
    }

    /// `/api/metrics` 合并用投影（**additive**：只新增 `inference_gates` 节，老字段零改动）。
    pub fn stats_json(&self) -> serde_json::Value {
        serde_json::json!({
            "inference_gates": {
                "note": "ASR/TTS 并发推理闸门（[asr]/[tts].max_concurrent，默认 1 路）；等待统计只记 ≥1ms 的排队",
                "asr": self.asr.stats_json(),
                "tts": self.tts.stats_json(),
            }
        })
    }
}

#[cfg(test)]
mod tests;
