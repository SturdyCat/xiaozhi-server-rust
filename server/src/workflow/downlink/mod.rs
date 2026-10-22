//! 下行音频链路：合成产物 → **水库**（服务器内存缓冲几秒）→ 按实时节拍下发设备。
//!
//! 三个子模块分工（父模块只留节拍器与协议原语）：
//! - [`framer`]：PCM 分片 → 重采样 → Opus 逐帧编码（跨分片复用相位/编码器状态）；
//! - [`reservoir`]：**已编码帧的内存水库**（生产者/消费者解耦 + 背压 + 水位观测）；
//! - [`sender`]：按实时节拍从水库抽取并下发（起跑闸、打断轮询、保活）。
//!
//! ## ⚠️ 下发必须按实时节奏节流（ESP 队列硬约束，勿删 [`DownlinkPacer`]）
//! ESP 固件解码队列上限 `MAX_DECODE_PACKETS_IN_QUEUE = 1200/60 = 20 包（≈1.2s）`，
//! **队满即静默丢包**（`PushPacketToDecodeQueue(wait=false)` 返回 false，无日志）。
//! TTS 合成远快于实时（实测讯飞在线 ~7 倍速、整段一次性回传时更快），若尽速灌入，
//! 长回复必然溢出：设备先播完 1.2s 缓冲、随后队列在阈值上下震荡 → 「先听到头几个字、
//! 然后卡住、接着断断续续」（短回复能装下故正常；macApp 无队列上限故正常——勿以此判
//! 断服务端无问题）。官方 Python 服务端同样有 `AudioRateController` 按 60ms/帧精确节流，
//! [`DownlinkPacer`] 即其 Rust 对齐实现：每帧目标时刻 = 锚点 + N×帧长 − lead_ms
//! （绝对时间表 + 可配置提前量，见 `[audio].downlink_lead_ms`，不累积漂移）；
//! 提前量让设备解码队列常备少量存货以吸收发送抖动，同时远低于 20 包的队满丢包线；
//! 生产者更慢（本地 Kokoro RTF>1）时自然立即发送、零额外等待；
//! 数据源真空 ≥ max(lead, 1帧) 后重锚定（播放已追平，不回补突发）。
//!
//! ⚠️ 真空判定基于**两次实际下发之间的间隔**（[`DownlinkPacer::last_sent_at`]），
//! 因此句与句之间的等待（LLM 还没吐出下一句、下一句合成还没出首片）同样纳入重锚定。
//!
//! ## 设备队列只有 ~1s，长回复靠**服务器侧水库**（见 [`reservoir`]）
//! 上游（LLM 出句 / 在线 TTS 首片）只要持续慢于实时，设备那 ~1s 存货必然被抽干。
//! 服务端能做的只有一件事：把**已经合成好的音频**多备几秒在内存里，发送端从水库取。
//! 于是「上游停顿 ≤ 水库水位 + 设备队列」时听感连续；代价是内存（Opus 帧极小）与
//! 起跑延迟（`[audio].downlink_buffer_start_ms`）。

pub(crate) mod framer;
pub(crate) mod reservoir;
pub(crate) mod sender;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::workflow::protocol::{ClientMessage, ServerMessage};
use crate::workflow::session::send_text;
use crate::workflow::protocol::{IncomingFrame, Transport};

pub(crate) use reservoir::{new_reservoir, spawn_reservoir_filler, spawn_single_sentence_filler};
pub(crate) use sender::{drain_reservoir, ReservoirDrain};

/// 下行节拍器：按帧长匀速下发，并**整体提前** `lead_ms`（抖动缓冲，ESP 解码队列保护）。
///
/// 语义：第 N 帧目标时刻 = `锚点 + N×帧长 − lead_ms`。开局先突发下发约
/// `lead_ms/帧长` 帧（设备解码队列预充），此后维持恒定提前量匀速——设备侧
/// 常备约 `lead_ms` 的音频存货，网络/调度抖动或**上游合成/LLM 的停顿**不再直接
/// 打穿设备播放缓冲。提前量受固件解码队列约束：20 包（60ms 帧 ≈1200ms），
/// 开局预充 `ceil(lead/帧长)+1` 包，故安全上限 ≈ `1200 − 2×帧长`（60ms 帧 → 1080ms），
/// 默认 1000ms 贴极限；配置超限由 `config::normalize` 收敛并告警。
///
/// ⚠️ 提前量 = **打断后的"停口"时间**（实测固件语义，2026-10 核对 `xiaozhi-esp32`）：
/// `tts stop` **不清空**解码队列，设备会先把已缓存的音频播完（`MAIN_EVENT_PLAYBACK_DRAINED`
/// + `pending_listening_start_`）才进入聆听。因此 lead 越大，打断后残留语音越多
///   （240ms ≈ 一个字，1000ms ≈ 三个字）。这是"抗抖动"与"打断灵敏"的折中。
///
/// 生命周期 = **一次回复**（多句连续播放共享同一时间表，句间无缝；新回复 [`Self::reset`]）。
#[derive(Debug)]
pub(crate) struct DownlinkPacer {
    /// 时间表锚点（首个 pace 调用时惰性设置）。
    anchor: Option<Instant>,
    /// 虚拟播放位置（毫秒）：第 N 帧目标时刻 = anchor + N×帧长 − lead_ms。
    pos_ms: u64,
    /// 抖动缓冲提前量（毫秒）：下发时间表比实时提前的固定量。
    lead_ms: u64,
    /// 上一帧**实际**下发时刻：真空判定（与"计划时刻"区分，sleep 调度延迟也算进真实间隔）。
    /// 只由 [`Self::mark_sent`] 写入（== `pace` 的睡眠之后）。
    last_sent_at: Option<Instant>,
    /// 本轮发生"真空重锚定"的次数与其中最长的一次（毫秒）。
    vacuum_events: u64,
    max_vacuum_ms: u64,
    /// 本轮**设备解码队列余量**的最低估计（毫秒）：`已下发音频 − 已流逝播放时间`。
    /// ≤0 即队列见底（听感断音）。比"单次真空"灵敏得多：上游只慢一点点（合成 RTF 略大于 1、
    /// LLM 出句略慢）时单次下发间隔够不上重锚定阈值，但缓冲会被**持续抽干**——
    /// 那正是"长回复越到后面越断续"的形态。`None` = 本轮还没测到（回复极短）。
    queue_min_ms: Option<i64>,
}

/// 本轮下行观测（诊断用）：见 `session::reply` 的每轮汇总日志。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PacerStats {
    /// 真空（两帧实际下发间隔 ≥ max(lead, 帧长)）次数
    pub vacuum_events: u64,
    /// 最长的一次真空（毫秒）
    pub max_vacuum_ms: u64,
    /// 设备解码队列余量最低估计（毫秒）；`None` = 未测到
    pub queue_min_ms: Option<i64>,
}

/// 默认提前量（≈17×60ms 帧 = 1000ms）：设备队列常备 1s 存货，吸收网络/调度抖动
/// 与上游 LLM 出句/TTS 合成的停顿——"前面流畅、后面断续"的直接解药之一（另一半是
/// 服务器侧水库，见 [`reservoir`]）。
/// 逼近设备侧硬上限（[`crate::config::AudioConfig::max_lead_ms`] = 队列 − 2 帧 = 1080ms）；
/// 想要更深必须先放大固件 `MAX_DECODE_PACKETS_IN_QUEUE`。
pub(crate) const DEFAULT_LEAD_MS: u64 = 1000;

impl Default for DownlinkPacer {
    fn default() -> Self {
        Self::new(DEFAULT_LEAD_MS)
    }
}

impl DownlinkPacer {
    /// 指定提前量（毫秒）构造；0 退化为"贴实时"严格节流（无预充）。
    pub(crate) fn new(lead_ms: u64) -> Self {
        Self {
            anchor: None,
            pos_ms: 0,
            lead_ms,
            last_sent_at: None,
            vacuum_events: 0,
            max_vacuum_ms: 0,
            queue_min_ms: None,
        }
    }

    /// 新一轮回复开始：重置时间表与观测统计。
    pub(crate) fn reset(&mut self) {
        self.anchor = None;
        self.pos_ms = 0;
        self.last_sent_at = None;
        self.vacuum_events = 0;
        self.max_vacuum_ms = 0;
        self.queue_min_ms = None;
    }

    /// 本轮下行观测统计（回复结束时汇总打印）。
    pub(crate) fn stats(&self) -> PacerStats {
        PacerStats {
            vacuum_events: self.vacuum_events,
            max_vacuum_ms: self.max_vacuum_ms,
            queue_min_ms: self.queue_min_ms,
        }
    }

    /// 记录"该帧已实际下发"。**必须在下发之后调用**（发送端在 socket write 完成之后调用）：
    /// 真空判定看的是两次真实下发的间隔，若在 sleep 之前记录，贴实时（lead=0）时
    /// 每帧都会把睡眠时长算成"真空"从而反复重锚定 → 节流失效。
    /// 单测用它在不 sleep 的前提下精确模拟下发时刻。
    fn mark_sent(&mut self, at: Instant) {
        self.last_sent_at = Some(at);
    }

    /// 一帧下发前的节流等待（未到目标时刻则等待；迟到/生产者慢则零等待）。
    pub(crate) async fn pace(&mut self, frame_ms: u64) {
        let wait = self.plan(Instant::now(), frame_ms);
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }

    /// 帧调度计算（纯函数，可测）：返回需等待时长并推进虚拟播放位置一帧。
    /// 目标 = 锚点 + 虚拟播放位置 − lead_ms；未到则等待，迟到/生产者慢则零等待。
    ///
    /// **真空重锚定**：距上一帧实际下发已 ≥ `max(lead, 帧长)` 时，数据源断供到足以
    /// 打穿设备缓冲——此刻虚拟播放位置已追平真实时间，把时间表重锚到 `now`，
    /// 后续帧恢复"提前 lead_ms"的匀速（而不是把积压的目标时刻一次性突发回补）。
    /// 阈值取 `max(lead, 帧长)` 的理由：小于提前量的短抖动正是提前量要吸收的东西，
    /// 重锚定反而会把预充的缓冲抖掉（对齐官方 `AudioRateController` 的空队列重锚定）。
    /// ⚠️ 阈值**不得小于帧长**，否则贴实时（lead=0）时每帧都判成真空、节流会失效
    ///（真机路径另有一道保险：[`Self::mark_sent`] 在下发之后才记录，见其文档）。
    fn plan(&mut self, now: Instant, frame_ms: u64) -> Duration {
        let threshold = self.lead_ms.max(frame_ms);
        if let Some(anchor) = self.anchor {
            // (1) 设备解码队列余量估计（**必须在重锚定之前**用旧时间表算：重锚定会把
            //     时间表对齐到"播放已追平"，从而抹掉断供证据）。
            //     统计口径：负值一定是断音（预充阶段不可能为负）→ 立刻记；
            //     正值只在**预充完成之后**才纳入（开局余量从 0 涨到 lead 是正常的，
            //     不是"最低余量"，否则每轮都误报断音）。
            if self.pos_ms > 0 {
                let played_ms = now.saturating_duration_since(anchor).as_millis() as i64;
                let depth = self.pos_ms as i64 - played_ms;
                if depth < 0 || self.pos_ms > self.lead_ms.saturating_add(frame_ms) {
                    self.queue_min_ms = Some(self.queue_min_ms.map_or(depth, |m| m.min(depth)));
                }
            }
            // (2) 真空重锚定
            if let Some(last) = self.last_sent_at {
                let gap_ms = now.duration_since(last).as_millis() as u64;
                if threshold > 0 && gap_ms >= threshold {
                    self.vacuum_events += 1;
                    self.max_vacuum_ms = self.max_vacuum_ms.max(gap_ms);
                    if let Some(a) = now.checked_sub(Duration::from_millis(self.pos_ms)) {
                        self.anchor = Some(a);
                    }
                }
            }
        }
        let anchor = *self.anchor.get_or_insert(now);
        let wait = match anchor
            .checked_add(Duration::from_millis(self.pos_ms))
            .and_then(|t| t.checked_sub(Duration::from_millis(self.lead_ms)))
        {
            Some(target) if target > now => target - now,
            _ => Duration::ZERO,
        };
        self.pos_ms = self.pos_ms.saturating_add(frame_ms);
        wait
    }
}

/// 非阻塞抽取承载中已到达的打断/关闭事件（流水线各等待点轮询）。
///
/// 返回当前打断标志（收到 abort、承载断开或已被置位均视为打断）。
/// 注意：播报期间到达的上行二进制音频会被丢弃——打断以 abort 文本消息为准
/// （ESP barge-in 时设备先发 abort 再重新 listen）。
pub(crate) fn poll_abort<T: Transport>(
    transport: &mut T,
    session_id: &str,
    abort: &Arc<AtomicBool>,
) -> bool {
    while let Some(frame) = transport.try_recv() {
        match frame {
            IncomingFrame::Closed => {
                tracing::debug!("session {session_id} 承载已断开（视为打断）");
                abort.store(true, Ordering::Relaxed);
                break;
            }
            IncomingFrame::Text(t) => {
                if matches!(
                    serde_json::from_str::<ClientMessage>(&t),
                    Ok(ClientMessage::Abort { .. })
                ) {
                    tracing::info!("session {session_id} 收到 abort（流水线终止）");
                    abort.store(true, Ordering::Relaxed);
                    break;
                }
            }
            // 播报期的上行音频/listen 等在轮询中丢弃
            IncomingFrame::Binary(_)
            | IncomingFrame::Audio(_)
            | IncomingFrame::AudioBoundary(_, _)
            | IncomingFrame::EndOfUtterance(_) => {}
        }
    }
    abort.load(Ordering::Relaxed)
}

/// 发送 `tts` 状态消息（`start`/`stop` 等，无正文）。
pub(crate) async fn send_tts_state<T: Transport>(
    transport: &mut T,
    session_id: &str,
    state: &str,
) -> Result<()> {
    send_text(
        transport,
        &ServerMessage::Tts {
            session_id: session_id.to_string(),
            state: state.to_string(),
            text: None,
        },
    )
    .await
}

/// 发送 `sentence_start(text)`：句首通知必须与音频同序（水库里是队列首项，
/// 发送端在**该句首帧之前**发出，见 [`reservoir::DownlinkItem`]）。
pub(crate) async fn send_sentence_start<T: Transport>(
    transport: &mut T,
    session_id: &str,
    text: &str,
) -> Result<()> {
    send_text(
        transport,
        &ServerMessage::Tts {
            session_id: session_id.to_string(),
            state: "sentence_start".to_string(),
            text: Some(text.to_string()),
        },
    )
    .await
}

/// 下发 `tts start` 并等**一帧**"起跑缓冲"，然后才允许首帧音频出发（两条下行路径共用）。
///
/// 为什么必须等：固件收到 `tts start` 只是 `Schedule()` 一次状态切换
///（`application.cc::OnIncomingJson`），而 `OnIncomingAudio` 在 `state != Speaking` 时
/// **直接丢弃**音频（零日志），且进入 Speaking 时会 `ResetDecoder()` 清队列。若把开局
/// 预充的 `lead/帧长 + 1` 帧紧贴着 `tts start` 灌下去，抢在状态切换前的那些帧会被静默
/// 丢掉 = 开头被吃掉（提前量越大丢得越多，默认 lead 下最多可丢 ~1s）。
/// 等一帧（60ms）几乎零成本（TTFA 本就是秒级），却让"状态切换 + 清队列"先于首帧到达。
pub(crate) async fn start_tts_guarded<T: Transport>(
    transport: &mut T,
    session_id: &str,
    frame_ms: u32,
) -> Result<()> {
    send_tts_state(transport, session_id, "start").await?;
    tokio::time::sleep(Duration::from_millis(frame_ms.max(1) as u64)).await;
    Ok(())
}

/// 末段 PCM 不足一整帧时补零到整帧（Opus 只接受合法帧长，否则 BadArgument）。
pub(crate) fn pad_tail(pcm: &[f32], frame: usize) -> Vec<f32> {
    let frame = frame.max(1);
    if pcm.is_empty() || pcm.len() >= frame {
        return Vec::new();
    }
    let mut v = Vec::with_capacity(frame);
    v.extend_from_slice(pcm);
    v.resize(frame, 0.0);
    v
}

#[cfg(test)]
mod tests;
