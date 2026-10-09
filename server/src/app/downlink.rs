//! 下行音频发送：把合成后的 PCM 重采样、Opus 分帧，逐帧经承载（[`crate::app::transport::Transport`]）
//! 下发设备，并在下行间隙轮询打断（barge-in）。
//!
//! 与 [`crate::app::session::Session`] 解耦为自由函数，仅依赖传入的传输实现 / 协商参数 /
//! 下行时间戳 / 打断标志 / [`DownlinkPacer`]——不持有会话可变状态，便于独立阅读与测试
//! （打断轮询可用内存 [`crate::app::transport::IncomingFrame`] 序列单测，无需真实连接）。
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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::app::audio::opus::OpusFrameEncoder;
use crate::app::audio::resample::StreamingResampler;
use crate::app::protocol::{ClientMessage, ServerMessage, wrap_downlink};
use crate::app::session::{send_binary, send_text, SessionParams};
use crate::app::transport::{IncomingFrame, Transport};
use tokio::sync::mpsc;

/// 下行节拍器：按帧长匀速下发，并**整体提前** `lead_ms`（抖动缓冲，ESP 解码队列保护）。
///
/// 语义：第 N 帧目标时刻 = `锚点 + N×帧长 − lead_ms`。开局先突发下发约
/// `lead_ms/帧长` 帧（设备解码队列预充），此后维持恒定提前量匀速——设备侧
/// 常备约 `lead_ms` 的音频存货，网络/调度抖动（几十 ms 的发送毛刺）不再直接
/// 打穿设备播放缓冲。提前量受固件解码队列上限约束：60ms 帧上限 20 包（1.2s），
/// 默认 240ms（4 包）安全；调大需注意 barge-in 响应延迟。
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
}

/// 默认提前量（4×60ms 帧）：设备队列预充 4 包，远低于 20 包上限。
pub(crate) const DEFAULT_LEAD_MS: u64 = 240;

impl Default for DownlinkPacer {
    fn default() -> Self {
        Self::new(DEFAULT_LEAD_MS)
    }
}

impl DownlinkPacer {
    /// 指定提前量（毫秒）构造；0 退化为"贴实时"严格节流（无预充）。
    pub(crate) fn new(lead_ms: u64) -> Self {
        Self { anchor: None, pos_ms: 0, lead_ms }
    }

    /// 新一轮回复开始：重置时间表。
    pub(crate) fn reset(&mut self) {
        self.anchor = None;
        self.pos_ms = 0;
    }

    /// 数据源真空观测（recv 等待 ≥ max(lead, 帧长)）：设备缓冲确已打穿、时间表重锚定到
    /// "当前=播放位置"，避免真空结束后把积压帧突发回补（对齐官方 `AudioRateController`
    /// 的空队列重锚定）。小于该阈值的短抖动**不**重锚定——提前量就是为吸收它而设，
    /// 重锚定反而会把预充的缓冲抖掉。
    pub(crate) fn note_producer_gap(&mut self, waited: Duration, frame_ms: u64) {
        let threshold = self.lead_ms.max(frame_ms);
        if threshold == 0 || (waited.as_millis() as u64) < threshold {
            return;
        }
        self.anchor = Some(Instant::now() - Duration::from_millis(self.pos_ms));
    }

    /// 帧调度计算（纯函数，可测）：返回需等待时长并推进虚拟播放位置一帧。
    /// 目标 = 锚点 + 虚拟播放位置 − lead_ms；未到则等待，迟到/生产者慢则零等待。
    fn plan(&mut self, now: Instant, frame_ms: u64) -> Duration {
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

    /// 一帧下发前的节流等待（未到目标时刻则等待；迟到/生产者慢则零等待）。
    pub(crate) async fn pace(&mut self, frame_ms: u64) {
        let wait = self.plan(Instant::now(), frame_ms);
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
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
            IncomingFrame::Binary(_) => {}
        }
    }
    abort.load(Ordering::Relaxed)
}

/// 发送 `tts` 状态消息（`start`/`stop`/`sentence_start` 等）。
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

/// 末段 PCM 不足一整帧时补零到整帧（Opus 只接受合法帧长，否则 BadArgument）。
/// 流式路径用它对最后一个残帧补零；整帧数据直接走 `wrap_downlink`，无需补零。
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

/// 流式单句下发：`sentence_start(text)` → 从 `rx` 收增量 PCM 分片，
/// **边收边**重采样（跨分片连续相位）+ 流式 Opus 编码 + 逐帧二进制下发；
/// `rx` 关闭后冲刷残帧并返回（调用方随后发 `tts stop`）。
///
/// `cache_sink` 传 `Some(&mut Vec)` 时同步收集已编码 Opus 帧负载（不含协议头），
/// 供调用方写入 [`crate::app::tts_cache::TtsCache`]；打断/发送失败时 sink 被清空
/// （残缺句不得入缓存）。
///
/// 关键：重采样器与 Opus 编码器都是**跨分片复用**的流式实例——每个分片单独处理
/// 会在边界产生相位跳变（重采样）与流重启（Opus），实听即"爆音/断续"。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_sentence_audio_stream<T: Transport>(
    transport: &mut T,
    params: &SessionParams,
    downlink_ts: &mut u32,
    abort: &Arc<AtomicBool>,
    session_id: &str,
    text: &str,
    tts_sr: u32,
    mut rx: mpsc::UnboundedReceiver<Vec<f32>>,
    pacer: &mut DownlinkPacer,
    mut cache_sink: Option<&mut Vec<Vec<u8>>>,
    // 首帧实际下发时刻（跨句共享：只记录本轮回复的第一帧；调用方据此打 TTFA）
    first_frame_at: &mut Option<Instant>,
) -> Result<usize> {
    send_text(
        transport,
        &ServerMessage::Tts {
            session_id: session_id.to_string(),
            state: "sentence_start".to_string(),
            text: Some(text.to_string()),
        },
    )
    .await?;

    let frame = ((params.downlink_sr as f32 * params.downlink_frame_ms as f32 / 1000.0) as usize).max(1);
    let frame_ms = params.downlink_frame_ms as u64;
    let mut resampler = StreamingResampler::new(tts_sr, params.downlink_sr);
    let mut encoder = OpusFrameEncoder::new(params.downlink_sr)?;
    let mut carry: Vec<f32> = Vec::new(); // 不足整帧的尾部（跨分片积攒）
    let mut frames_sent: usize = 0;
    // 缓存收集（可选）：sink 与 abort 标志都要在借用结束后使用 → 先取裸指针级联不可行，
    // 用局部 Option 代理，循环后统一回写/清空
    let mut collected: Option<Vec<Vec<u8>>> =
        cache_sink.is_some().then(Vec::new);

    // 收分片 → 重采样 → 攒满整帧即编码下发（下发节奏由 pacer 钉在实时，保护 ESP 解码队列）
    loop {
        let wait_start = Instant::now();
        let chunk = rx.recv().await;
        let Some(chunk) = chunk else { break };
        pacer.note_producer_gap(wait_start.elapsed(), frame_ms);
        let down = resampler.push(&chunk);
        carry.extend_from_slice(&down);
        while carry.len() >= frame {
            let opus = match encoder.encode_frame(&carry[..frame]) {
                Ok(o) => o,
                Err(e) => {
                    tracing::warn!("Opus 编码失败: {e}");
                    break;
                }
            };
            carry.drain(..frame);
            if let Some(c) = collected.as_mut() {
                c.push(opus.clone());
            }
            let framed = wrap_downlink(params.downlink_bin_ver, &opus, *downlink_ts);
            *downlink_ts += params.downlink_frame_ms;
            pacer.pace(frame_ms).await;
            if first_frame_at.is_none() {
                *first_frame_at = Some(Instant::now());
            }
            frames_sent += 1;
            send_binary(transport, framed).await?;
            if poll_abort(transport, session_id, abort) {
                tracing::info!("session {} 中断流式 TTS 下发（{text}）", session_id);
                // 残缺句不得入缓存
                if let Some(dst) = &mut cache_sink {
                    dst.clear();
                }
                return Ok(frames_sent);
            }
        }
    }
    // 尾帧：残样本补零到整帧（Opus 只接受合法帧长）
    if !carry.is_empty() && !abort.load(Ordering::Relaxed) {
        let tail = pad_tail(&carry, frame);
        match encoder.encode_frame(&tail) {
            Ok(opus) => {
                if let Some(c) = collected.as_mut() {
                    c.push(opus.clone());
                }
                let framed = wrap_downlink(params.downlink_bin_ver, &opus, *downlink_ts);
                *downlink_ts += params.downlink_frame_ms;
                pacer.pace(frame_ms).await;
                if first_frame_at.is_none() {
                    *first_frame_at = Some(Instant::now());
                }
                frames_sent += 1;
                send_binary(transport, framed).await?;
            }
            Err(e) => tracing::warn!("Opus 编码失败（流式尾帧）: {e}"),
        }
    }
    // 回写缓存收集（完整句才写；已在中途打断的上面已清空并 return）
    if let Some(out) = collected.take() {
        if let Some(dst) = cache_sink {
            *dst = out;
        }
    }    Ok(frames_sent)
}

/// 缓存命中路径：`sentence_start(text)` → 逐帧把**已编码**的 Opus 帧负载加协议头
/// 下发（零合成/重采样/编码计算），节奏同样由 [`DownlinkPacer`] 钉住（ESP 队列保护
/// 与流式路径完全一致）。打断时提前返回已发送帧数。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_cached_sentence_audio<T: Transport>(
    transport: &mut T,
    params: &SessionParams,
    downlink_ts: &mut u32,
    abort: &Arc<AtomicBool>,
    session_id: &str,
    text: &str,
    frames: &Arc<Vec<Vec<u8>>>,
    pacer: &mut DownlinkPacer,
    first_frame_at: &mut Option<Instant>,
) -> Result<usize> {
    send_text(
        transport,
        &ServerMessage::Tts {
            session_id: session_id.to_string(),
            state: "sentence_start".to_string(),
            text: Some(text.to_string()),
        },
    )
    .await?;

    let frame_ms = params.downlink_frame_ms as u64;
    let mut frames_sent: usize = 0;
    for opus in frames.iter() {
        let framed = wrap_downlink(params.downlink_bin_ver, opus, *downlink_ts);
        *downlink_ts += params.downlink_frame_ms;
        pacer.pace(frame_ms).await;
        if first_frame_at.is_none() {
            *first_frame_at = Some(Instant::now());
        }
        frames_sent += 1;
        send_binary(transport, framed).await?;
        if poll_abort(transport, session_id, abort) {
            tracing::info!("session {} 中断缓存 TTS 下发（{text}）", session_id);
            break;
        }
    }
    Ok(frames_sent)
}

#[cfg(test)]
mod tests;
