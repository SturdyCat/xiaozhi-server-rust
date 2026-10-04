//! 每连接会话状态机与语音流水线。
//!
//! 两条路径：
//! - 真实（`sherpa` feature 且 backend=sherpa）：上行 Opus → 解码 → VAD → ASR → stt → LLM → tts → 下行 Opus。
//! - mock（默认）：收到 `listen start` 后直接触发 mock ASR → LLM → mock TTS，验证协议回包。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures_util::{FutureExt, StreamExt};

use crate::audio::opus::{OpusFrameDecoder, OpusFrameEncoder};
use crate::audio::resample::resample;
use crate::config::Config;
use crate::engine::Engines;
use crate::llm::{LlmEvent, build_llm};
use crate::protocol::{BinVersion, ClientMessage, ServerMessage, unwrap_uplink, wrap_downlink};
use crate::vad::VadEngine;

/// 无标点兜底句长（字符数）：LLM 长输出无标点时按此长度强制切句，保证首句延迟有上界。
const MAX_SENTENCE_CHARS: usize = 50;

/// LLM 流式文本按句切分器。
///
/// 命中句末标点（`。！？；` 及 ASCII `! ? ; \n`）即出句（标点保留在句尾）；
/// 流结束 `flush` 残句；无标点超长按 [`MAX_SENTENCE_CHARS`] 兜底硬切。
/// 已知取舍：英文缩写 `Mr.`/`Dr.` 会被误切——第一版接受（对齐方案文档 §3.2）。
struct SentenceSplitter {
    buf: String,
}

impl SentenceSplitter {
    fn new() -> Self {
        Self { buf: String::new() }
    }

    /// 喂入文本增量，返回本次切出的完整句（可能 0~N 句）。
    fn feed(&mut self, delta: &str) -> Vec<String> {
        self.buf.push_str(delta);
        let mut out = Vec::new();
        while let Some(pos) = find_sentence_end(&self.buf) {
            let s: String = self.buf.drain(..pos).collect();
            push_sentence(&mut out, &s);
        }
        // 兜底：无标点且已超长 → 按最大句长硬切（对齐字符边界，避免切碎多字节字符）。
        // 循环切：单次大 delta 也全部切块，只留不足句长的尾巴。
        while self.buf.chars().count() >= MAX_SENTENCE_CHARS {
            let cut = self
                .buf
                .char_indices()
                .nth(MAX_SENTENCE_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(self.buf.len());
            let s: String = self.buf.drain(..cut).collect();
            push_sentence(&mut out, &s);
        }
        out
    }

    /// 流结束：返回剩余未成句的残句（可能为空）。
    fn flush(&mut self) -> Vec<String> {
        let rest = std::mem::take(&mut self.buf);
        let mut out = Vec::new();
        push_sentence(&mut out, &rest);
        out
    }
}

/// 返回第一个句末标点（含该标点）之后的字节偏移；无则 `None`。
fn find_sentence_end(buf: &str) -> Option<usize> {
    for (idx, ch) in buf.char_indices() {
        if matches!(ch, '。' | '！' | '？' | '；' | '!' | '?' | ';' | '\n') {
            return Some(idx + ch.len_utf8());
        }
    }
    None
}

/// 句子去除首尾空白后入列；空句（如纯换行）跳过。
fn push_sentence(out: &mut Vec<String>, s: &str) {
    let s = s.trim();
    if !s.is_empty() {
        out.push(s.to_string());
    }
}

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

    /// 将 VAD 切出的语音段送 ASR → 完整流水线；常规分帧与会话结束 flush 共用。
    async fn recognize_segments(&mut self, segments: Vec<Vec<f32>>) {
        let session_id = self.session_id.clone();
        for seg in segments {
            // 识别是同步阻塞的 CPU 调用：spawn_blocking 隔离（勿在 async 线程直接跑）
            let asr = self.engines.asr.clone();
            let sr = self.params.uplink_sr;
            let t0 = std::time::Instant::now();
            let user_text = match blocking_result(move || asr.recognize(&seg, sr)).await {
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
                if self.real_audio {
                    tracing::info!("session {session_id} 进入 listening（真实音频模式）");
                } else {
                    let user_text = match self.engines.asr.recognize(&[], self.params.uplink_sr) {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!("mock ASR 失败: {e}");
                            return Ok(());
                        }
                    };
                    self.stream_response(&user_text).await?;
                }
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
            ClientMessage::AsrTest { action, .. } => match action.as_str() {
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
                        // 识别是同步阻塞的 CPU 调用：spawn_blocking 隔离（勿在 async 线程直接跑）
                        let asr = self.engines.asr.clone();
                        let sr = self.params.uplink_sr;
                        let t0 = std::time::Instant::now();
                        let recognized = blocking_result(move || asr.recognize(&buf, sr)).await;
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
                    .unwrap_or_else(|| self.engines.config.tts.lang.clone());
                let speaker = speaker.unwrap_or(self.engines.config.tts.speaker);
                let speed = speed.unwrap_or(self.engines.config.tts.speed);
                tracing::info!("session {session_id} 测试合成：lang={lang} speaker={speaker} speed={speed}");
                let tts = self.engines.tts_for(&lang);
                let t0 = std::time::Instant::now();
                // 推理是同步阻塞的 CPU 密集调用（实测单次 8~20s）：必须放 spawn_blocking，
                // 否则整个 tokio worker 线程被独占，期间同 runtime 的其他会话/HTTP 全部卡死。
                let text_c = text.clone();
                let synthesis = blocking_result(move || tts.synthesize(&text_c, speed, speaker)).await;
                let (pcm, tts_sr) = match synthesis {
                    Ok(x) => x,
                    Err(e) => {
                        tracing::warn!("测试合成失败（耗时 {}ms）: {e}", t0.elapsed().as_millis());
                        return Ok(());
                    }
                };
                tracing::info!(
                    "session {session_id} 合成完成：耗时 {}ms，音频 {:.2}s / {tts_sr}Hz（{} 样本，RTF={:.2}）",
                    t0.elapsed().as_millis(),
                    pcm.len() as f32 / tts_sr as f32,
                    pcm.len(),
                    t0.elapsed().as_secs_f32() / (pcm.len() as f32 / tts_sr as f32).max(0.01)
                );
                let frames = self.send_tts_audio(&text, &pcm, tts_sr).await?;
                tracing::info!(
                    "session {session_id} 下行发送完成：{frames} 帧（{}ms / {}ms 帧）",
                    frames as u32 * self.params.downlink_frame_ms,
                    self.params.downlink_frame_ms
                );
            }
            ClientMessage::LlmTest { text, .. } => {
                if text.trim().is_empty() {
                    return Ok(());
                }
                // 测试台 LLM 连通性验证：按**磁盘上最新配置**临时构建 LLM 客户端——
                // 引擎在启动时构建（engine.rs），改完配置无需重启即可验证刚保存的
                // api_base/api_key/model；未指定配置文件（内置 mock 默认）或读盘失败
                // 时退回启动时的内存配置。单轮直调、不读写会话历史。
                let llm_cfg = match &self.engines.config_path {
                    Some(p) => Config::load(p).map(|c| c.llm).unwrap_or_else(|e| {
                        tracing::warn!("session {session_id} LLM 测试读配置失败（用启动配置）: {e}");
                        self.engines.config.llm.clone()
                    }),
                    None => self.engines.config.llm.clone(),
                };
                tracing::info!(
                    "session {session_id} LLM 测试：backend={} model={} api_base={}",
                    llm_cfg.backend,
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
        // 网页测试台录音中：直接缓冲整段 PCM，跳过 VAD（mock 后端也可用）。
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

    /// 非阻塞抽取 socket 中已到达的打断/关闭消息（流水线各等待点轮询）。
    ///
    /// 返回当前打断标志（收到 abort、连接断开或已被置位均视为打断）。
    /// 注意：播报期间到达的上行二进制音频会被丢弃——打断以 abort 文本消息为准
    /// （ESP barge-in 时设备先发 abort 再重新 listen）。
    fn poll_abort(&mut self) -> bool {
        loop {
            match self.socket.next().now_or_never() {
                None => break, // 暂无待处理消息
                Some(None) => {
                    tracing::debug!("session {} socket 已关闭（视为打断）", self.session_id);
                    self.abort.store(true, Ordering::Relaxed);
                    break;
                }
                Some(Some(Ok(m))) => {
                    if let Message::Text(t) = &m {
                        if matches!(
                            serde_json::from_str::<ClientMessage>(t),
                            Ok(ClientMessage::Abort { .. })
                        ) {
                            tracing::info!("session {} 收到 abort（流水线终止）", self.session_id);
                            self.abort.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                    // 其余消息（上行音频/listen 等）在播报期丢弃
                }
                Some(Some(Err(e))) => {
                    tracing::debug!("session {} socket 读取错误（视为断开）: {e}", self.session_id);
                    self.abort.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }
        self.abort.load(Ordering::Relaxed)
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
                    if self.poll_abort() {
                        interrupted = true;
                        break;
                    }
                    if !tts_started {
                        self.send_tts_start().await?;
                        tts_started = true;
                    }
                    // 合成是同步阻塞的 CPU 调用：spawn_blocking 隔离（勿在 async 线程直接跑）
                    let tts = self.engines.tts();
                    let speed = self.engines.config.tts.speed;
                    let speaker = self.engines.config.tts.speaker;
                    let t_tts = std::time::Instant::now();
                    let s = sentence.clone();
                    match blocking_result(move || tts.synthesize(&s, speed, speaker)).await {
                        Ok((pcm, tts_sr)) => {
                            if self.poll_abort() {
                                interrupted = true;
                                break;
                            }
                            let frames =
                                self.send_sentence_audio(&sentence, &pcm, tts_sr).await?;
                            sentences_done += 1;
                            if sentences_done == 1 {
                                tracing::info!(
                                    "session {session_id} 首句下发（TTFA≈{}ms，句子合成 {}ms，{frames} 帧）",
                                    t0.elapsed().as_millis(),
                                    t_tts.elapsed().as_millis()
                                );
                            }
                        }
                        Err(e) => tracing::warn!("session {session_id} 句子合成失败: {e}"),
                    }
                    if self.poll_abort() {
                        interrupted = true;
                        break;
                    }
                }
                Ok(None) => break, // 生产者结束（tx 已 drop）
                Err(_elapsed) => {
                    // 等待超时：轮询打断 + 长静默期 Ping 保活（tungstenite 自动回 Pong）
                    if self.poll_abort() {
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
            self.send_tts_stop().await?;
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

    /// 发送 `tts start`（一段播报的开始，一次对话轮只发一次）。
    async fn send_tts_start(&mut self) -> anyhow::Result<()> {
        send_text(
            &mut self.socket,
            &ServerMessage::Tts {
                session_id: self.session_id.clone(),
                state: "start".to_string(),
                text: None,
            },
        )
        .await
    }

    /// 发送 `tts stop`（一段播报的结束；打断时也发，设备据此停播并清队列）。
    async fn send_tts_stop(&mut self) -> anyhow::Result<()> {
        send_text(
            &mut self.socket,
            &ServerMessage::Tts {
                session_id: self.session_id.clone(),
                state: "stop".to_string(),
                text: None,
            },
        )
        .await
    }

    /// 发送单句：`sentence_start(text)` → 重采样/Opus 分帧逐帧二进制（逐帧轮询打断）。
    /// 返回实际下发的帧数（供日志统计耗时/流量）。
    async fn send_sentence_audio(
        &mut self,
        text: &str,
        pcm: &[f32],
        tts_sr: u32,
    ) -> anyhow::Result<usize> {
        send_text(
            &mut self.socket,
            &ServerMessage::Tts {
                session_id: self.session_id.clone(),
                state: "sentence_start".to_string(),
                text: Some(text.to_string()),
            },
        )
        .await?;

        let pcm_down = resample(pcm, tts_sr, self.params.downlink_sr);
        let pcm_down: &[f32] = pcm_down.as_ref();
        let frame_samples =
            (self.params.downlink_sr as f32 * self.params.downlink_frame_ms as f32 / 1000.0) as usize;
        // 整段下行共用一个流式编码器：逐帧新建会让每个 60ms 边界都是「流重启」→
        // 实听「声音不连续」（详见 opus.rs 的 OpusFrameEncoder 注释）。
        let mut encoder = OpusFrameEncoder::new(self.params.downlink_sr)?;
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
            let framed = wrap_downlink(self.params.downlink_bin_ver, &opus, self.downlink_ts);
            self.downlink_ts += self.params.downlink_frame_ms;
            frames_sent += 1;
            send_binary(&mut self.socket, framed).await?;

            // 每帧后非阻塞轮询 abort / close
            if self.poll_abort() {
                tracing::info!("session {} 中断 TTS 播报（{text}）", self.session_id);
                break;
            }
        }
        // 残帧补零到整帧再编码（Opus 只接受合法帧长），避免最后不足一帧被丢弃。
        if n % frame > 0 && !self.abort.load(Ordering::Relaxed) {
            let tail = pad_tail(&pcm_down[n_full * frame..], frame);
            match encoder.encode_frame(&tail) {
                Ok(opus) => {
                    let framed = wrap_downlink(self.params.downlink_bin_ver, &opus, self.downlink_ts);
                    self.downlink_ts += self.params.downlink_frame_ms;
                    frames_sent += 1;
                    send_binary(&mut self.socket, framed).await?;
                }
                Err(e) => tracing::warn!("Opus 编码失败（尾帧）: {e}"),
            }
        }
        Ok(frames_sent)
    }

    /// 测试台整段路径：`start` → 单句全量 → `stop`（网页测试台 `tts_test` 共用）。
    async fn send_tts_audio(
        &mut self,
        text: &str,
        pcm: &[f32],
        tts_sr: u32,
    ) -> anyhow::Result<usize> {
        self.send_tts_start().await?;
        let frames = self.send_sentence_audio(text, pcm, tts_sr).await?;
        self.send_tts_stop().await?;
        Ok(frames)
    }
}

/// 会话主循环：收发消息、驱动流水线。
pub async fn run_session(
    mut socket: WebSocket,
    engines: Arc<Engines>,
    params: SessionParams,
    session_id: String,
) -> anyhow::Result<()> {
    let vad = engines.new_vad();
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

/// 末段 PCM 不足一整帧时补零到整帧（Opus 只接受合法帧长，否则 BadArgument）。
///
/// 下行热路径请直接 `pcm.chunks(frame)` 遍历整帧（零拷贝），仅对最后一个不足整帧的
/// 切片调用本函数补零后再编码，避免把整段 PCM 物化为 `Vec<Vec<f32>>` 造成整段拷贝。
fn pad_tail(pcm: &[f32], frame: usize) -> Vec<f32> {
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
    use super::{find_sentence_end, pad_tail, SentenceSplitter};

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

    #[test]
    fn splitter_splits_on_chinese_punctuation() {
        let mut sp = SentenceSplitter::new();
        assert!(sp.feed("今天天气不错，").is_empty(), "逗号不是句末标点");
        let out = sp.feed("适合出门。明天呢？");
        assert_eq!(out, vec!["今天天气不错，适合出门。", "明天呢？"]);
        assert!(sp.flush().is_empty());
    }

    #[test]
    fn splitter_multiple_sentences_single_delta() {
        let mut sp = SentenceSplitter::new();
        let out = sp.feed("一。二！三？四；");
        assert_eq!(out, vec!["一。", "二！", "三？", "四；"]);
    }

    #[test]
    fn splitter_flush_tail_without_punctuation() {
        let mut sp = SentenceSplitter::new();
        assert_eq!(sp.feed("你好。再见"), vec!["你好。"]);
        let out = sp.flush();
        assert_eq!(out, vec!["再见"]);
    }

    #[test]
    fn splitter_fallback_max_len_no_punctuation() {
        let mut sp = SentenceSplitter::new();
        let text: String = "啊".repeat(120);
        let out = sp.feed(&text);
        // 120 字无标点 → 按 50 字兜底切出 2 句，剩 20 字留在缓冲
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|s| s.chars().count() == 50));
        assert_eq!(sp.flush(), vec!["啊".repeat(20)]);
    }

    #[test]
    fn splitter_ascii_and_newline_boundaries() {
        let mut sp = SentenceSplitter::new();
        // \n 段去掉首尾空白后为空 → 跳过；"new line" 留在缓冲待 flush
        let out = sp.feed("ok! next? done;\nnew line");
        assert_eq!(out, vec!["ok!", "next?", "done;"]);
        assert_eq!(sp.flush(), vec!["new line"]);
    }

    #[test]
    fn splitter_fallback_aligned_to_char_boundary() {
        // 兜底硬切必须对齐多字节字符边界（中文 3 字节），不得切碎
        let mut sp = SentenceSplitter::new();
        let text: String = "语".repeat(60);
        let out = sp.feed(&text);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].chars().count(), 50);
        assert_eq!(sp.flush(), vec!["语".repeat(10)]);
    }

    #[test]
    fn find_sentence_end_positions() {
        assert_eq!(find_sentence_end("你好。"), Some(9)); // 3 字节 ×2 + 3 字节句号
        assert_eq!(find_sentence_end("你好"), None);
        assert_eq!(find_sentence_end(""), None);
        assert_eq!(find_sentence_end("hi!\n"), Some(3));
    }
}

async fn send_binary(socket: &mut WebSocket, data: Vec<u8>) -> anyhow::Result<()> {
    socket
        .send(Message::Binary(data.into()))
        .await
        .map_err(|e| anyhow::anyhow!("发送二进制失败: {e}"))?;
    Ok(())
}
