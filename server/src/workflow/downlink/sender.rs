//! 下行**发送端**：从水库按实时节拍抽取「句首通知 / 音频帧」下发设备。
//!
//! 与 [`crate::workflow::downlink::DownlinkPacer`] 分工：
//! - 本模块负责「取什么、什么时候发」——顺序、`tts start` 起跑缓冲、打断轮询、保活；
//! - [`DownlinkPacer`] 负责「发多快」——每帧目标时刻 = 锚点 + N×帧长 − lead，
//!   **不得移除**（ESP 解码队列硬约束，见 `downlink.rs` 模块文档）。
//!
//! ## 起跑闸（`[audio].downlink_buffer_start_ms`）
//! 首帧之前先等水库攒够 `start_ms`（或生产端收工 / 超时 `start_max_ms`）。
//! 为什么值得这点延迟：上游"略慢于实时"时，立刻开播只会把缺口摊成一路断续
//! （听感最差）；先攒一段再播，缺口被水库吸收，代价是一次性起跑等待。
//! 短回复（生产端已收工）**不会**白等——那时闸门立刻放行。

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::media::media::{AudioChunk, AudioWrite};
use crate::workflow::downlink::reservoir::{DownlinkItem, ReservoirSource, ReservoirStats};
use crate::workflow::downlink::{poll_abort, send_sentence_start, start_tts_guarded, DownlinkPacer};
use crate::workflow::session::SessionParams;
use crate::workflow::protocol::Transport;

/// 一轮下行的发送观测（与 [`crate::workflow::downlink::PacerStats`] 一起打印）。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ReservoirDrain {
    /// 已下发句数（`sentence_start` 次数）
    pub sentences: usize,
    /// 已下发音频帧数
    pub frames: usize,
    /// 是否被打断（abort / 承载断开）
    pub interrupted: bool,
    /// 是否已下发过 `tts start`（决定调用方要不要补 `tts stop`）
    pub tts_started: bool,
    /// 本轮水库观测
    pub reservoir: ReservoirStats,
}

/// 等下一项时的轮询粒度：打断（barge-in）与保活的最坏响应延迟。
const POLL_MS: u64 = 200;

/// 从水库按实时节拍下发，直到生产端收工且抽空、或被打断。
///
/// `t0` = 本轮回复起点（TTFA 口径：用户文本就绪 → **首帧实际下发**）；
/// `first_frame_at` 跨句共享：只有本轮首帧真正发出时才写入。
#[allow(clippy::too_many_arguments)] // 发送端要拿传输/协商参数/打断/时间戳/节拍器/水库/观测
pub(crate) async fn drain_reservoir<T: Transport>(
    transport: &mut T,
    params: &SessionParams,
    session_id: &str,
    abort: &Arc<AtomicBool>,
    downlink_ts: &mut u32,
    pacer: &mut DownlinkPacer,
    mut source: ReservoirSource,
    t0: Instant,
    first_frame_at: &mut Option<Instant>,
) -> Result<ReservoirDrain> {
    let frame_ms = params.downlink_frame_ms.max(1) as u64;
    let mut out = ReservoirDrain::default();

    // 起跑闸必须在**任何**下发之前（含 `tts start`）：固件进入 Speaking 后会 ResetDecoder，
    // 抢在状态切换前灌的音频被静默丢弃（见 `start_tts_guarded` 文档）。
    if wait_start_gate(transport, params, session_id, abort, &source).await? {
        out.interrupted = true;
        out.reservoir = source.stats();
        return Ok(out);
    }
    // 起跑闸观测先落进结果（首帧日志要用它）
    out.reservoir = source.stats();

    let mut last_ping = Instant::now();
    loop {
        // 取下一项：水库空则等待，同时轮询打断（barge-in）与长静默期保活
        let item = match tokio::time::timeout(Duration::from_millis(POLL_MS), source.recv()).await {
            Ok(Some(item)) => item,
            // 生产端收工且水库已抽空 → 本轮下发自然结束
            Ok(None) => break,
            Err(_elapsed) => {
                // 轮询超时 = 这 200ms 水库是空的：补记"上游断供"（`recv` 被超时取消，
                // 它的记账路径不会执行；长断供会被切成多段，不补记就观测不到）
                source.note_starve(Duration::from_millis(POLL_MS));
                if poll_abort(transport, session_id, abort) {
                    out.interrupted = true;
                    break;
                }
                if last_ping.elapsed() >= Duration::from_secs(10) {
                    last_ping = Instant::now();
                    transport.keepalive().await;
                }
                continue;
            }
        };
        match item {
            DownlinkItem::SentenceStart(text) => {
                ensure_tts_started(transport, session_id, frame_ms, &mut out).await?;
                send_sentence_start(transport, session_id, &text).await?;
                out.sentences += 1;
            }
            DownlinkItem::Frame(opus) => {
                // 防御：正常路径 `SentenceStart` 必先到达；异常时也要先 `tts start`
                ensure_tts_started(transport, session_id, frame_ms, &mut out).await?;
                let profile = transport.terminal_profile(params);
                let audio = AudioChunk::new(params.output_format(), opus);
                pacer.pace(frame_ms).await;
                if transport.send_audio(audio, profile, *downlink_ts).await?
                    == AudioWrite::Cancelled
                {
                    out.interrupted = true;
                    break;
                }
                pacer.mark_sent(Instant::now());
                *downlink_ts = downlink_ts.wrapping_add(params.downlink_frame_ms);
                if first_frame_at.is_none() {
                    let now = Instant::now();
                    *first_frame_at = Some(now);
                    tracing::info!(
                        "session {session_id} 首帧音频已下发（TTFA≈{}ms，起跑闸等待 {}ms / 水位 {}ms，水库峰值 {}ms）",
                        now.duration_since(t0).as_millis(),
                        out.reservoir.gate_ms,
                        out.reservoir.gate_depth_ms,
                        out.reservoir.peak_ms
                    );
                }
                out.frames += 1;
                if poll_abort(transport, session_id, abort) {
                    out.interrupted = true;
                    break;
                }
            }
        }
    }
    out.reservoir = source.stats();
    Ok(out)
}

/// 首次下发前补 `tts start` + 一帧起跑缓冲（幂等）。
async fn ensure_tts_started<T: Transport>(
    transport: &mut T,
    session_id: &str,
    frame_ms: u64,
    out: &mut ReservoirDrain,
) -> Result<()> {
    if out.tts_started {
        return Ok(());
    }
    start_tts_guarded(transport, session_id, frame_ms as u32).await?;
    out.tts_started = true;
    Ok(())
}

/// 起跑闸：等水库攒够 `downlink_buffer_start_ms`（或生产端收工 / 超时）。返回 true = 被打断。
async fn wait_start_gate<T: Transport>(
    transport: &mut T,
    params: &SessionParams,
    session_id: &str,
    abort: &Arc<AtomicBool>,
    source: &ReservoirSource,
) -> Result<bool> {
    let start_ms = params.downlink_buffer_start_ms as u64;
    if start_ms == 0 {
        return Ok(false);
    }
    let max_ms = params
        .downlink_buffer_start_max_ms
        .max(params.downlink_buffer_start_ms) as u64;
    let t0 = Instant::now();
    let deadline = t0 + Duration::from_millis(max_ms);
    loop {
        if source.depth_ms() >= start_ms {
            break; // 攒够起跑缓冲
        }
        if source.producer_done() {
            break; // 短回复：生产端已收工，没什么可等（不白加延迟）
        }
        if Instant::now() >= deadline {
            tracing::info!(
                "session {session_id} 起跑闸到点（{}ms）：水库只有 {}ms（< {start_ms}ms），\
                 上游供数慢于实时；按现有存货起跑",
                max_ms,
                source.depth_ms()
            );
            break;
        }
        // 水位变化通知 + 100ms 兜底（通知可能在我们注册前发生）
        let _ = source.wait_depth_change(Duration::from_millis(100)).await;
        if poll_abort(transport, session_id, abort) {
            source.record_gate(t0.elapsed());
            return Ok(true);
        }
    }
    source.record_gate(t0.elapsed());
    Ok(false)
}

#[cfg(test)]
mod tests;
