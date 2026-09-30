//! 每连接会话状态机与语音流水线。
//!
//! 两条路径：
//! - 真实（`sherpa` feature 且 backend=sherpa）：上行 Opus → 解码 → VAD → ASR → stt → LLM → tts → 下行 Opus。
//! - mock（默认）：收到 `listen start` 后直接触发 mock ASR → LLM → mock TTS，验证协议回包。

use std::sync::Arc;

use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures_util::{FutureExt, StreamExt};
use uuid::Uuid;

use crate::audio::opus::{decode_opus_frame, encode_opus_frame};
use crate::audio::resample::resample;
use crate::engine::Engines;
use crate::protocol::{BinVersion, ClientMessage, ServerMessage, unwrap_uplink, wrap_downlink};
use crate::vad::VadEngine;

/// 会话协商参数（握手后固定）。
#[derive(Clone, Copy)]
pub struct SessionParams {
    pub uplink_bin_ver: BinVersion,
    pub downlink_bin_ver: BinVersion,
    pub uplink_sr: u32,
    pub downlink_sr: u32,
    pub downlink_frame_ms: u32,
}

/// 会话主循环：收发消息、驱动流水线。
pub async fn run_session(
    mut socket: WebSocket,
    engines: Arc<Engines>,
    params: SessionParams,
) -> anyhow::Result<()> {
    let mut vad = engines.new_vad();
    let mut history: Vec<(String, String)> = Vec::new();
    let mut downlink_ts: u32 = 0;
    let session_id = Uuid::new_v4().to_string();
    let real_audio =
        cfg!(feature = "sherpa") && !engines.config.asr_is_mock() && !engines.config.tts_is_mock();
    // 网页测试台：录音中标志与 PCM 缓冲（跳过 VAD，整段一次性识别）。
    let mut test_recording = false;
    let mut test_buf: Vec<f32> = Vec::new();

    while let Some(item) = socket.next().await {
        let msg = match item {
            Ok(m) => m,
            Err(_) => break,
        };
        match msg {
            Message::Text(t) => {
                let s = t.to_string();
                if let Err(e) = handle_text(
                    &mut socket,
                    &s,
                    &engines,
                    &mut history,
                    &params,
                    &mut downlink_ts,
                    &session_id,
                    &mut test_recording,
                    &mut test_buf,
                )
                .await
                {
                    tracing::warn!("处理文本消息失败: {e}");
                }
            }
            Message::Binary(b) => {
                let data: &[u8] = b.as_ref();
                handle_binary(
                    &mut socket,
                    data,
                    &engines,
                    &mut vad,
                    &mut history,
                    &params,
                    &mut downlink_ts,
                    &session_id,
                    &mut test_recording,
                    &mut test_buf,
                    real_audio,
                )
                .await;
                // mock 模式且未在测试录音时忽略上行音频（由 listen start 触发模拟流水）
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    // 会话结束：flush VAD 残留语音段（真实音频模式），尽力把最后一句话送完流水线。
    // 连接可能已关闭，此时发送会失败并被记录，属正常情况。
    if real_audio {
        let mut segments = Vec::new();
        vad.flush(&mut |seg| segments.push(seg));
        recognize_segments(
            &mut socket,
            &engines,
            segments,
            &mut history,
            &params,
            &mut downlink_ts,
            &session_id,
        )
        .await;
    }
    Ok(())
}

/// 将 VAD 切出的语音段送 ASR → 完整流水线；常规分帧与会话结束 flush 共用。
async fn recognize_segments(
    socket: &mut WebSocket,
    engines: &Arc<Engines>,
    segments: Vec<Vec<f32>>,
    history: &mut Vec<(String, String)>,
    params: &SessionParams,
    downlink_ts: &mut u32,
    session_id: &str,
) {
    for seg in segments {
        let Ok(user_text) = engines.asr.recognize(&seg, params.uplink_sr) else {
            continue;
        };
        if user_text.trim().is_empty() {
            continue;
        }
        if let Err(e) = stream_response(
            socket,
            engines,
            history,
            params,
            downlink_ts,
            session_id,
            &user_text,
        )
        .await
        {
            tracing::warn!("流水处理失败: {e}");
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_text(
    socket: &mut WebSocket,
    text: &str,
    engines: &Arc<Engines>,
    history: &mut Vec<(String, String)>,
    params: &SessionParams,
    downlink_ts: &mut u32,
    session_id: &str,
    test_recording: &mut bool,
    test_buf: &mut Vec<f32>,
) -> anyhow::Result<()> {
    let msg: ClientMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("解析客户端消息失败: {e}");
            return Ok(());
        }
    };
    match msg {
        ClientMessage::Listen { state, .. } if state == "start" => {
            let real_audio = cfg!(feature = "sherpa")
                && !engines.config.asr_is_mock()
                && !engines.config.tts_is_mock();
            if real_audio {
                tracing::info!("session {session_id} 进入 listening（真实音频模式）");
            } else {
                let user_text = match engines.asr.recognize(&[], params.uplink_sr) {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!("mock ASR 失败: {e}");
                        return Ok(());
                    }
                };
                stream_response(socket, engines, history, params, downlink_ts, session_id, &user_text)
                    .await?;
            }
        }
        ClientMessage::Listen { state, .. } => {
            tracing::info!("listen state={state}");
        }
        ClientMessage::Abort { .. } => {
            tracing::info!("session {session_id} 收到 abort");
        }
        ClientMessage::Mcp { payload, .. } => {
            send_text(
                socket,
                &ServerMessage::Mcp {
                    session_id: Some(session_id.to_string()),
                    payload,
                },
            )
            .await?;
        }
        ClientMessage::AsrTest { action, .. } => match action.as_str() {
            "start" => {
                test_buf.clear();
                *test_recording = true;
                tracing::info!("session {session_id} 测试录音开始");
            }
            "stop" => {
                *test_recording = false;
                let buf = std::mem::take(test_buf);
                let secs = buf.len() as f32 / params.uplink_sr as f32;
                tracing::info!("session {session_id} 测试录音结束，{secs:.1}s，开始识别");
                let text = if buf.is_empty() {
                    "(未录制到音频)".to_string()
                } else {
                    match engines.asr.recognize(&buf, params.uplink_sr) {
                        Ok(t) if t.trim().is_empty() => "(识别结果为空，请重试)".to_string(),
                        Ok(t) => t,
                        Err(e) => format!("识别失败: {e}"),
                    }
                };
                send_text(
                    socket,
                    &ServerMessage::Stt {
                        session_id: session_id.to_string(),
                        text,
                    },
                )
                .await?;
            }
            _ => tracing::warn!("未知 asr_test action: {action}"),
        },
        ClientMessage::TtsTest {
            text,
            speaker,
            lang,
            speed,
            ..
        } => {
            if text.trim().is_empty() {
                return Ok(());
            }
            let lang = lang
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| engines.config.tts.lang.clone());
            let speaker = speaker.unwrap_or(engines.config.tts.speaker);
            let speed = speed.unwrap_or(engines.config.tts.speed);
            tracing::info!("session {session_id} 测试合成：lang={lang} speaker={speaker} speed={speed}");
            let tts = engines.tts_for(&lang);
            let (pcm, tts_sr) = match tts.synthesize(&text, speed, speaker) {
                Ok(x) => x,
                Err(e) => {
                    tracing::warn!("测试合成失败: {e}");
                    return Ok(());
                }
            };
            send_tts_audio(socket, params, downlink_ts, session_id, &text, &pcm, tts_sr).await?;
        }
        ClientMessage::Hello(_) => {
            tracing::warn!("会话内收到重复 hello，忽略");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_binary(
    socket: &mut WebSocket,
    data: &[u8],
    engines: &Arc<Engines>,
    vad: &mut Box<dyn VadEngine>,
    history: &mut Vec<(String, String)>,
    params: &SessionParams,
    downlink_ts: &mut u32,
    session_id: &str,
    test_recording: &mut bool,
    test_buf: &mut Vec<f32>,
    real_audio: bool,
) {
    let payload = unwrap_uplink(params.uplink_bin_ver, data);
    if payload.is_empty() {
        return;
    }
    let Ok(pcm) = decode_opus_frame(payload, params.uplink_sr) else {
        return;
    };
    // 网页测试台录音中：直接缓冲整段 PCM，跳过 VAD（mock 后端也可用）。
    if *test_recording {
        test_buf.extend_from_slice(&pcm);
        return;
    }
    if !real_audio {
        return;
    }
    let mut segments = Vec::new();
    vad.accept(&pcm, &mut |seg| segments.push(seg));
    recognize_segments(
        socket,
        engines,
        segments,
        history,
        params,
        downlink_ts,
        session_id,
    )
    .await;
}

/// 从用户文本开始：stt → LLM → llm → TTS → 下行音频（支持 abort 中断）。
async fn stream_response(
    socket: &mut WebSocket,
    engines: &Arc<Engines>,
    history: &mut Vec<(String, String)>,
    params: &SessionParams,
    downlink_ts: &mut u32,
    session_id: &str,
    user_text: &str,
) -> anyhow::Result<()> {
    send_text(
        socket,
        &ServerMessage::Stt {
            session_id: session_id.to_string(),
            text: user_text.to_string(),
        },
    )
    .await?;

    let reply = match engines.llm.chat(history, user_text).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("LLM 调用失败: {e}");
            return Ok(());
        }
    };
    send_text(
        socket,
        &ServerMessage::Llm {
            session_id: session_id.to_string(),
            emotion: Some("happy".to_string()),
            text: None,
        },
    )
    .await?;
    history.push(("user".to_string(), user_text.to_string()));
    history.push(("assistant".to_string(), reply.clone()));
    // 按 [llm].max_history 截断多轮历史，只保留最近 N 条（role, content）。
    let max_history = engines.config.llm.max_history;
    while history.len() > max_history {
        history.remove(0);
    }

    let (pcm, tts_sr) = match engines
        .tts()
        .synthesize(&reply, engines.config.tts.speed, engines.config.tts.speaker)
    {
        Ok(x) => x,
        Err(e) => {
            tracing::warn!("TTS 失败: {e}");
            return Ok(());
        }
    };
    send_tts_audio(socket, params, downlink_ts, session_id, &reply, &pcm, tts_sr).await?;
    Ok(())
}

/// 发送完整 TTS 下行序列：tts start → sentence_start → 重采样/Opus 分帧逐帧二进制 → tts stop
/// （支持 abort 中断）。设备流水线与网页测试台 `tts_test` 共用。
#[allow(clippy::too_many_arguments)]
async fn send_tts_audio(
    socket: &mut WebSocket,
    params: &SessionParams,
    downlink_ts: &mut u32,
    session_id: &str,
    text: &str,
    pcm: &[f32],
    tts_sr: u32,
) -> anyhow::Result<()> {
    send_text(
        socket,
        &ServerMessage::Tts {
            session_id: session_id.to_string(),
            state: "start".to_string(),
            text: None,
        },
    )
    .await?;
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
    let frame_samples =
        (params.downlink_sr as f32 * params.downlink_frame_ms as f32 / 1000.0) as usize;
    for chunk in frame_chunks(&pcm_down, frame_samples.max(1)) {
        let opus = match encode_opus_frame(chunk, params.downlink_sr) {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!("Opus 编码失败: {e}");
                break;
            }
        };
        let framed = wrap_downlink(params.downlink_bin_ver, &opus, *downlink_ts);
        *downlink_ts += params.downlink_frame_ms;
        send_binary(socket, framed).await?;

        // 每帧后非阻塞检查 abort / close
        if let Some(Some(Ok(m))) = socket.next().now_or_never() {
            match m {
                Message::Close(_) => return Ok(()),
                Message::Text(t) => {
                    let s = t.to_string();
                    if let Ok(ClientMessage::Abort { .. }) = serde_json::from_str(&s) {
                        tracing::info!("session {session_id} 中断 TTS 播报（{text}）");
                        break;
                    }
                }
                _ => {}
            }
        }
    }

    send_text(
        socket,
        &ServerMessage::Tts {
            session_id: session_id.to_string(),
            state: "stop".to_string(),
            text: None,
        },
    )
    .await?;
    Ok(())
}

async fn send_text(socket: &mut WebSocket, msg: &ServerMessage) -> anyhow::Result<()> {
    socket
        .send(Message::Text(Utf8Bytes::from(msg.to_json())))
        .await
        .map_err(|e| anyhow::anyhow!("发送文本失败: {e}"))?;
    Ok(())
}

/// 把下行 PCM 切成固定帧长；末尾不足一帧补 0（静音）到整帧。
///
/// Opus 只接受合法帧长（2.5/5/10/20/40/60ms）。TTS 音频总样本数几乎不可能整除帧长，
/// 残帧直接编码会返回 BadArgument（日志 "Opus 编码失败: Opus(BadArgument)"），
/// 整段语音最后不足一帧被丢弃——补零后音频完整、听感无差异。
fn frame_chunks(pcm: &[f32], frame: usize) -> Vec<Vec<f32>> {
    let frame = frame.max(1);
    let mut out: Vec<Vec<f32>> = pcm.chunks(frame).map(|c| c.to_vec()).collect();
    if let Some(last) = out.last_mut() {
        if last.len() < frame {
            last.resize(frame, 0.0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::frame_chunks;

    #[test]
    fn frame_chunks_pads_partial_tail() {
        // 24k/60ms = 1440 样本；1500 样本 = 1 整帧 + 60 残帧
        let pcm = vec![0.5f32; 1500];
        let frames = frame_chunks(&pcm, 1440);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].len(), 1440);
        assert_eq!(frames[1].len(), 1440, "残帧必须补零到整帧，否则 Opus 编码 BadArgument");
        // 残帧内容：前 60 个样本保留原值，其余补 0
        assert_eq!(frames[1][0], 0.5);
        assert_eq!(frames[1][59], 0.5);
        assert_eq!(frames[1][60], 0.0);
        assert_eq!(frames[1][1439], 0.0);
    }

    #[test]
    fn frame_chunks_exact_multiple_untouched() {
        let pcm = vec![1.0f32; 2880];
        let frames = frame_chunks(&pcm, 1440);
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|f| f.len() == 1440));
    }

    #[test]
    fn frame_chunks_single_partial() {
        // 短于一帧：补成一帧（原先直接编码会 BadArgument）
        let pcm = vec![0.25f32; 100];
        let frames = frame_chunks(&pcm, 1440);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), 1440);
        assert_eq!(frames[0][99], 0.25);
        assert_eq!(frames[0][100], 0.0);
    }

    #[test]
    fn frame_chunks_empty() {
        assert!(frame_chunks(&[], 1440).is_empty());
    }
}

async fn send_binary(socket: &mut WebSocket, data: Vec<u8>) -> anyhow::Result<()> {
    socket
        .send(Message::Binary(data.into()))
        .await
        .map_err(|e| anyhow::anyhow!("发送二进制失败: {e}"))?;
    Ok(())
}
