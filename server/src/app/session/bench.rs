//! 测试台（macApp，`hello.test=true`）三个专用端点的处理实现：
//! `asr_test`（录音缓冲整段识别）/ `tts_test`（合成 + 流式下发）/ `llm_test`（直调 LLM 验证）。
//!
//! ⚠️ 这三类消息**仅测试会话受理**（`session.rs` 的 match 臂先做 `is_test` 检查）；
//! 本文件作为 [`super::Session`] 的子模块 impl 扩展拆分自 `session.rs`（可访问会话私有字段）。
//! 服务端每次都按**磁盘最新配置**工作（读盘热切换），因此测试台改配置保存后无需重启。

use crate::app::downlink::{
    drain_reservoir, new_reservoir, send_tts_state, spawn_single_sentence_filler,
};
use crate::app::prefetch::PreparedAudio;
use crate::app::protocol::ServerMessage;
use crate::app::session::{send_text, Session};
use crate::app::transport::Transport;
use crate::plugins::llm::build_llm;
use crate::plugins::tts::build_xfyun;

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
                    let recognized = Self::asr_recognize(
                        self.engines.asr.clone(),
                        // 测试台与生产会话共用同一把 ASR 闸门（缺口①）：管理台测试台
                        // 与生产共享 CPU，不过闸门等于给生产埋雷。
                        self.engines.inference.asr(),
                        self.params.uplink_sr,
                        buf,
                    )
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
                send_text(self.transport, &ServerMessage::Stt { session_id, text }).await?;
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
        // 会话级最新快照：测试台看的就是"刚保存的配置"（热刷新后 live = 盘上值）
        let live = self.eff_config.clone();
        let lang = lang
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| live.tts.kokoro.lang.clone());
        let speaker = speaker.unwrap_or(live.tts.speaker);
        let speed = speed.unwrap_or(live.tts.speed);
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
        let disk_tts = if self.engines.config_path().is_none() {
            self.engines.config.tts.clone() // 内置默认配置：无来源可读
        } else {
            self.engines
                .config_store
                .load()
                .map(|c| c.tts)
                .unwrap_or_else(|e| {
                    tracing::warn!("session {session_id} 测试合成读盘配置失败（用启动配置）: {e}");
                    self.engines.config.tts.clone()
                })
        };
        let (tts, vcn_note) = match &vcn_override {
            Some(v) => {
                if disk_tts.engine_id() != "xfyun" {
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
                let mut xfyun = disk_tts.xfyun.clone();
                xfyun.voice = v.clone();
                match build_xfyun(&xfyun) {
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
        // 推理闸门（缺口①）：测试台合成与生产会话共用同一把 TTS 闸门——
        // 排队期响应打断（abort 感知），permit move 进闭包随合成结束 RAII 归还。
        let Some(tts_permit) = self
            .engines
            .inference
            .tts()
            .acquire_with_abort(&self.abort)
            .await
        else {
            tracing::info!("session {session_id} 测试合成在闸门排队期被打断");
            return Ok(());
        };
        // 流式合成：spawn_blocking 跑引擎，分片经 channel 边收边下发（首片即出声）。
        // 失败经 tts_test 结果回报测试台（不再静默）。
        // 与正常会话走**同一个下载缓冲**（按预算控制下载流速，见 `app::prefetch`）。
        let download = crate::app::prefetch::DownloadBudget::new(self.params.download_buffer_ms);
        let (tx, chunk_rx) = tokio::sync::mpsc::channel::<crate::app::prefetch::PcmChunk>(
            crate::app::prefetch::PREFETCH_PCM_SLOTS,
        );
        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<bool>();
        let text_c = text.clone();
        let abort_c = self.abort.clone();
        let budget_c = download.clone();
        let producer = tokio::task::spawn_blocking(move || {
            let _gate_permit = tts_permit; // RAII：闭包退出即归还推理闸门
            let r = tts.synthesize_stream(
                &text_c,
                speed,
                speaker,
                Box::new(move |sr, chunk| {
                    // 下载缓冲按预算限速 + 打断感知（见 `prefetch::push_pcm` 文档）
                    crate::app::prefetch::push_pcm(&budget_c, &tx, chunk.to_vec(), sr, &abort_c)
                }),
            );
            // 合成是否正常结束：水库填充任务据此决定该句能否写结果缓存
            let _ = done_tx.send(r.is_ok());
            r
        });
        // 测试台单次合成也走**同一条**下行链路（水库 + 节拍器；保护 ESP/一致行为）
        self.pacer.reset();
        let (sink, source) = new_reservoir(
            self.params.downlink_buffer_ms,
            self.params.downlink_frame_ms,
        );
        let filler = spawn_single_sentence_filler(
            self.abort.clone(),
            self.engines.tts_cache.clone(),
            self.params,
            sink,
            text.clone(),
            PreparedAudio::Stream {
                sr: tts_sr,
                chunks: chunk_rx,
                done: done_rx,
            },
            0, // 测试台测的是真实合成，不走缓存
        );
        let mut first_frame_at = None;
        let drain = drain_reservoir(
            self.transport,
            &self.params,
            &self.session_id,
            &self.abort,
            &mut self.downlink_ts,
            &mut self.pacer,
            source,
            t0,
            &mut first_frame_at,
        )
        .await?;
        filler.abort();
        let frames = drain.frames;
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
                tracing::warn!(
                    "session {session_id} 测试合成失败（耗时 {}ms）: {e}",
                    t0.elapsed().as_millis()
                );
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
        let llm_cfg = if self.engines.config_path().is_none() {
            self.engines.config.llm.clone() // 内置默认配置：无来源可读
        } else {
            self.engines
                .config_store
                .load()
                .map(|c| c.llm)
                .unwrap_or_else(|e| {
                    tracing::warn!("session {session_id} LLM 测试读配置失败（用启动配置）: {e}");
                    self.engines.config.llm.clone()
                })
        };
        tracing::info!(
            "session {session_id} LLM 测试：model={} api_base={}",
            llm_cfg.model,
            llm_cfg.api_base
        );
        let llm = build_llm(&llm_cfg);
        // 测试台也走**组装后的 prompt**：这样"人格是否生效"可以在测试台当场看到
        // （soul 文档 P0 的验收项），记忆召回同样按本会话作用域生效。
        let prompt = self.build_turn_prompt(&text).await;
        let t0 = std::time::Instant::now();
        let result = llm
            .chat_stream(&[], &text, &prompt, None, Box::new(|_| true))
            .await;
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
