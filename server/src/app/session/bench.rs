//! 测试台（macApp，`hello.test=true`）三个专用端点的处理实现：
//! `asr_test`（录音缓冲整段识别）/ `tts_test`（合成 + 流式下发）/ `llm_test`（直调 LLM 验证）。
//!
//! ⚠️ 这三类消息**仅测试会话受理**（`session.rs` 的 match 臂先做 `is_test` 检查）；
//! 本文件作为 [`super::Session`] 的子模块 impl 扩展拆分自 `session.rs`（可访问会话私有字段）。
//! 服务端每次都按**磁盘最新配置**工作（读盘热切换），因此测试台改配置保存后无需重启。

use crate::app::downlink::{send_sentence_audio_stream, send_tts_state};
use crate::app::protocol::ServerMessage;
use crate::app::session::{send_text, Session};
use crate::app::transport::Transport;
use crate::config::Config;
use crate::plugins::llm::build_llm;
use crate::plugins::tts::{build_tts, TtsBackendKind};

impl<'a, T: Transport> Session<'a, T> {
    /// `asr_test`：start 开始录音缓冲 / stop 对整段缓冲一次性识别（跳过 VAD），结果回 `stt`。
    pub(super) async fn handle_asr_test(&mut self, action: String) -> anyhow::Result<()> {
        let session_id = self.session_id.clone();
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
                            self.transport,
                            &ServerMessage::Stt {
                                session_id,
                                text,
                            },
                        )
                        .await?;
                    }
                    _ => tracing::warn!("未知 asr_test action: {action}"),
                    }
        Ok(())
    }

    /// `tts_test`：按磁盘最新配置热切换引擎（可选 vcn 覆盖）→ 流式合成 → 复用下行链路。
    pub(super) async fn handle_tts_test(
        &mut self,
        text: String,
        speaker: Option<i32>,
        lang: Option<String>,
        speed: Option<f32>,
        vcn: Option<String>,
    ) -> anyhow::Result<()> {
        let session_id = self.session_id.clone();
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
                    // 测试台音色覆盖：macApp 把当前选中的 vcn 随请求带上（无需先保存配置）。
                    let vcn_override = vcn.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
                    tracing::info!(
                        "session {session_id} 测试合成：lang={lang} speaker={speaker} speed={speed}{}",
                        vcn_override
                            .as_deref()
                            .map(|v| format!(" vcn={v}（测试台指定）"))
                            .unwrap_or_default()
                    );
                    // 热切换：先按磁盘最新配置刷新 TTS 引擎（管理页保存后无需重启即可测新配置）。
                    // 刷新失败（读盘/构建失败）以 tts_test 结果回报，测试台对话框直接可见。
                    let engines = self.engines.clone();
                    let refresh = tokio::task::spawn_blocking(move || engines.refresh_tts_from_disk())
                        .await
                        .map_err(|e| anyhow::anyhow!("TTS 热切换任务异常: {e}"))?;
                    if let Err(msg) = refresh {
                        tracing::warn!("session {session_id} {msg}");
                        send_text(
                            self.transport,
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
                    // 引擎选择：带 vcn 覆盖时按**磁盘最新配置**（凭据以保存值为准）即时构建
                    // 讯飞合成器（短连接、无模型加载，成本可忽略），本次合成用所选音色；
                    // 未带 vcn 时用热切换后的默认引擎。
                    let disk_tts = match &self.engines.config_path {
                        Some(p) => Config::load(p).map(|c| c.tts).unwrap_or_else(|e| {
                            tracing::warn!("session {session_id} 测试合成读盘配置失败（用启动配置）: {e}");
                            self.engines.config.tts.clone()
                        }),
                        None => self.engines.config.tts.clone(),
                    };
                    let (tts, vcn_note) = match &vcn_override {
                        Some(v) => {
                            if disk_tts.backend_kind() != TtsBackendKind::Xfyun {
                                // 音色覆盖只对讯飞远程引擎有意义；服务端仍是本地引擎时给出可行动提示
                                let msg = format!(
                                    "音色「{v}」为远程音色，但服务端当前 TTS 引擎是本地（{}）：                                 请先在 TTS 配置卡把「合成方式」切到远程服务并保存配置",
                                    self.engines.tts_name()
                                );
                                tracing::warn!("session {session_id} {msg}");
                                send_text(
                                    self.transport,
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
                            let mut cfg = disk_tts.clone();
                            cfg.xfyun.voice = v.clone();
                            match build_tts(&cfg) {
                                Ok(e) => (e, format!("，vcn={v}（测试台指定）")),
                                Err(e) => {
                                    let msg = format!("按所选音色构建讯飞合成器失败: {e:#}");
                                    tracing::warn!("session {session_id} {msg}");
                                    send_text(
                                        self.transport,
                                        &ServerMessage::TtsTestResult {
                                            session_id,
                                            state: "error".to_string(),
                                            engine: Some("xfyun".to_string()),
                                            text: Some(msg),
                                        },
                                    )
                                    .await?;
                                    return Ok(());
                                }
                            }
                        }
                        None => {
                            let e = self.engines.tts_for(&lang);
                            let note = if e.name() == "xfyun" {
                                format!("，vcn={}（服务端配置）", disk_tts.xfyun.voice)
                            } else {
                                String::new()
                            };
                            (e, note)
                        }
                    };
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
                        self.transport,
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
                                "session {session_id} 测试合成完成（{engine_name}）：耗时 {}ms，{} 帧 / {tts_sr}Hz{vcn_note}",
                                t0.elapsed().as_millis(),
                                frames
                            );
                            // 成功回报（音频已下发；state=ok, text=耗时）
                            send_text(
                                self.transport,
                                &ServerMessage::TtsTestResult {
                                    session_id,
                                    state: "ok".to_string(),
                                    engine: Some(engine_name),
                                    text: Some(format!("{}ms", t0.elapsed().as_millis())),
                                },
                            )
                            .await?;
                            // 协议收尾：与设备流水线同构，以 tts stop 结束本次播报。测试台原生模块
                            // 在 ok 帧只记引擎名，**tts stop 才消费 speak 回调**——缺 stop 会卡
                            // 「合成中」（7753f12 全链路流式改造时丢失，实测回归）。
                            send_tts_state(self.transport, &self.session_id, "stop").await?;
                        }
                        Ok(Err(e)) => {
                            tracing::warn!("session {session_id} 测试合成失败（耗时 {}ms）: {e}", t0.elapsed().as_millis());
                            send_text(
                                self.transport,
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
                                self.transport,
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
        Ok(())
    }

    /// `llm_test`：按**磁盘最新配置**临时构建 LLM 客户端直调（单轮无历史），结果回 `llm_test`。
    pub(super) async fn handle_llm_test(&mut self, text: String) -> anyhow::Result<()> {
        let session_id = self.session_id.clone();
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
                    let result = llm.chat_stream(&[], &text, None, Box::new(|_| true)).await;
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
                        self.transport,
                        &ServerMessage::LlmTestResult {
                            session_id,
                            state: state.to_string(),
                            text: Some(reply),
                            elapsed_ms: Some(elapsed),
                        },
                    )
                    .await?;
        Ok(())
    }
}
