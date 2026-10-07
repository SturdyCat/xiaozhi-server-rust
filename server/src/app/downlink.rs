//! 下行音频发送：把合成后的 PCM 重采样、Opus 分帧，逐帧经承载（[`crate::app::transport::Transport`]）
//! 下发设备，并在下行间隙轮询打断（barge-in）。
//!
//! 与 [`crate::app::session::Session`] 解耦为自由函数，仅依赖传入的传输实现 / 协商参数 /
//! 下行时间戳 / 打断标志——不持有会话可变状态，便于独立阅读与测试
//! （打断轮询可用内存 [`crate::app::transport::IncomingFrame`] 序列单测，无需真实连接）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;

use crate::app::audio::opus::OpusFrameEncoder;
use crate::app::audio::resample::StreamingResampler;
use crate::app::protocol::{ClientMessage, ServerMessage, wrap_downlink};
use crate::app::session::{send_binary, send_text, SessionParams};
use crate::app::transport::{IncomingFrame, Transport};
use tokio::sync::mpsc;

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
    let mut resampler = StreamingResampler::new(tts_sr, params.downlink_sr);
    let mut encoder = OpusFrameEncoder::new(params.downlink_sr)?;
    let mut carry: Vec<f32> = Vec::new(); // 不足整帧的尾部（跨分片积攒）
    let mut frames_sent: usize = 0;

    // 收分片 → 重采样 → 攒满整帧即编码下发
    loop {
        let chunk = rx.recv().await;
        let Some(chunk) = chunk else { break };
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
    use crate::app::transport::{IncomingFrame, Transport};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

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
