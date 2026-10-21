//! AIUI 全链路在会话层的落点：切段即轮次（识别+大模型+合成在 AIUI 云端闭环），
//! 回传音频复用现有下行链路（水库 + 节拍器）。
//!
//! 从 `session.rs` 拆出（缺口②实施时 session.rs 行数逼近 600 上限，AGENTS.md §5.10：
//! 行为不变的搬移）。`Session` 定义在父模块，子模块可直接访问其私有字段。

use std::sync::Arc;

use crate::app::downlink::{
    drain_reservoir, new_reservoir, send_tts_state, spawn_single_sentence_filler,
};
use crate::app::prefetch::PreparedAudio;
use crate::app::protocol::ServerMessage;
use crate::app::transport::Transport;
use crate::plugins::aiui::FullChainTurn;

use super::{blocking_result, f32_to_i16_16k, send_text, Session, UtteranceDecision};

impl<'a, T: Transport> Session<'a, T> {
    /// AIUI 全链路：每段 = 一轮 oneshot 交互（识别 + 大模型 + 合成在 AIUI 云端闭环）。
    /// stt/nlp 文本照常走 stt 消息与会话历史；合成音频（16k PCM）复用现有下行链路。
    ///
    /// 分段延迟（缺口②）：anchor=`aiui`（段落起点），只采 `first_audio_ms`
    /// ——识别与合成都发生在云端闭环内，本地无中间里程碑可标。
    pub(super) async fn aiui_segments(&mut self, segments: Vec<Vec<f32>>) {
        let session_id = self.session_id.clone();
        for seg in segments {
            let trace = crate::app::events::TurnTrace::new(crate::app::events::ANCHOR_AIUI);
            let turn = trace.turn_id();
            let pcm_i16 = f32_to_i16_16k(&seg, self.params.uplink_sr);
            let Some(aiui) = self.aiui.as_ref().map(Arc::clone) else {
                continue;
            };
            tracing::debug!(turn, "session {session_id} AIUI 轮次开始（锚点 aiui）");
            let t0 = std::time::Instant::now();
            let res = blocking_result(move || {
                let mut g = aiui.lock().unwrap_or_else(|e| e.into_inner());
                g.turn(&pcm_i16)
            })
            .await;
            let turn_res = match res {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(turn, "session {session_id} AIUI 轮次失败: {e}");
                    continue;
                }
            };
            if turn_res.stt.trim().is_empty() {
                continue; // 静音轮（云端 Silence 事件），同空识别结果处理
            }
            tracing::info!(
                turn,
                "session {session_id} AIUI 轮次完成：耗时 {}ms（识别 {} 字 / 回复 {} 字 / 音频 {} 字节）",
                t0.elapsed().as_millis(),
                turn_res.stt.chars().count(),
                turn_res.reply.chars().count(),
                turn_res.audio.len()
            );
            // 文本侧优先级管线（P0-4）：与级联流水线共用统一入口（记录 + 规则 NLU）。
            // 识别文本要等云端闭环回来才可见（本地没有 ASR 可拦），因此命中意图时只能
            // "丢弃本轮云端回复"—但会话照样按意图处置（语义与级联流水线一致）。
            match self.decide_user_text(&turn_res.stt, turn).await {
                UtteranceDecision::NluStop => return,
                UtteranceDecision::Respond => {
                    if let Err(e) = self.aiui_downlink(&turn_res, &trace).await {
                        tracing::warn!(turn, "session {session_id} AIUI 下行失败: {e}");
                    }
                }
            }
        }
    }

    /// AIUI 轮次结果下发：stt 文本 → 水库（重采样 + Opus 分帧）→ 按实时节拍下发 → `tts stop`。
    ///
    /// 与级联流水线（`reply::drain_prepared`）走**同一条**下行链路：同一节拍器、同一水库
    /// （服务器内存缓冲 + 起跑闸）、同一 `tts start` 起跑缓冲语义——只在"音频从哪来"
    /// 上不同（云端一次性回传的 PCM，而不是逐句流式合成）。
    ///
    /// `trace`（缺口②）：首轮首帧实际下发时 mark `first_audio_ms` 并在本轮结束 emit。
    async fn aiui_downlink(
        &mut self,
        turn: &FullChainTurn,
        trace: &Arc<crate::app::events::TurnTrace>,
    ) -> anyhow::Result<()> {
        self.pacer.reset(); // 新一轮回复：节拍器时间表重置
        send_text(
            self.transport,
            &ServerMessage::Stt {
                session_id: self.session_id.clone(),
                text: turn.stt.clone(),
            },
        )
        .await?;
        // i16 LE 字节 → f32 分片（3200 字节 = 1600 样本 = 100ms @16k）
        let chunks: Vec<Vec<f32>> = turn
            .audio
            .chunks(3200)
            .map(|chunk| {
                chunk
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| i16::from_le_bytes(*c) as f32 / 32768.0)
                    .collect()
            })
            .collect();
        // 有界分片通道（容量与内存口径同 `prefetch` 的一次性回传路径）：用后台任务喂，
        // 避免在水库还没被消费时把会话任务阻塞在 `send` 上。
        // ⚠️ AIUI 是**一次性回传**（整段音频已在 `turn.audio` 里），没有"流式下载"可限速，
        // 故分片不占下载缓冲（`PcmChunk::unbudgeted`）——下载缓冲只服务逐句流式合成。
        let (tx, chunk_rx) = tokio::sync::mpsc::channel::<crate::app::prefetch::PcmChunk>(
            crate::app::prefetch::PREFETCH_PCM_CHUNKS,
        );
        tokio::spawn(async move {
            for c in chunks {
                if tx
                    .send(crate::app::prefetch::PcmChunk::unbudgeted(c))
                    .await
                    .is_err()
                {
                    break; // 水库读端已放弃（打断/本轮结束）
                }
            }
        });
        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<bool>();
        let _ = done_tx.send(true); // 云端音频已完整在手：视为"合成正常结束"
        let (sink, source) = new_reservoir(
            self.params.downlink_buffer_ms,
            self.params.downlink_frame_ms,
        );
        let filler = spawn_single_sentence_filler(
            self.abort.clone(),
            self.engines.tts_cache.clone(),
            self.params,
            sink,
            turn.stt.clone(),
            PreparedAudio::Stream {
                // AIUI 全链路回传音频的采样率 = 请求参数（24k），与级联 TTS 同源常量
                sr: crate::plugins::aiui::TTS_SAMPLE_RATE,
                chunks: chunk_rx,
                done: done_rx,
            },
            0, // AIUI 全链路回传音频不入缓存（音频由云端给定，同句不保证可复现）
        );
        let mut first_frame_at = None;
        let t0 = std::time::Instant::now();
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
        // 缺口②：首帧实际下发 → mark。⚠️ 传 sender 记下的**首帧时刻**（drain 已返回，
        // 用"现在"记会失真成整轮耗时）；本轮结束 emit（interrupted 与级联同口径）。
        trace.mark_first_audio_at(first_frame_at);
        self.emit_turn_latency(trace, drain.interrupted);
        self.log_turn_end(trace, drain.interrupted);
        if drain.tts_started {
            send_tts_state(self.transport, &self.session_id, "stop").await?;
        }
        // 历史语义与级联流水线一致：被打断（下行早停）不入历史，避免脏上下文
        if !drain.interrupted && !turn.reply.trim().is_empty() {
            self.history.push(("user".to_string(), turn.stt.clone()));
            self.history
                .push(("assistant".to_string(), turn.reply.clone()));
            let max_history = self.eff_config.llm.max_history;
            while self.history.len() > max_history {
                self.history.remove(0);
            }
        }
        Ok(())
    }
}
