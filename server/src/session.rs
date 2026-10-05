//! 每连接会话状态机与语音流水线。
//!
//! 两条路径（由 hello 的 `test` 参数区分）：
//! - ESP 设备（正式流程）：上行 Opus → 解码 → VAD → ASR → stt → LLM → tts → 下行 Opus。
//! - macApp 测试台（hello 带 `test:true`）：`asr_test`/`tts_test`/`llm_test` 三种
//!   **独立服务请求-响应**（各自直调对应真实引擎、各自回包），不进设备流水线；
//!   设备会话收到这三类消息一律忽略。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures_util::StreamExt;

use crate::audio::opus::OpusFrameDecoder;
use crate::asr::AsrEngine;
use crate::config::Config;
use crate::downlink::{poll_abort, send_sentence_audio_stream, send_tts_state};
use crate::engine::Engines;
use crate::llm::{LlmEvent, build_llm};
use crate::protocol::{BinVersion, ClientMessage, ServerMessage, unwrap_uplink};
use crate::splitter::SentenceSplitter;
use crate::vad::VadEngine;

/// 在 `spawn_blocking` 中执行同步阻塞的 CPU 密集调用（ASR/TTS 推理），
/// 统一处理任务取消/panic 与内部错误，返回 `Err(String)`（含可读错误信息）。
/// 调用方据此决定降级行为（warn 后 continue 或 return）。
async fn blocking_result<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(format!("推理失败: {e:#}")),
        Err(e) => Err(format!("推理任务异常（可能 panic）: {e}")),
    }
}

/// 会话协商参数（握手后固定）。
#[derive(Clone, Copy)]
pub struct SessionParams {
    pub uplink_bin_ver: BinVersion,
    pub downlink_bin_ver: BinVersion,
    pub uplink_sr: u32,
    pub downlink_sr: u32,
    pub downlink_frame_ms: u32,
}

/// 单连接会话：承载 WebSocket、引擎引用与全部可变会话状态。
///
/// 流水线函数（`handle_text`/`handle_binary`/`stream_response`/`send_tts_audio`/
/// `recognize_segments`）原本要传 7~11 个离散参数（含多个可变状态），增删一个状态
/// 就要改 N 处签名。聚合成 `Session` 后统一用 `&mut self` 访问，消除参数膨胀与漏传风险。
pub struct Session<'a> {
    socket: &'a mut WebSocket,
    engines: Arc<Engines>,
    params: SessionParams,
    session_id: String,
    history: Vec<(String, String)>,
    downlink_ts: u32,
    test_recording: bool,
    test_buf: Vec<f32>,
    vad: Box<dyn VadEngine>,
    uplink_decoder: OpusFrameDecoder,
    real_audio: bool,
    /// hello.test=true 的测试台会话：唯一能发起 asr_test/tts_test/llm_test 的连接。
    is_test: bool,
    /// 打断标志（barge-in）：`handle_text` 收 abort、或流水线轮询到 socket 中 abort/断开时置位。
    /// 生产者（LLM 读流）与消费者（TTS 合成/下行）各阶段轮询，一轮对话结束重置。
    abort: Arc<AtomicBool>,
}

impl<'a> Session<'a> {
    /// 主循环：接收消息并分发；循环结束后 flush VAD 残留段并送流水线。
    async fn run(mut self) -> anyhow::Result<()> {
        while let Some(item) = self.socket.next().await {
            let msg = match item {
                Ok(m) => m,
                Err(e) => {
                    tracing::debug!("socket 读取结束/出错（会话结束）: {e}");
                    break;
                }
            };
            match msg {
                Message::Text(t) => {
                    let s = t.to_string();
                    if let Err(e) = self.handle_text(&s).await {
                        tracing::warn!("处理文本消息失败: {e}");
                    }
                }
                Message::Binary(b) => {
                    self.handle_binary(b.as_ref()).await;
                }
                Message::Close(_) => break,
                _ => {}
            }
        }

        // 会话结束：flush VAD 残留语音段（真实音频模式），尽力把最后一句话送完流水线。
        // 连接可能已关闭，此时发送会失败并被记录，属正常情况。
        if self.real_audio {
            let mut segments = Vec::new();
            self.vad.flush(&mut |seg| segments.push(seg));
            self.recognize_segments(segments).await;
        }
        Ok(())
    }

    /// 在 `spawn_blocking` 中执行同步阻塞的 ASR 识别，统一处理任务取消/panic 与内部错误。
    /// `recognize_segments` 与 `AsrTest` 共用，避免重复 clone 引擎与计时样板。
    /// 写成不借用 `&self` 的关联函数：避免捕获 `Session`（含 `&mut WebSocket`，非 `Sync`）
    /// 导致返回的 future 不满足 `Send`，破坏 `ws.rs` 的 `on_upgrade` 要求。
    async fn asr_recognize(
        asr: Arc<dyn AsrEngine>,
        sr: u32,
        pcm: Vec<f32>,
    ) -> Result<String, String> {
        blocking_result(move || asr.recognize(&pcm, sr)).await
    }

    /// 将 VAD 切出的语音段送 ASR → 完整流水线；常规分帧与会话结束 flush 共用。
    async fn recognize_segments(&mut self, segments: Vec<Vec<f32>>) {
        let session_id = self.session_id.clone();
        for seg in segments {
            let t0 = std::time::Instant::now();
            let user_text = match Self::asr_recognize(self.engines.asr.clone(), self.params.uplink_sr, seg).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("ASR 识别失败: {e}");
                    continue;
                }
            };
            if user_text.trim().is_empty() {
                continue;
            }
            tracing::info!(
                "session {session_id} 识别完成：耗时 {}ms（{} 字）",
                t0.elapsed().as_millis(),
                user_text.chars().count()
            );
            if let Err(e) = self.stream_response(&user_text).await {
                tracing::warn!("流水处理失败: {e}");
            }
        }
    }

    async fn handle_text(&mut self, text: &str) -> anyhow::Result<()> {
        let session_id = self.session_id.clone();
        let msg: ClientMessage = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("解析客户端消息失败: {e}");
                return Ok(());
            }
        };
        match msg {
            ClientMessage::Listen { state, .. } if state == "start" => {
                // ESP 正式流程：listening 状态由设备侧 VAD 上报触发，语音段经二进制帧上行，
                // 服务端 VAD 切句后走 ASR → LLM → TTS 流水线（见 handle_binary）。
                tracing::info!("session {session_id} 进入 listening");
            }
            ClientMessage::Listen { state, .. } => {
                tracing::info!("listen state={state}");
            }
            ClientMessage::Abort { .. } => {
                tracing::info!("session {session_id} 收到 abort（置位打断标志）");
                self.abort.store(true, Ordering::Relaxed);
            }
            ClientMessage::Mcp { payload, .. } => {
                send_text(
                    &mut self.socket,
                    &ServerMessage::Mcp {
                        session_id: Some(session_id),
                        payload,
                    },
                )
                .await?;
            }
            ClientMessage::AsrTest { action, .. } => {
                // 测试台专用端点：仅 hello.test=true 的会话受理（设备正式流程不走测试端点）。
                if !self.is_test {
                    tracing::warn!("session {session_id} 非测试会话收到 asr_test，忽略");
                    return Ok(());
                }
                match action.as_str() {
                "start" => {
                    self.test_buf.clear();
                    self.test_recording = true;
                    tracing::info!("session {session_id} 测试录音开始");
                }
                "stop" => {
                    self.test_recording = false;
                    let buf = std::mem::take(&mut self.test_buf);
                    let secs = buf.len() as f32 / self.params.uplink_sr as f32;
                    tracing::info!("session {session_id} 测试录音结束，{secs:.1}s，开始识别");
                    let text = if buf.is_empty() {
                        "(未录制到音频)".to_string()
                } else {
                    let t0 = std::time::Instant::now();
                    let recognized =
                        Self::asr_recognize(self.engines.asr.clone(), self.params.uplink_sr, buf)
                            .await;
                    match recognized {
                            Ok(t) => {
                                tracing::info!(
                                    "session {session_id} 测试识别完成：耗时 {}ms（音频 {secs:.1}s）",
                                    t0.elapsed().as_millis()
                                );
                                if t.trim().is_empty() {
                                    "(识别结果为空，请重试)".to_string()
                                } else {
                                    t
                                }
                            }
                            Err(e) => e,
                        }
                    };
                    send_text(
                        &mut self.socket,
                        &ServerMessage::Stt {
                            session_id,
                            text,
                        },
                    )
                    .await?;
                }
                _ => tracing::warn!("未知 asr_test action: {action}"),
                }
            },
            ClientMessage::TtsTest {
                text,
                speaker,
                lang,
                speed,
                ..
            } => {
                // 测试台专用端点：仅 hello.test=true 的会话受理。
                if !self.is_test {
                    tracing::warn!("session {session_id} 非测试会话收到 tts_test，忽略");
                    return Ok(());
                }
                if text.trim().is_empty() {
                    return Ok(());
                }
                let lang = lang
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| self.engines.config.tts.lang.clone());
                let speaker = speaker.unwrap_or(self.engines.config.tts.speaker);
                let speed = speed.unwrap_or(self.engines.config.tts.speed);
                tracing::info!("session {session_id} 测试合成：lang={lang} speaker={speaker} speed={speed}");
                // 热切换：先按磁盘最新配置刷新 TTS 引擎（管理页保存后无需重启即可测新配置）。
                // 刷新失败（读盘/构建失败）以 tts_test 结果回报，测试台对话框直接可见。
                let engines = self.engines.clone();
                let refresh = tokio::task::spawn_blocking(move || engines.refresh_tts_from_disk())
                    .await
                    .map_err(|e| anyhow::anyhow!("TTS 热切换任务异常: {e}"))?;
                if let Err(msg) = refresh {
                    tracing::warn!("session {session_id} {msg}");
                    send_text(
                        &mut self.socket,
                        &ServerMessage::TtsTestResult {
                            session_id,
                            state: "error".to_string(),
                            engine: Some(self.engines.tts_name().to_string()),
                            text: Some(msg),
                        },
                    )
                    .await?;
                    return Ok(());
                }
                let tts = self.engines.tts_for(&lang);
                let tts_sr = tts.output_sample_rate();
                let engine_name = tts.name().to_string();
                let t0 = std::time::Instant::now();
                // 流式合成：spawn_blocking 跑引擎，分片经 channel 边收边下发（首片即出声）。
                // 失败经 tts_test 结果回报测试台（不再静默）。
                let (tx, chunk_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<f32>>();
                let text_c = text.clone();
                let producer = tokio::task::spawn_blocking(move || {
                    tts.synthesize_stream(
                        &text_c,
                        speed,
                        speaker,
                        Box::new(move |_sr, chunk| {
                            tx.send(chunk.to_vec()).is_ok()
                        }),
                    )
                });
                let frames = send_sentence_audio_stream(
                    &mut self.socket,
                    &self.params,
                    &mut self.downlink_ts,
                    &self.abort,
                    &self.session_id,
                    &text,
                    tts_sr,
                    chunk_rx,
                )
                .await?;
                match producer.await {
                    Ok(Ok(())) => {
                        tracing::info!(
                            "session {session_id} 测试合成完成（{engine_name}）：耗时 {}ms，{} 帧 / {tts_sr}Hz",
                            t0.elapsed().as_millis(),
                            frames
                        );
                        // 成功回报（音频已下发；state=ok, text=耗时）
                        send_text(
                            &mut self.socket,
                            &ServerMessage::TtsTestResult {
                                session_id,
                                state: "ok".to_string(),
                                engine: Some(engine_name),
                                text: Some(format!("{}ms", t0.elapsed().as_millis())),
                            },
                        )
                        .await?;
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("session {session_id} 测试合成失败（耗时 {}ms）: {e}", t0.elapsed().as_millis());
                        send_text(
                            &mut self.socket,
                            &ServerMessage::TtsTestResult {
                                session_id,
                                state: "error".to_string(),
                                engine: Some(engine_name),
                                text: Some(e.to_string()),
                            },
                        )
                        .await?;
                    }
                    Err(e) => {
                        tracing::warn!("session {session_id} 测试合成任务异常: {e}");
                        send_text(
                            &mut self.socket,
                            &ServerMessage::TtsTestResult {
                                session_id,
                                state: "error".to_string(),
                                engine: Some(engine_name),
                                text: Some(format!("合成任务异常: {e}")),
                            },
                        )
                        .await?;
                    }
                }
            }
            ClientMessage::LlmTest { text, .. } => {
                // 测试台专用端点：仅 hello.test=true 的会话受理。
                if !self.is_test {
                    tracing::warn!("session {session_id} 非测试会话收到 llm_test，忽略");
                    return Ok(());
                }
                if text.trim().is_empty() {
                    return Ok(());
                }
                // 测试台 LLM 连通性验证（独立服务请求-响应，不进设备流水线、不读写会话历史）：
                // 按**磁盘上最新配置**临时构建 LLM 客户端——引擎在启动时构建（engine.rs），
                // 改完配置无需重启即可验证刚保存的 api_base/api_key/model；未指定配置文件
                // （内置默认）或读盘失败时退回启动时的内存配置。
                let llm_cfg = match &self.engines.config_path {
                    Some(p) => Config::load(p).map(|c| c.llm).unwrap_or_else(|e| {
                        tracing::warn!("session {session_id} LLM 测试读配置失败（用启动配置）: {e}");
                        self.engines.config.llm.clone()
                    }),
                    None => self.engines.config.llm.clone(),
                };
                tracing::info!(
                    "session {session_id} LLM 测试：model={} api_base={}",
                    llm_cfg.model,
                    llm_cfg.api_base
                );
                let llm = build_llm(&llm_cfg);
                let t0 = std::time::Instant::now();
                let result = llm.chat_stream(&[], &text, None, |_| true).await;
                let elapsed = t0.elapsed().as_millis() as u64;
                let (state, reply) = match result {
                    Ok(r) => ("ok", r.text),
                    Err(e) => {
                        tracing::warn!("session {session_id} LLM 测试失败（耗时 {elapsed}ms）: {e:#}");
                        ("error", format!("{e:#}"))
                    }
                };
                tracing::info!(
                    "session {session_id} LLM 测试完成：{state}，耗时 {elapsed}ms（{} 字）",
                    reply.chars().count()
                );
                send_text(
                    &mut self.socket,
                    &ServerMessage::LlmTestResult {
                        session_id,
                        state: state.to_string(),
                        text: Some(reply),
                        elapsed_ms: Some(elapsed),
                    },
                )
                .await?;
            }
            ClientMessage::Hello(_) => {
                tracing::warn!("会话内收到重复 hello，忽略");
            }
        }
        Ok(())
    }

    async fn handle_binary(&mut self, data: &[u8]) {
        let payload = unwrap_uplink(self.params.uplink_bin_ver, data);
        if payload.is_empty() {
            return;
        }
        // 会话级解码器连续解码（帧间状态保留；逐包新建会在帧边界产生 PCM 断层）
        let Ok(pcm) = self.uplink_decoder.decode_frame(payload) else {
            return;
        };
        // 测试台录音中（hello.test=true 的会话）：直接缓冲整段 PCM，跳过 VAD。
        if self.test_recording {
            self.test_buf.extend_from_slice(&pcm);
            return;
        }
        if !self.real_audio {
            return;
        }
        let mut segments = Vec::new();
        self.vad.accept(&pcm, &mut |seg| segments.push(seg));
        self.recognize_segments(segments).await;
    }

    /// 从用户文本开始：stt → LLM 流式按句切分 → 逐句 TTS + 逐帧下行（流水线重叠）。
    ///
    /// 生产者 task 读 LLM SSE 流并按句切分推入 channel；消费者（本 task）逐句
    /// `spawn_blocking` 合成 + 下发，读流与合成/下发**重叠**——首字音频延迟（TTFA）
    /// 从「整段串行」降到「首句」。abort 贯穿全链路：LLM 读流回调、合成间隙、
    /// 逐帧下行均轮询打断标志，置位即停。
    async fn stream_response(&mut self, user_text: &str) -> anyhow::Result<()> {
        let session_id = self.session_id.clone();
        self.abort.store(false, Ordering::Relaxed); // 新一轮对话重置打断标志
        send_text(
            &mut self.socket,
            &ServerMessage::Stt {
                session_id: session_id.clone(),
                text: user_text.to_string(),
            },
        )
        .await?;
        // 早发 llm 状态：刷新设备 last_incoming_time_，避免 LLM 首字等待期被 ESP IsTimeout() 断连。
        // （协议约定 llm 消息仅做表情同步，回复正文走 tts.sentence_start。）
        send_text(
            &mut self.socket,
            &ServerMessage::Llm {
                session_id: session_id.clone(),
                emotion: Some("neutral".to_string()),
                text: None,
            },
        )
        .await?;

        let t0 = std::time::Instant::now();
        // 生产者：读 LLM 流 → 按句切分 → channel。
        // unbounded：文本量级小（KB），无需反压；闭包借用 splitter/tx，流结束后仍可 flush。
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let producer = {
            let engines = self.engines.clone();
            let abort = self.abort.clone();
            let history = self.history.clone();
            let user = user_text.to_string();
            tokio::spawn(async move {
                let mut splitter = SentenceSplitter::new();
                let result = engines
                    .llm
                    .chat_stream(&history, &user, None, |event| {
                        if abort.load(Ordering::Relaxed) {
                            return false; // 设备打断：停止读流
                        }
                        if let LlmEvent::Text(delta) = event {
                            for s in splitter.feed(&delta) {
                                let _ = tx.send(s);
                            }
                        }
                        true
                    })
                    .await;
                for s in splitter.flush() {
                    let _ = tx.send(s);
                }
                result
            })
        };

        // 消费者：tts start → 逐句（sentence_start + spawn_blocking TTS + 逐帧下行）→ tts stop
        let mut tts_started = false;
        let mut interrupted = false;
        let mut sentences_done = 0usize;
        let mut last_ping = std::time::Instant::now();
        loop {
            // 200ms 粒度等待下一句：等待间隙轮询打断 + 定期 ping 保活
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Some(sentence)) => {
                    if poll_abort(&mut self.socket, &self.session_id, &self.abort) {
                        interrupted = true;
                        break;
                    }
                    if !tts_started {
                        send_tts_state(&mut self.socket, &self.session_id, "start").await?;
                        tts_started = true;
                    }
                    // 流式合成：spawn_blocking 里跑引擎（同步阻塞），分片经 channel 回流；
                    // 本 task 边收边重采样/编码/下发——首片即发声（TTFA 从「整句合成完」
                    // 提前到「首片合成完」）。abort 时回调返回 false，引擎尽早停止合成。
                    let tts = self.engines.tts();
                    let tts_sr = tts.output_sample_rate();
                    let speed = self.engines.config.tts.speed;
                    let speaker = self.engines.config.tts.speaker;
                    let t_tts = std::time::Instant::now();
                    let s = sentence.clone();
                    let (tx, chunk_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<f32>>();
                    let producer = tokio::task::spawn_blocking(move || {
                        tts.synthesize_stream(
                            &s,
                            speed,
                            speaker,
                            Box::new(move |_sr, chunk| tx.send(chunk.to_vec()).is_ok()),
                        )
                    });
                    let frames = send_sentence_audio_stream(
                        &mut self.socket,
                        &self.params,
                        &mut self.downlink_ts,
                        &self.abort,
                        &self.session_id,
                        &sentence,
                        tts_sr,
                        chunk_rx,
                    )
                    .await?;
                    match producer.await {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => tracing::warn!(
                            "session {session_id} 句子合成失败（耗时 {}ms）: {e}",
                            t_tts.elapsed().as_millis()
                        ),
                        Err(e) => tracing::warn!("session {session_id} 合成任务异常: {e}"),
                    }
                    sentences_done += 1;
                    if sentences_done == 1 {
                        tracing::info!(
                            "session {session_id} 首句下发（TTFA≈{}ms，{frames} 帧）",
                            t0.elapsed().as_millis()
                        );
                    }
                    if poll_abort(&mut self.socket, &self.session_id, &self.abort) {
                        interrupted = true;
                        break;
                    }
                }
                Ok(None) => break, // 生产者结束（tx 已 drop）
                Err(_elapsed) => {
                    // 等待超时：轮询打断 + 长静默期 Ping 保活（tungstenite 自动回 Pong）
                    if poll_abort(&mut self.socket, &self.session_id, &self.abort) {
                        interrupted = true;
                        break;
                    }
                    if last_ping.elapsed() >= Duration::from_secs(10) {
                        last_ping = std::time::Instant::now();
                        let _ = self.socket.send(Message::Ping("ping".into())).await;
                    }
                }
            }
        }
        if tts_started {
            // 打断时也发 stop：设备据此立即停止播放并清理队列
            send_tts_state(&mut self.socket, &self.session_id, "stop").await?;
        }

        // 汇总 LLM 结果；仅未打断且流正常结束时压入历史，避免脏历史
        let llm_out = match producer.await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                tracing::warn!(
                    "session {session_id} LLM 调用失败（耗时 {}ms）: {e}",
                    t0.elapsed().as_millis()
                );
                return Ok(());
            }
            Err(e) => {
                tracing::warn!("session {session_id} LLM 任务异常: {e}");
                return Ok(());
            }
        };
        tracing::info!(
            "session {session_id} 本轮完成：LLM 流 {}ms / {} 字 / {} 个工具调用，下发 {sentences_done} 句{}",
            t0.elapsed().as_millis(),
            llm_out.text.chars().count(),
            llm_out.tool_calls.len(),
            if interrupted { "（被打断）" } else { "" }
        );
        if !interrupted && !llm_out.text.trim().is_empty() {
            self.history.push(("user".to_string(), user_text.to_string()));
            self.history
                .push(("assistant".to_string(), llm_out.text.clone()));
            // 按 [llm].max_history 截断多轮历史，只保留最近 N 条（role, content）。
            let max_history = self.engines.config.llm.max_history;
            while self.history.len() > max_history {
                self.history.remove(0);
            }
        }
        Ok(())
    }
}

/// 会话主循环：收发消息、驱动流水线。`is_test` 来自 hello 的 `test` 参数，
/// 只有测试台会话能发起 asr_test/tts_test/llm_test（设备正式流程不经过测试端点）。
pub async fn run_session(
    mut socket: WebSocket,
    engines: Arc<Engines>,
    params: SessionParams,
    session_id: String,
    is_test: bool,
) -> anyhow::Result<()> {
    let vad = engines.new_vad()?;
    let uplink_decoder = OpusFrameDecoder::new(params.uplink_sr)?;
    let real_audio = engines.config.real_audio();
    let session = Session {
        socket: &mut socket,
        engines,
        params,
        session_id,
        history: Vec::new(),
        downlink_ts: 0,
        test_recording: false,
        test_buf: Vec::new(),
        vad,
        uplink_decoder,
        real_audio,
        is_test,
        abort: Arc::new(AtomicBool::new(false)),
    };
    session.run().await
}

pub(crate) async fn send_text(socket: &mut WebSocket, msg: &ServerMessage) -> anyhow::Result<()> {
    socket
        .send(Message::Text(Utf8Bytes::from(msg.to_json())))
        .await
        .map_err(|e| anyhow::anyhow!("发送文本失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {}

pub(crate) async fn send_binary(socket: &mut WebSocket, data: Vec<u8>) -> anyhow::Result<()> {
    socket
        .send(Message::Binary(data.into()))
        .await
        .map_err(|e| anyhow::anyhow!("发送二进制失败: {e}"))?;
    Ok(())
}
