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
    Ok(())
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
            send_tts_audio(socket, engines, params, downlink_ts, session_id, &text, &pcm, tts_sr).await?;
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
    send_tts_audio(socket, engines, params, downlink_ts, session_id, &reply, &pcm, tts_sr).await?;
    Ok(())
}

/// 发送完整 TTS 下行序列：tts start → sentence_start → 重采样/Opus 分帧逐帧二进制 → tts stop
/// （支持 abort 中断）。设备流水线与网页测试台 `tts_test` 共用。
#[allow(clippy::too_many_arguments)]
async fn send_tts_audio(
    socket: &mut WebSocket,
    engines: &Arc<Engines>,
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
    for chunk in pcm_down.chunks(frame_samples.max(1)) {
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

async fn send_binary(socket: &mut WebSocket, data: Vec<u8>) -> anyhow::Result<()> {
    socket
        .send(Message::Binary(data.into()))
        .await
        .map_err(|e| anyhow::anyhow!("发送二进制失败: {e}"))?;
    Ok(())
}
