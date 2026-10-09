//! 每连接会话状态机与语音流水线。
//!
//! 两条路径（由 hello 的 `test` 参数区分）：
//! - ESP 设备（正式流程）：上行 Opus → 解码 → VAD → ASR → stt → LLM → tts → 下行 Opus。
//!   其中 ASR 与 LLM **之间**还有一道可选的指令闸门（`[command]`，见 [`command`] 子模块）：
//!   识别到「退下/闭嘴/关闭」这类指令即说一句告别语并**断开本次会话**（跳过 LLM）。
//! - macApp 测试台（hello 带 `test:true`）：`asr_test`/`tts_test`/`llm_test` 三种
//!   **独立服务请求-响应**（各自直调对应真实引擎、各自回包），不进设备流水线；
//!   设备会话收到这三类消息一律忽略。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::app::audio::opus::OpusFrameDecoder;
use crate::app::downlink::{send_sentence_audio_stream, send_tts_state, DownlinkPacer};
use crate::app::protocol::{unwrap_uplink, BinVersion, ClientMessage, ServerMessage};
use crate::app::transport::{IncomingFrame, Transport};
use crate::engine::Engines;
use crate::plugins::aiui::{AiuiSession, FullChainEngine, FullChainTurn};
use crate::plugins::asr::AsrEngine;
use crate::plugins::command::CommandGate;
use crate::plugins::vad::VadEngine;

mod bench;
mod command;
mod reply;

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
    /// 下行抖动缓冲提前量（毫秒），见 `[audio].downlink_lead_ms`。
    pub downlink_lead_ms: u32,
}

/// LLM 失败且用户尚未听到任何内容时的兜底回应（**不暴露技术细节**，只给可行动的安抚）。
const FALLBACK_REPLY: &str = "抱歉，我这边网络不太顺畅，请稍后再试。";

/// 单连接会话：承载 [`Transport`]、引擎引用与全部可变会话状态。
///
/// 流水线函数（`handle_text`/`handle_binary`/`stream_response`/`send_tts_audio`/
/// `recognize_segments`）原本要传 7~11 个离散参数（含多个可变状态），增删一个状态
/// 就要改 N 处签名。聚合成 `Session` 后统一用 `&mut self` 访问，消除参数膨胀与漏传风险。
/// 泛型 `T: Transport` 静态分发：会话/流水线不感知承载协议（当前 WS，未来
/// MQTT+UDP / 本机通道各实现一份 [`Transport`] 即可复用整条流水线）。
pub struct Session<'a, T: Transport> {
    transport: &'a mut T,
    engines: Arc<Engines>,
    params: SessionParams,
    session_id: String,
    /// 设备标识（握手头 `Device-Id`）：记忆作用域的稳定键（`xiaozhi:<device_id>`）
    /// 与 `{{device_id}}` 插值的取值来源。
    device_id: String,
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
    /// 全链路引擎（[aiui].enabled 时启用；跨轮次长连接，Mutex 供 spawn_blocking 互斥）。
    aiui: Option<Arc<Mutex<dyn FullChainEngine>>>,
    /// 下行节拍器：按帧长匀速下发（保护 ESP 解码队列，见 `downlink` 模块文档）。
    /// 一次回复内跨句共享时间表（句间无缝）；每轮回复开始时 reset。
    pacer: DownlinkPacer,
    /// 语音指令闸门（`[command]`）：ASR → LLM 之间的拦截器。
    /// 会话开始时按配置快照构造一次 = 注册表声明的 `next_session` 语义。
    commands: CommandGate,
    /// 指令要求断开本次会话（告别语已下发）：主循环收尾时主动关闭承载。
    close_requested: bool,
}

impl<'a, T: Transport> Session<'a, T> {
    /// 主循环：接收上行帧并分发；循环结束后 flush VAD 残留段并送流水线。
    async fn run(mut self) -> anyhow::Result<()> {
        loop {
            match self.transport.recv().await {
                IncomingFrame::Text(s) => {
                    if let Err(e) = self.handle_text(&s).await {
                        tracing::warn!("处理文本消息失败: {e}");
                    }
                }
                IncomingFrame::Binary(b) => {
                    self.handle_binary(&b).await;
                }
                // 承载关闭/读取错误：会话结束（recv 内已记日志）
                IncomingFrame::Closed => break,
            }
            // 指令闸门命中（[command]）：告别语已下发，主动断开本次会话
            if self.close_requested {
                tracing::info!("session {} 按指令断开本次会话", self.session_id);
                break;
            }
        }

        // 会话结束：flush VAD 残留语音段（真实音频模式），尽力把最后一句话送完流水线。
        // 连接可能已关闭，此时发送会失败并被记录，属正常情况。
        // ⚠️ 指令断开会话时**不** flush：用户已明确要求退下，不该再为残留音频开一轮对话。
        if self.real_audio && !self.close_requested {
            let mut segments = Vec::new();
            self.vad.flush(&mut |seg| segments.push(seg));
            self.recognize_segments(segments).await;
        }
        if self.close_requested {
            // 告别语的下行帧是**按实时节奏**下发的：最后一帧发出时，设备解码队列里还有约
            // `downlink_lead_ms` 的音频没播完（见 `downlink` 模块的节拍器文档）。立即关闭连接
            // 有可能把告别语的尾巴掐掉，因此留一段等长的排空时间（夹在 100ms~1s，避免异常
            // 配置把会话收尾拖住）。失败无妨——会话即将随 Transport 释放而结束。
            let drain_ms = self.params.downlink_lead_ms.clamp(100, 1000) as u64;
            tokio::time::sleep(std::time::Duration::from_millis(drain_ms)).await;
            self.transport.close().await;
        }
        Ok(())
    }

    /// 在 `spawn_blocking` 中执行同步阻塞的 ASR 识别，统一处理任务取消/panic 与内部错误。
    /// `recognize_segments` 与 `AsrTest` 共用，避免重复 clone 引擎与计时样板。
    /// 写成不借用 `&self` 的关联函数：避免捕获 `Session`（含 `&mut T` 传输，非 `Sync`）
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
        // AIUI 全链路模式：切段即轮次，识别+大模型+合成在 AIUI 云端闭环
        if self.aiui.is_some() {
            self.aiui_segments(segments).await;
            return;
        }
        for seg in segments {
            let t0 = std::time::Instant::now();
            let user_text =
                match Self::asr_recognize(self.engines.asr.clone(), self.params.uplink_sr, seg)
                    .await
                {
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
            // 指令闸门（ASR → LLM **之间**）：命中即说告别语并断开会话，**一次 LLM 都不调用**。
            // 本批剩余语音段一并丢弃（用户已经要求退下）。
            if self.handle_command_gate(&user_text).await {
                return;
            }
            if let Err(e) = self.stream_response(&user_text).await {
                tracing::warn!("流水处理失败: {e}");
            }
        }
    }

    /// AIUI 全链路：每段 = 一轮 oneshot 交互（识别 + 大模型 + 合成在 AIUI 云端闭环）。
    /// stt/nlp 文本照常走 stt 消息与会话历史；合成音频（16k PCM）复用现有下行链路。
    async fn aiui_segments(&mut self, segments: Vec<Vec<f32>>) {
        let session_id = self.session_id.clone();
        for seg in segments {
            let pcm_i16 = f32_to_i16_16k(&seg, self.params.uplink_sr);
            let Some(aiui) = self.aiui.as_ref().map(Arc::clone) else {
                continue;
            };
            let t0 = std::time::Instant::now();
            let res = blocking_result(move || {
                let mut g = aiui.lock().unwrap_or_else(|e| e.into_inner());
                g.turn(&pcm_i16)
            })
            .await;
            let turn = match res {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("session {session_id} AIUI 轮次失败: {e}");
                    continue;
                }
            };
            if turn.stt.trim().is_empty() {
                continue; // 静音轮（云端 Silence 事件），同空识别结果处理
            }
            tracing::info!(
                "session {session_id} AIUI 轮次完成：耗时 {}ms（识别 {} 字 / 回复 {} 字 / 音频 {} 字节）",
                t0.elapsed().as_millis(),
                turn.stt.chars().count(),
                turn.reply.chars().count(),
                turn.audio.len()
            );
            // 指令闸门同样适用于全链路：识别文本要等云端闭环回来才可见（本地没有 ASR 可拦），
            // 因此这里只能"丢弃本轮云端回复"—但会话照样按指令断开（语义与级联流水线一致）。
            if self.handle_command_gate(&turn.stt).await {
                return;
            }
            if let Err(e) = self.aiui_downlink(&turn).await {
                tracing::warn!("session {session_id} AIUI 下行失败: {e}");
            }
        }
    }

    /// AIUI 轮次结果下发：stt 文本 + tts start → 音频流 → tts stop → 会话历史。
    async fn aiui_downlink(&mut self, turn: &FullChainTurn) -> anyhow::Result<()> {
        self.pacer.reset(); // 新一轮回复：节拍器时间表重置
        send_text(
            self.transport,
            &ServerMessage::Stt {
                session_id: self.session_id.clone(),
                text: turn.stt.clone(),
            },
        )
        .await?;
        send_tts_state(self.transport, &self.session_id, "start").await?;
        // i16 LE 字节 → f32 分片进既有下行通道（3200 字节 = 1600 样本 = 100ms @16k）
        let (tx, chunk_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<f32>>();
        for chunk in turn.audio.chunks(3200) {
            let samples: Vec<f32> = chunk
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                .collect();
            let _ = tx.send(samples);
        }
        drop(tx);
        send_sentence_audio_stream(
            self.transport,
            &self.params,
            &mut self.downlink_ts,
            &self.abort,
            &self.session_id,
            &turn.stt,
            16000,
            chunk_rx,
            &mut self.pacer,
            None, // AIUI 全链路回传音频不入缓存（音频由云端给定，同句不保证可复现）
            &mut None,
        )
        .await?;
        send_tts_state(self.transport, &self.session_id, "stop").await?;
        // 历史语义与级联流水线一致：被打断（下行早停）不入历史，避免脏上下文
        if !self.abort.load(Ordering::Relaxed) && !turn.reply.trim().is_empty() {
            self.history.push(("user".to_string(), turn.stt.clone()));
            self.history
                .push(("assistant".to_string(), turn.reply.clone()));
            let max_history = self.engines.live_config().llm.max_history;
            while self.history.len() > max_history {
                self.history.remove(0);
            }
        }
        Ok(())
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
                    self.transport,
                    &ServerMessage::Mcp {
                        session_id: Some(session_id),
                        payload,
                    },
                )
                .await?;
            }
            ClientMessage::AsrTest { action, .. } => {
                // 测试台专用端点（仅 hello.test=true 受理）；实现见 session/bench.rs
                if !self.is_test {
                    tracing::warn!("session {session_id} 非测试会话收到 asr_test，忽略");
                    return Ok(());
                }
                self.handle_asr_test(action).await?;
            }
            ClientMessage::TtsTest {
                text,
                speaker,
                lang,
                speed,
                vcn,
                ..
            } => {
                // 测试台专用端点（仅 hello.test=true 受理）；实现见 session/bench.rs
                if !self.is_test {
                    tracing::warn!("session {session_id} 非测试会话收到 tts_test，忽略");
                    return Ok(());
                }
                self.handle_tts_test(text, speaker, lang, speed, vcn)
                    .await?;
            }
            ClientMessage::LlmTest { text, .. } => {
                // 测试台专用端点（仅 hello.test=true 受理）；实现见 session/bench.rs
                if !self.is_test {
                    tracing::warn!("session {session_id} 非测试会话收到 llm_test，忽略");
                    return Ok(());
                }
                self.handle_llm_test(text).await?;
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
}

/// 会话主循环：收发消息、驱动流水线。`is_test` 来自 hello 的 `test` 参数，
/// 只有测试台会话能发起 asr_test/tts_test/llm_test（设备正式流程不经过测试端点）。
pub async fn run_session<T: Transport>(
    transport: &mut T,
    engines: Arc<Engines>,
    params: SessionParams,
    session_id: String,
    device_id: String,
    is_test: bool,
) -> anyhow::Result<()> {
    // 会话开始时按磁盘最新配置热刷新引擎（管理页保存后**新会话即生效**，无需重启）。
    // ⚠️ 必须在 spawn_blocking 内：sherpa 重建引擎可能加载模型数秒，直接调用会独占
    // tokio worker（见 engine.rs 模块注释的 CPU 预算）。刷新失败保持旧引擎，不致命。
    {
        let engines = engines.clone();
        let sid = session_id.clone();
        match tokio::task::spawn_blocking(move || engines.refresh_from_disk()).await {
            Ok(notes) => {
                for n in notes {
                    tracing::warn!("session {sid} 引擎热刷新：{n}");
                }
            }
            Err(e) => tracing::warn!("session {sid} 引擎热刷新任务异常: {e}"),
        }
    }
    let vad = engines.new_vad()?;
    let uplink_decoder = OpusFrameDecoder::new(params.uplink_sr)?;
    let real_audio = engines.config.real_audio();
    // AIUI 全链路会话：sn = 前缀 + 会话短 id（云端个性化/上下文绑定，≤32 字符）
    // ⚠️ AIUI 全链路刻意用**启动快照**（注册表里是 RestartOnly）：它决定整条流水线的
    // 走向，且启用时凭据缺失是致命错误（boot 即失败）；热启用会带来"半配置切换链路"的风险。
    let aiui = if engines.config.aiui.enabled {
        let sn = format!(
            "{}-{}",
            engines.config.aiui.sn_prefix,
            session_id.get(..8).unwrap_or(&session_id)
        );
        let engine: Arc<Mutex<dyn FullChainEngine>> = Arc::new(Mutex::new(AiuiSession::new(
            engines.config.aiui.clone(),
            sn,
        )));
        Some(engine)
    } else {
        None
    };
    // 指令闸门快照：会话开始时取一次（= 注册表声明的 next_session 语义）。
    // 未启用时是恒不命中的空闸门，零开销、零行为变化。
    let commands = CommandGate::new(&engines.live_config().command);
    let session = Session {
        transport,
        engines,
        params,
        session_id,
        device_id,
        history: Vec::new(),
        downlink_ts: 0,
        test_recording: false,
        test_buf: Vec::new(),
        vad,
        uplink_decoder,
        real_audio,
        is_test,
        abort: Arc::new(AtomicBool::new(false)),
        aiui,
        pacer: DownlinkPacer::new(params.downlink_lead_ms as u64),
        commands,
        close_requested: false,
    };
    session.run().await
}

/// 上行 f32 PCM → 16k i16（AIUI 仅收 16k；设备默认上行 16k，非 16k 走线性插值重采样）。
fn f32_to_i16_16k(pcm: &[f32], uplink_sr: u32) -> Vec<i16> {
    if uplink_sr == 16_000 {
        return pcm
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();
    }
    let ratio = uplink_sr as f32 / 16_000.0;
    let mut out = Vec::with_capacity((pcm.len() as f32 / ratio) as usize + 1);
    let mut pos = 0.0f32;
    while (pos as usize) < pcm.len() {
        let i = pos as usize;
        let frac = pos - i as f32;
        let a = pcm[i];
        let b = pcm.get(i + 1).copied().unwrap_or(a);
        out.push(((a + (b - a) * frac).clamp(-1.0, 1.0) * 32767.0) as i16);
        pos += ratio;
    }
    out
}

/// 协议层发送：`ServerMessage` 序列化后交承载下发（错误映射在承载实现内完成）。
pub(crate) async fn send_text<T: Transport>(
    transport: &mut T,
    msg: &ServerMessage,
) -> anyhow::Result<()> {
    transport.send_text(msg.to_json()).await
}

pub(crate) async fn send_binary<T: Transport>(
    transport: &mut T,
    data: Vec<u8>,
) -> anyhow::Result<()> {
    transport.send_binary(data).await
}
