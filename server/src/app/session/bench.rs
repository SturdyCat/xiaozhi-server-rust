//! 测试台（macApp，`hello.test=true`）四个专用端点的处理实现：
//! `asr_test`（录音缓冲整段识别）/ `tts_test`（合成 + 流式下发）/ `llm_test`（直调 LLM 验证）
//! / `nlu_test`（直调 TinyBERT 意图分类验证技能级联第二级）。
//!
//! ⚠️ 这四类消息**仅测试会话受理**（`session.rs` 的 match 臂先做 `is_test` 检查）；
//! 本文件作为 [`super::Session`] 的子模块 impl 扩展拆分自 `session.rs`（可访问会话私有字段）。
//! 服务端每次都按**磁盘最新配置**工作（读盘热切换），因此测试台改配置保存后无需重启。

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::app::downlink::{
    drain_reservoir, new_reservoir, send_tts_state, spawn_single_sentence_filler,
};
use crate::app::prefetch::PreparedAudio;
use crate::app::protocol::ServerMessage;
use crate::app::session::{send_text, Session};
use crate::app::transport::Transport;
use crate::plugins::llm::build_llm;
use crate::plugins::nlu::build_bert_checked;
use crate::plugins::tts::build_xfyun;

mod synth;

use synth::{race_synth, timeout_error_text, SynthRace, TTS_TEST_TIMEOUT};

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
        // 整链超时预算从这里起算（含热切换、闸门排队与合成，见 TTS_TEST_TIMEOUT）。
        let deadline = tokio::time::Instant::now() + TTS_TEST_TIMEOUT;
        let engines = self.engines.clone();
        let refresh_fut = tokio::task::spawn_blocking(move || engines.refresh_tts_from_disk());
        let refresh = match tokio::time::timeout_at(deadline, refresh_fut).await {
            Ok(joined) => joined.map_err(|e| anyhow::anyhow!("TTS 热切换任务异常: {e}"))?,
            Err(_) => {
                tracing::warn!("session {session_id} {}", timeout_error_text());
                send_text(
                    self.transport,
                    &ServerMessage::TtsTestResult {
                        session_id,
                        state: "error".to_string(),
                        engine: Some(self.engines.tts_name().to_string()),
                        text: Some(timeout_error_text()),
                    },
                )
                .await?;
                return Ok(());
            }
        };
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
        // 排队期响应打断（abort 感知）+ 同一超时预算；超时分支在拿到 permit 前
        // 直接放弃排队，不占许可。
        let gate = self.engines.inference.tts();
        let acquired = tokio::select! {
            p = gate.acquire_with_abort(&self.abort) => p,
            _ = tokio::time::sleep_until(deadline) => {
                tracing::warn!("session {session_id} 闸门排队超时：{}", timeout_error_text());
                send_text(
                    self.transport,
                    &ServerMessage::TtsTestResult {
                        session_id,
                        state: "error".to_string(),
                        engine: Some(self.engines.tts_name().to_string()),
                        text: Some(timeout_error_text()),
                    },
                )
                .await?;
                return Ok(());
            }
        };
        let Some(tts_permit) = acquired else {
            tracing::info!("session {session_id} 测试合成在闸门排队期被打断");
            return Ok(());
        };
        // 流式合成：spawn_blocking 跑引擎，分片经 channel 边收边下发（首片即出声）。
        // 失败经 tts_test 结果回报测试台（不再静默）。
        // 与正常会话走**同一个下载缓冲**（按预算控制下载流速，见 `app::prefetch`）。
        // `cancelled` 是本条 tts_test 独立的取消标志（不复用会话级 abort——超时只应
        // 取消本次合成，不波及会话其他状态）：race_synth 超时即置位，producer 的
        // 下一个分片回调拿到 false 提前终止、permit 随闭包退出归还。
        let download = crate::app::prefetch::DownloadBudget::new(self.params.download_buffer_ms);
        let (tx, chunk_rx) = tokio::sync::mpsc::channel::<crate::app::prefetch::PcmChunk>(
            crate::app::prefetch::PREFETCH_PCM_SLOTS,
        );
        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<bool>();
        let text_c = text.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_c = cancelled.clone();
        let budget_c = download.clone();
        let producer = tokio::task::spawn_blocking(move || {
            let _gate_permit = tts_permit; // RAII：闭包退出即归还推理闸门
            let r = tts.synthesize_stream(
                &text_c,
                speed,
                speaker,
                Box::new(move |sr, chunk| {
                    // 下载缓冲按预算限速 + 本条合成取消感知（见 `prefetch::push_pcm` 文档）
                    crate::app::prefetch::push_pcm(&budget_c, &tx, chunk.to_vec(), sr, &cancelled_c)
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
        // 下发 future 需与 producer 竞速：三者（合成/下发/超时）都要被持续 poll——
        // 停 poll 下发会让下载缓冲顶死、合成假死（见 `race_synth` 文档）。
        // 整个竞速块结束时 drain_fut（借 self.transport）即释放，超时分支的
        // 错误帧在块外发送。
        enum BenchFlow {
            TimedOut,
            Done(anyhow::Result<()>, usize),
        }
        let mut producer = producer;
        let flow = {
            let mut drain_fut = Box::pin(drain_reservoir(
                self.transport,
                &self.params,
                &self.session_id,
                &self.abort,
                &mut self.downlink_ts,
                &mut self.pacer,
                source,
                t0,
                &mut first_frame_at,
            ));
            let (race, drained) =
                race_synth(&mut producer, &mut drain_fut, &cancelled, deadline).await;
            match (race, drained) {
                (SynthRace::TimedOut, _) => {
                    filler.abort();
                    BenchFlow::TimedOut
                }
                (SynthRace::Producer(r), d) => {
                    // 合成已结束，剩余音频按节拍排空（不受超时预算约束）
                    let drain = match d {
                        Some(d) => d?,
                        None => drain_fut.await?,
                    };
                    filler.abort();
                    BenchFlow::Done(r, drain.frames)
                }
                (SynthRace::Drained, d) => {
                    // 下发已完（说明 producer 已发出 done），收尾 join producer
                    let r = producer
                        .await
                        .unwrap_or_else(|e| Err(anyhow::anyhow!("合成任务异常: {e}")));
                    let drain = d.expect("Drained 分支必带下发结果")?;
                    filler.abort();
                    BenchFlow::Done(r, drain.frames)
                }
            }
        };
        let (producer_res, frames) = match flow {
            BenchFlow::TimedOut => {
                tracing::warn!(
                    "session {session_id} 测试合成超时取消（{}ms）：{}",
                    t0.elapsed().as_millis(),
                    timeout_error_text()
                );
                send_text(
                    self.transport,
                    &ServerMessage::TtsTestResult {
                        session_id,
                        state: "error".to_string(),
                        engine: Some(engine_name),
                        text: Some(timeout_error_text()),
                    },
                )
                .await?;
                return Ok(());
            }
            BenchFlow::Done(r, frames) => (r, frames),
        };
        match producer_res {
            Ok(()) => {
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
            Err(e) => {
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

    /// `nlu_test`：按**磁盘最新配置**临时构建 TinyBERT 分类器直调（跳过 ASR/LLM），
    /// 结果回 `nlu_test`。带分词/推理耗时拆解；低于 `bert_threshold` 时 `intent=None`
    /// 并在 `text` 说明会放行 LLM（与技能级联行为一致）。
    pub(super) async fn handle_nlu_test(&mut self, text: String) -> anyhow::Result<()> {
        let session_id = self.session_id.clone();
        // 测试台专用端点：仅 hello.test=true 的会话受理。
        if !self.is_test {
            tracing::warn!("session {session_id} 非测试会话收到 nlu_test，忽略");
            return Ok(());
        }
        if text.trim().is_empty() {
            return Ok(());
        }
        // 按**磁盘上最新配置**构建分类器——改完 `[nlu].bert_*` 配置无需重启即可验证；
        // 未指定配置文件（内置默认）或读盘失败时退回启动时的内存配置。
        let nlu_cfg = if self.engines.config_path().is_none() {
            self.engines.config.nlu.clone() // 内置默认配置：无来源可读
        } else {
            self.engines
                .config_store
                .load()
                .map(|c| c.nlu)
                .unwrap_or_else(|e| {
                    tracing::warn!("session {session_id} NLU 测试读配置失败（用启动配置）: {e}");
                    self.engines.config.nlu.clone()
                })
        };
        // 构建失败原样回测试台（含怎么修文案），方便当场定位模型/词表路径问题。
        let bert = match build_bert_checked(&nlu_cfg) {
            Ok(bert) => bert,
            Err(e) => {
                tracing::warn!("session {session_id} NLU 测试构建失败: {e:#}");
                send_text(
                    self.transport,
                    &ServerMessage::NluTestResult {
                        session_id,
                        state: "error".to_string(),
                        intent: None,
                        confidence: None,
                        tokenize_ms: None,
                        infer_ms: None,
                        text: Some(format!("{e:#}")),
                    },
                )
                .await?;
                return Ok(());
            }
        };
        // ort 推理是同步阻塞调用，隔离出 async 执行器。
        let input = text.clone();
        let result = tokio::task::spawn_blocking(move || bert.classify_timed(&input))
            .await
            .map_err(|e| anyhow::anyhow!("NLU 测试任务异常: {e}"))?;
        match result {
            Ok(hit) => {
                let below = hit.confidence < nlu_cfg.bert_threshold;
                let text = if below {
                    Some(format!(
                        "最高置信度 {:.3} 低于阈值 {}，技能级联会放行 LLM",
                        hit.confidence, nlu_cfg.bert_threshold
                    ))
                } else {
                    Some(nlu_cfg.bert_map_label(&hit.label).to_string())
                };
                tracing::info!(
                    "session {session_id} NLU 测试完成：label={} conf={:.3}（分词 {}ms + 推理 {}ms）",
                    hit.label,
                    hit.confidence,
                    hit.tokenize_ms,
                    hit.infer_ms
                );
                send_text(
                    self.transport,
                    &ServerMessage::NluTestResult {
                        session_id,
                        state: "ok".to_string(),
                        intent: (!below).then(|| nlu_cfg.bert_map_label(&hit.label).to_string()),
                        confidence: Some(hit.confidence),
                        tokenize_ms: Some(hit.tokenize_ms),
                        infer_ms: Some(hit.infer_ms),
                        text,
                    },
                )
                .await?;
            }
            Err(e) => {
                tracing::warn!("session {session_id} NLU 测试推理失败: {e:#}");
                send_text(
                    self.transport,
                    &ServerMessage::NluTestResult {
                        session_id,
                        state: "error".to_string(),
                        intent: None,
                        confidence: None,
                        tokenize_ms: None,
                        infer_ms: None,
                        text: Some(format!("{e:#}")),
                    },
                )
                .await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
