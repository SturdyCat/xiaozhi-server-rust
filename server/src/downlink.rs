//! 下行音频发送：把合成后的 PCM 重采样、Opus 分帧，逐帧经 WebSocket 下发设备，
//! 并在下行间隙轮询打断（barge-in）。
//!
//! 与 [`crate::session::Session`] 解耦为自由函数，仅依赖传入的 socket / 协商参数 /
//! 下行时间戳 / 打断标志——不持有会话可变状态，便于独立阅读与测试。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use axum::extract::ws::{Message, WebSocket};
use futures_util::{FutureExt, StreamExt};

use crate::audio::opus::OpusFrameEncoder;
use crate::audio::resample::StreamingResampler;
use crate::protocol::{ClientMessage, ServerMessage, wrap_downlink};
use crate::session::{send_binary, send_text, SessionParams};
use tokio::sync::mpsc;

/// 非阻塞抽取 socket 中已到达的打断/关闭消息（流水线各等待点轮询）。
///
/// 返回当前打断标志（收到 abort、连接断开或已被置位均视为打断）。
/// 注意：播报期间到达的上行二进制音频会被丢弃——打断以 abort 文本消息为准
/// （ESP barge-in 时设备先发 abort 再重新 listen）。
pub(crate) fn poll_abort(socket: &mut WebSocket, session_id: &str, abort: &Arc<AtomicBool>) -> bool {
    loop {
        match socket.next().now_or_never() {
            None => break, // 暂无待处理消息
            Some(None) => {
                tracing::debug!("session {} socket 已关闭（视为打断）", session_id);
                abort.store(true, Ordering::Relaxed);
                break;
            }
            Some(Some(Ok(m))) => {
                if let Message::Text(t) = &m {
                    if matches!(
                        serde_json::from_str::<ClientMessage>(t),
                        Ok(ClientMessage::Abort { .. })
                    ) {
                        tracing::info!("session {} 收到 abort（流水线终止）", session_id);
                        abort.store(true, Ordering::Relaxed);
                        break;
                    }
                }
                // 其余消息（上行音频/listen 等）在播报期丢弃
            }
            Some(Some(Err(e))) => {
                tracing::debug!("session {} socket 读取错误（视为断开）: {e}", session_id);
                abort.store(true, Ordering::Relaxed);
                break;
            }
        }
    }
    abort.load(Ordering::Relaxed)
}

/// 发送 `tts` 状态消息（`start`/`stop`/`sentence_start` 等）。
pub(crate) async fn send_tts_state(
    socket: &mut WebSocket,
    session_id: &str,
    state: &str,
) -> Result<()> {
    send_text(
        socket,
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
pub(crate) async fn send_sentence_audio_stream(
    socket: &mut WebSocket,
    params: &SessionParams,
    downlink_ts: &mut u32,
    abort: &Arc<AtomicBool>,
    session_id: &str,
    text: &str,
    tts_sr: u32,
    mut rx: mpsc::UnboundedReceiver<Vec<f32>>,
) -> Result<usize> {
    send_text(
        socket,
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
            send_binary(socket, framed).await?;
            if poll_abort(socket, session_id, abort) {
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
                send_binary(socket, framed).await?;
            }
            Err(e) => tracing::warn!("Opus 编码失败（流式尾帧）: {e}"),
        }
    }
    Ok(frames_sent)
}

#[cfg(test)]
mod tests {
    use super::pad_tail;

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
