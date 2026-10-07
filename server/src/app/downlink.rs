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
//! [`DownlinkPacer`] 即其 Rust 对齐实现：每帧目标时刻 = 锚点 + N×帧长（绝对时间表，
//! 不累积漂移）；生产者更慢（本地 Kokoro RTF>1）时自然立即发送、零额外等待；
//! 数据源真空 ≥1 帧后重锚定（播放已追平，不回补突发）。

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

/// 下行节拍器：按帧长匀速下发（ESP 解码队列保护，语义见模块文档）。
///
/// 生命周期 = **一次回复**（多句连续播放共享同一时间表，句间无缝；新回复 [`Self::reset`]）。
#[derive(Debug, Default)]
pub(crate) struct DownlinkPacer {
    /// 时间表锚点（首个 pace 调用时惰性设置）。
    anchor: Option<Instant>,
    /// 虚拟播放位置（毫秒）：第 N 帧目标时刻 = anchor + N×帧长。
    pos_ms: u64,
}

impl DownlinkPacer {
    /// 新一轮回复开始：重置时间表。
    pub(crate) fn reset(&mut self) {
        self.anchor = None;
        self.pos_ms = 0;
    }

    /// 数据源真空观测（recv 等待 ≥1 帧长）：播放已追平、时间表重锚定到"当前=播放位置"，
    /// 避免真空结束后把积压帧突发回补（对齐官方 `AudioRateController` 的空队列重锚定）。
    pub(crate) fn note_producer_gap(&mut self, waited: Duration, frame_ms: u64) {
        if frame_ms == 0 || (waited.as_millis() as u64) < frame_ms {
            return;
        }
        self.anchor = Some(Instant::now() - Duration::from_millis(self.pos_ms));
    }

    /// 帧调度计算（纯函数，可测）：返回需等待时长并推进虚拟播放位置一帧。
    fn plan(&mut self, now: Instant, frame_ms: u64) -> Duration {
        let anchor = *self.anchor.get_or_insert(now);
        let wait = match anchor.checked_add(Duration::from_millis(self.pos_ms)) {
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
/// 关键：重采样器与 Opus 编码器都是**跨分片复用**的流式实例——每个分片单独处理
/// 会在边界产生相位跳变（重采样）与流重启（Opus），实听即"爆音/断续"。
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
                return Ok(frames_sent);
            }
        }
    }
    // 尾帧：残样本补零到整帧（Opus 只接受合法帧长）
    if !carry.is_empty() && !abort.load(Ordering::Relaxed) {
        let tail = pad_tail(&carry, frame);
        match encoder.encode_frame(&tail) {
            Ok(opus) => {
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
    Ok(frames_sent)
}

#[cfg(test)]
mod tests {
    use super::pad_tail;
    use super::poll_abort;
    use super::DownlinkPacer;
    use crate::app::transport::{IncomingFrame, Transport};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    /// 节拍器核心语义（纯 plan 计算，无 sleep）：
    /// 首帧零等待 → 按绝对时间表匀速（不累积漂移）→ 迟到零等待且时间表不倒退。
    #[test]
    fn pacer_keeps_real_time_cadence() {
        let mut p = DownlinkPacer::default();
        let base = Instant::now();
        // 首帧：锚点=now，目标=now → 零等待
        assert_eq!(p.plan(base, 60), Duration::ZERO);
        // 第二帧在目标前 50ms：等到 base+60
        assert_eq!(p.plan(base + Duration::from_millis(10), 60), Duration::from_millis(50));
        // 第三帧恰在目标时刻：零等待
        assert_eq!(p.plan(base + Duration::from_millis(120), 60), Duration::ZERO);
        // 迟到（如生产者慢）：零等待，且后续目标仍锚定原时间表（不倒退、不突发回补）
        assert_eq!(p.plan(base + Duration::from_millis(500), 60), Duration::ZERO);
        // 第四帧目标仍是 base+240（绝对时间表），base+500 已过 → 继续零等待
        assert_eq!(p.plan(base + Duration::from_millis(501), 60), Duration::ZERO);
    }

    /// 真实计时（async 走 sleep 路径）：N 帧按帧长铺开而非瞬时灌入。
    /// 3 帧 × 40ms：首帧零等待，总耗时 ≥ 2×40ms（守护 ESP 解码队列的核心行为）。
    #[tokio::test]
    async fn pacer_actually_sleeps_real_time() {
        let mut p = DownlinkPacer::default();
        let t0 = Instant::now();
        for _ in 0..3 {
            p.pace(40).await;
        }
        let elapsed = t0.elapsed();
        assert!(
            elapsed >= Duration::from_millis(75),
            "3 帧应按 40ms 节奏铺开（实测 {elapsed:?}）；瞬时灌入会撑爆 ESP 20 包解码队列"
        );
    }

    /// 数据源真空 ≥1 帧后重锚定：新帧立即下发（播放已追平），后续恢复 60ms 节奏。
    #[test]
    fn pacer_reanchors_after_producer_gap() {
        let mut p = DownlinkPacer::default();
        let base = Instant::now();
        assert_eq!(p.plan(base, 60), Duration::ZERO); // 第 1 帧
        // 真空 5 秒（recv 等待）→ 重锚定
        p.note_producer_gap(Duration::from_millis(5000), 60);
        // 重锚定后：目标 = now - pos + pos = now → 零等待（不把积压帧突发回补）
        assert_eq!(p.plan(Instant::now(), 60), Duration::ZERO);
        // 下一帧恢复匀速（目标 ≈ 60ms 后）
        let wait = p.plan(Instant::now(), 60);
        assert!(wait >= Duration::from_millis(58) && wait <= Duration::from_millis(60), "恢复节奏: {wait:?}");
        // 小于 1 帧的短真空：不重锚定（时间表保持原样——第 3 帧目标仍是 base+120）
        let mut q = DownlinkPacer::default();
        assert_eq!(q.plan(base, 60), Duration::ZERO); // 第 1 帧（目标 base+0）
        assert_eq!(q.plan(base + Duration::from_millis(60), 60), Duration::ZERO); // 第 2 帧（目标 base+60）
        q.note_producer_gap(Duration::from_millis(10), 60); // 10ms < 60ms：不锚定
        assert_eq!(
            q.plan(base + Duration::from_millis(60), 60),
            Duration::from_millis(60),
            "短真空不得改变时间表（第 3 帧目标仍为 base+120）"
        );
        // reset 后回到首帧语义
        q.reset();
        assert_eq!(q.plan(base, 60), Duration::ZERO);
    }

    /// 内存传输：预置上行帧序列，供 poll_abort 无网络单测。
    struct SeqTransport {
        frames: Vec<IncomingFrame>,
    }

    impl Transport for SeqTransport {
        async fn send_text(&mut self, _json: String) -> anyhow::Result<()> {
            Ok(())
        }
        async fn send_binary(&mut self, _data: Vec<u8>) -> anyhow::Result<()> {
            Ok(())
        }
        async fn recv(&mut self) -> IncomingFrame {
            if self.frames.is_empty() {
                IncomingFrame::Closed
            } else {
                self.frames.remove(0)
            }
        }
        fn try_recv(&mut self) -> Option<IncomingFrame> {
            (!self.frames.is_empty()).then(|| self.frames.remove(0))
        }
        async fn keepalive(&mut self) {}
    }

    fn transport_with(frames: Vec<IncomingFrame>) -> SeqTransport {
        SeqTransport { frames }
    }

    fn abort_flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[test]
    fn poll_abort_triggers_on_abort_text() {
        let mut t = transport_with(vec![IncomingFrame::Text(
            r#"{"type":"abort"}"#.to_string(),
        )]);
        let flag = abort_flag();
        assert!(poll_abort(&mut t, "s1", &flag));
    }

    #[test]
    fn poll_abort_ignores_non_abort_text_and_binary() {
        let mut t = transport_with(vec![
            IncomingFrame::Text(r#"{"type":"listen","state":"start"}"#.to_string()),
            IncomingFrame::Binary(vec![0u8; 8]),
        ]);
        let flag = abort_flag();
        assert!(!poll_abort(&mut t, "s1", &flag));
    }

    #[test]
    fn poll_abort_treats_closed_as_interrupt() {
        let mut t = transport_with(vec![IncomingFrame::Closed]);
        let flag = abort_flag();
        assert!(poll_abort(&mut t, "s1", &flag));
    }

    #[test]
    fn poll_abort_empty_is_not_interrupt() {
        let mut t = transport_with(vec![]);
        let flag = abort_flag();
        assert!(!poll_abort(&mut t, "s1", &flag));
    }

    #[test]
    fn pad_tail_pads_partial_frame() {
        // 24k/60ms = 1440 样本/帧；60 残帧必须补零到整帧，否则 Opus 编码 BadArgument
        let pcm = vec![0.5f32; 60];
        let tail = pad_tail(&pcm, 1440);
        assert_eq!(tail.len(), 1440, "残帧必须补零到整帧");
        assert_eq!(tail[0], 0.5);
        assert_eq!(tail[59], 0.5);
        assert_eq!(tail[60], 0.0);
        assert_eq!(tail[1439], 0.0);
    }

    #[test]
    fn pad_tail_returns_empty_when_full() {
        // 整帧或更长：无需补零（下行热路径直接 chunks 遍历，不分配）
        let pcm = vec![1.0f32; 2880];
        assert!(pad_tail(&pcm, 1440).is_empty());
        let pcm2 = vec![1.0f32; 1441];
        assert!(pad_tail(&pcm2, 1440).is_empty());
    }

    #[test]
    fn pad_tail_short_input() {
        // 短于一帧：补成一帧
        let pcm = vec![0.25f32; 100];
        let tail = pad_tail(&pcm, 1440);
        assert_eq!(tail.len(), 1440);
        assert_eq!(tail[99], 0.25);
        assert_eq!(tail[100], 0.0);
    }

    #[test]
    fn pad_tail_empty() {
        assert!(pad_tail(&[], 1440).is_empty());
    }
}
