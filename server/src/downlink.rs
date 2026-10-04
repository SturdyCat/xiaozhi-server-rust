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
use crate::audio::resample::resample;
use crate::protocol::{ClientMessage, ServerMessage, wrap_downlink};
use crate::session::{send_binary, send_text, SessionParams};

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

/// 发送单句：`sentence_start(text)` → 重采样/Opus 分帧逐帧二进制（逐帧轮询打断）。
/// 返回实际下发的帧数（供日志统计耗时/流量）。
pub(crate) async fn send_sentence_audio(
    socket: &mut WebSocket,
    params: &SessionParams,
    downlink_ts: &mut u32,
    abort: &Arc<AtomicBool>,
    session_id: &str,
    text: &str,
    pcm: &[f32],
    tts_sr: u32,
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

    let pcm_down = resample(pcm, tts_sr, params.downlink_sr);
    let pcm_down: &[f32] = pcm_down.as_ref();
    let frame_samples =
        (params.downlink_sr as f32 * params.downlink_frame_ms as f32 / 1000.0) as usize;
    // 整段下行共用一个流式编码器：逐帧新建会让每个 60ms 边界都是「流重启」→
    // 实听「声音不连续」（详见 opus.rs 的 OpusFrameEncoder 注释）。
    let mut encoder = OpusFrameEncoder::new(params.downlink_sr)?;
    let mut frames_sent: usize = 0;
    let frame = frame_samples.max(1);
    let n = pcm_down.len();
    let n_full = n / frame;
    for i in 0..n_full {
        let opus = match encoder.encode_frame(&pcm_down[i * frame..(i + 1) * frame]) {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!("Opus 编码失败: {e}");
                break;
            }
        };
        let framed = wrap_downlink(params.downlink_bin_ver, &opus, *downlink_ts);
        *downlink_ts += params.downlink_frame_ms;
        frames_sent += 1;
        send_binary(socket, framed).await?;

        // 每帧后非阻塞轮询 abort / close
        if poll_abort(socket, session_id, abort) {
            tracing::info!("session {} 中断 TTS 播报（{text}）", session_id);
            break;
        }
    }
    // 残帧补零到整帧再编码（Opus 只接受合法帧长），避免最后不足一帧被丢弃。
    if n % frame > 0 && !abort.load(Ordering::Relaxed) {
        let tail = pad_tail(&pcm_down[n_full * frame..], frame);
        match encoder.encode_frame(&tail) {
            Ok(opus) => {
                let framed = wrap_downlink(params.downlink_bin_ver, &opus, *downlink_ts);
                *downlink_ts += params.downlink_frame_ms;
                frames_sent += 1;
                send_binary(socket, framed).await?;
            }
            Err(e) => tracing::warn!("Opus 编码失败（尾帧）: {e}"),
        }
    }
    Ok(frames_sent)
}

/// 测试台整段路径：`start` → 单句全量 → `stop`（网页测试台 `tts_test` 共用）。
pub(crate) async fn send_tts_audio(
    socket: &mut WebSocket,
    params: &SessionParams,
    downlink_ts: &mut u32,
    abort: &Arc<AtomicBool>,
    session_id: &str,
    text: &str,
    pcm: &[f32],
    tts_sr: u32,
) -> Result<usize> {
    send_tts_state(socket, session_id, "start").await?;
    let frames = send_sentence_audio(socket, params, downlink_ts, abort, session_id, text, pcm, tts_sr).await?;
    send_tts_state(socket, session_id, "stop").await?;
    Ok(frames)
}

/// 末段 PCM 不足一整帧时补零到整帧（Opus 只接受合法帧长，否则 BadArgument）。
///
/// 下行热路径请直接 `pcm.chunks(frame)` 遍历整帧（零拷贝），仅对最后一个不足整帧的
/// 切片调用本函数补零后再编码，避免把整段 PCM 物化为 `Vec<Vec<f32>>` 造成整段拷贝。
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
