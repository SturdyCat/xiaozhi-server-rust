//! 会话回复的生成与下发：LLM 流式 → 分句 → 合成 → 下行（含失败兜底话术）。
//!
//! 从 `session.rs` 拆出（见 `AGENTS.md` §5.10 文件规模约定）。`Session` 定义在父模块，
//! **子模块可直接访问其私有字段**，因此拆分无需放宽任何可见性。

use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::app::downlink::{
    poll_abort, send_cached_sentence_audio, send_sentence_audio_stream, send_tts_state,
};
use crate::app::protocol::ServerMessage;
use crate::app::splitter::SentenceSplitter;
use crate::app::transport::Transport;
use crate::app::tts_cache::TtsCache;
use crate::plugins::llm::LlmEvent;
use crate::plugins::prompt::TurnPrompt;

use super::{send_text, Session, FALLBACK_REPLY};

impl<'a, T: Transport> Session<'a, T> {
    /// 组装本轮 prompt：人格（`[soul]`）+ 记忆召回（`[memory]`，关键路径**硬预算**）。
    ///
    /// - 配置来自**会话级最新快照**（会话开始时热刷新），因此语义与 TTS/LLM 一致：
    ///   "保存后新会话生效"；
    /// - 人格组装失败（例如旧配置里写了未知变量）**不打断对话**：记日志并退回
    ///   `[llm].system_prompt`（保存期/启动期已校验，这是兜底）；
    /// - 记忆召回自带预算与超时降级，返回 `None` 就是"本轮无记忆"，绝不会抛出来。
    pub(super) async fn build_turn_prompt(&mut self, user_text: &str) -> TurnPrompt {
        let cfg = self.engines.live_config();
        let instructions = match crate::plugins::soul::instructions(&cfg, &self.device_id) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(
                    "session {} 人格组装失败（退回 [llm].system_prompt）: {e:#}",
                    self.session_id
                );
                cfg.llm.system_prompt.clone()
            }
        };
        let mut memory = None;
        if cfg.memory.recall_enabled {
            let scope = cfg.memory.scope_for(&self.device_id);
            let r = self.engines.memory().recall(Some(&scope), user_text).await;
            if let Some(err) = &r.error {
                tracing::warn!("session {} 记忆召回降级: {err}", self.session_id);
            }
            if r.block.is_some() {
                tracing::info!(
                    "session {} 记忆召回命中 {} 条（{} 词项匹配，耗时 {}ms）",
                    self.session_id,
                    r.hits,
                    r.matched_terms,
                    r.elapsed_ms
                );
            }
            memory = r.block;
        }
        TurnPrompt {
            instructions,
            memory,
            position: cfg.memory.inject_position(),
        }
    }

    /// 把本轮对话交给记忆插件（**后台任务**：写入原文 + 抽取 + 周期维护）。
    ///
    /// 为什么放在后台：抽取是一次真实 LLM 调用，放在关键路径会拖长 TTFA 并占用远端额度。
    /// 写原文本身很快，但同样没必要让用户等——两者一起 `spawn`，失败只记日志
    ///（记忆坏了不能让对话坏）。
    pub(super) fn spawn_memory_write(&self, user_text: &str, assistant_text: &str) {
        let cfg = self.engines.live_config();
        if !cfg.memory.active() {
            return;
        }
        let memory = self.engines.memory();
        let turn = crate::plugins::memory::CompletedTurn {
            scope: cfg.memory.scope_for(&self.device_id),
            session_id: self.session_id.clone(),
            user_text: user_text.to_string(),
            assistant_text: assistant_text.to_string(),
        };
        let sid = self.session_id.clone();
        tokio::spawn(async move {
            if let Err(e) = memory.record(turn).await {
                tracing::warn!("session {sid} 记忆写入失败（不影响对话）: {e:#}");
                return;
            }
            let o = memory.extract_pending(1).await;
            if o.attempted > 0 {
                tracing::info!(
                    "session {sid} 记忆抽取：尝试 {} / 成功 {} / 失败 {}",
                    o.attempted,
                    o.succeeded,
                    o.failed
                );
            }
            match memory.maintain(crate::plugins::memory::MaintainOpts::periodic()).await {
                Ok(m) if m.ran => tracing::info!(
                    "session {sid} 记忆维护：隔离释放 {} / 保留策略处理 {}（dry_run={}）",
                    m.released,
                    m.deleted,
                    m.dry_run
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!("session {sid} 记忆维护异常: {e:#}"),
            }
        });
    }
    /// 从用户文本开始：stt → LLM 流式按句切分 → 逐句 TTS + 逐帧下行（流水线重叠）。
    ///
    /// 生产者 task 读 LLM SSE 流并按句切分推入 channel；消费者（本 task）逐句
    /// `spawn_blocking` 合成 + 下发，读流与合成/下发**重叠**——首字音频延迟（TTFA）
    /// 从「整段串行」降到「首句」。abort 贯穿全链路：LLM 读流回调、合成间隙、
    /// 逐帧下行均轮询打断标志，置位即停。
    ///
    /// ⚠️ `pub(super)`：调用方在**父模块**（消息循环 `handle_text`）。父模块看不到子模块的私有项，
    /// 故此处放宽到 `pub(super)`（最小可见性，而非对外 `pub`）。
    pub(super) async fn stream_response(&mut self, user_text: &str) -> anyhow::Result<()> {
        let session_id = self.session_id.clone();
        self.abort.store(false, Ordering::Relaxed); // 新一轮对话重置打断标志
        send_text(
            self.transport,
            &ServerMessage::Stt {
                session_id: session_id.clone(),
                text: user_text.to_string(),
            },
        )
        .await?;
        // 早发 llm 状态：刷新设备 last_incoming_time_，避免 LLM 首字等待期被 ESP IsTimeout() 断连。
        // （协议约定 llm 消息仅做表情同步，回复正文走 tts.sentence_start。）
        send_text(
            self.transport,
            &ServerMessage::Llm {
                session_id: session_id.clone(),
                emotion: Some("neutral".to_string()),
                text: None,
            },
        )
        .await?;

        let t0 = std::time::Instant::now();
        // 本轮上下文：人格 + 记忆召回（**在 LLM 之前**，且召回失败不影响继续）
        let prompt = self.build_turn_prompt(user_text).await;
        // 生产者：读 LLM 流 → 按句切分 → channel。
        // unbounded：文本量级小（KB），无需反压；闭包借用 splitter/tx，流结束后仍可 flush。
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let producer = {
            // 本轮固定用"当前生效"的 LLM 引擎（读锁后克隆 Arc）：热切换发生在会话之间，
            // 不会在一轮对话中途换掉引擎。
            let llm = self.engines.llm();
            let abort = self.abort.clone();
            let history = self.history.clone();
            let user = user_text.to_string();
            tokio::spawn(async move {
                let mut splitter = SentenceSplitter::new();
                let result = llm
                    .chat_stream(
                        &history,
                        &user,
                        &prompt,
                        None,
                        Box::new(|event| {
                            if abort.load(Ordering::Relaxed) {
                                return false; // 设备打断：停止读流
                            }
                            if let LlmEvent::Text(delta) = event {
                                for s in splitter.feed(&delta) {
                                    let _ = tx.send(s);
                                }
                            }
                            true
                        }),
                    )
                    .await;
                for s in splitter.flush() {
                    let _ = tx.send(s);
                }
                result
            })
        };

        // 消费者：tts start → 逐句（sentence_start + spawn_blocking TTS + 逐帧下行）→ tts stop
        // 新一轮回复：节拍器时间表重置（本回复内各句共享同一时间表 → 句间无缝）
        self.pacer.reset();
        // 首帧实际下发时刻（TTFA 观测用；pacing 后"句子下发完成"≠首帧出声时刻）
        let mut first_frame_at: Option<std::time::Instant> = None;
        let mut tts_started = false;
        let mut interrupted = false;
        let mut sentences_done = 0usize;
        let mut last_ping = std::time::Instant::now();
        loop {
            // 200ms 粒度等待下一句：等待间隙轮询打断 + 定期 ping 保活
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Some(sentence)) => {
                    if poll_abort(self.transport, &self.session_id, &self.abort) {
                        interrupted = true;
                        break;
                    }
                    if !tts_started {
                        send_tts_state(self.transport, &self.session_id, "start").await?;
                        tts_started = true;
                    }
                    let frames = self.speak_sentence(&sentence, &mut first_frame_at).await?;
                    sentences_done += 1;
                    if sentences_done == 1 {
                        // TTFA = 用户文本就绪 → **首帧音频实际下发**（非句子播完）
                        let ttfa_ms = first_frame_at
                            .map(|t| t.duration_since(t0).as_millis())
                            .unwrap_or_else(|| t0.elapsed().as_millis());
                        tracing::info!(
                            "session {session_id} 首帧音频已下发（TTFA≈{ttfa_ms}ms，本句 {frames} 帧）"
                        );
                    }
                    if poll_abort(self.transport, &self.session_id, &self.abort) {
                        interrupted = true;
                        break;
                    }
                }
                Ok(None) => break, // 生产者结束（tx 已 drop）
                Err(_elapsed) => {
                    // 等待超时：轮询打断 + 长静默期 Ping 保活（tungstenite 自动回 Pong）
                    if poll_abort(self.transport, &self.session_id, &self.abort) {
                        interrupted = true;
                        break;
                    }
                    if last_ping.elapsed() >= Duration::from_secs(10) {
                        last_ping = std::time::Instant::now();
                        self.transport.keepalive().await;
                    }
                }
            }
        }
        if tts_started {
            // 打断时也发 stop：设备据此立即停止播放并清理队列
            send_tts_state(self.transport, &self.session_id, "stop").await?;
        }

        // 汇总 LLM 结果；仅未打断且流正常结束时压入历史，避免脏历史
        let llm_out = match producer.await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                // 具名分类：日志/上报/后续路由都按分类走，而不是解析错误串
                let failure = crate::plugins::llm::LlmFailure::classify(&e);
                tracing::warn!(
                    "session {session_id} LLM 调用失败（{}，耗时 {}ms）: {e} —— {}",
                    failure.category(),
                    t0.elapsed().as_millis(),
                    failure.hint()
                );
                // ⚠️ 语音场景**绝不能静默**：用户听不到任何声音 = "AI 没反应"（最差体验，
                // 也是此前运行期失败全部只 warn! 的老问题）。一个字都还没下发时，用一句
                // 兜底话术回应；已经出过声就不再补充（避免"说完半句又道歉"）。
                if !interrupted && !tts_started {
                    self.speak_fallback(FALLBACK_REPLY).await?;
                }
                return Ok(());
            }
            Err(e) => {
                tracing::warn!("session {session_id} LLM 任务异常: {e}");
                if !interrupted && !tts_started {
                    self.speak_fallback(FALLBACK_REPLY).await?;
                }
                return Ok(());
            }
        };
        // 令牌计量：回答"成本多少 / 记忆与灵魂让上下文长了多少"
        self.engines.usage.record(&session_id, llm_out.usage);
        tracing::info!(
            "session {session_id} 本轮完成：LLM 流 {}ms / {} 字 / {} 个工具调用，下发 {sentences_done} 句{}",
            t0.elapsed().as_millis(),
            llm_out.text.chars().count(),
            llm_out.tool_calls.len(),
            if interrupted { "（被打断）" } else { "" }
        );
        if !interrupted && !llm_out.text.trim().is_empty() {
            self.history
                .push(("user".to_string(), user_text.to_string()));
            self.history
                .push(("assistant".to_string(), llm_out.text.clone()));
            // 按 [llm].max_history 截断多轮历史，只保留最近 N 条（role, content）。
            let max_history = self.engines.live_config().llm.max_history;
            while self.history.len() > max_history {
                self.history.remove(0);
            }
            // 记忆写入：**仅未被打断且回复非空**（打断的半句没有记忆价值，且会污染事实源）
            self.spawn_memory_write(user_text, &llm_out.text);
        }
        Ok(())
    }

    /// 用一句兜底话术回应 LLM 失败（拔网 / 限流 / 超时），**绝不静默**。
    ///
    /// 复用 [`Self::speak_canned`] 的完整下行链路（结果缓存 / 节拍器 / 首帧时刻），
    /// 因此兜底话术的听感与正常回复完全一致；失败只记日志，不再向上抛
    ///（兜底本身失败时向上抛错误没有意义，会话该结束还是结束）。
    async fn speak_fallback(&mut self, text: &str) -> anyhow::Result<()> {
        self.speak_canned(text).await?;
        tracing::info!("session {} 已下发兜底话术", self.session_id);
        Ok(())
    }

    /// 主动说一句**非 LLM 生成**的话（LLM 失败兜底 / 指令闸门的告别语）。
    ///
    /// 抽成方法是为了让"必须由本地下行链路说出的一句话"只有一个实现：tts start →
    /// 逐句合成下发 → tts stop，缓存/节拍器/打断语义与正常回复完全一致。
    /// 合成失败只记日志（调用方各自决定后续行为：兜底继续返回、指令闸门照常断开）。
    pub(super) async fn speak_canned(&mut self, text: &str) -> anyhow::Result<()> {
        // 一句独立的话 = 一次独立回复：节拍器时间表重置（指令闸门路径不会经过
        // `stream_response` 的 reset，不重置会沿用上一轮的时间锚点）。
        self.pacer.reset();
        send_tts_state(self.transport, &self.session_id, "start").await?;
        let mut first_frame_at: Option<std::time::Instant> = None;
        if let Err(e) = self.speak_sentence(text, &mut first_frame_at).await {
            tracing::warn!("话术下发失败: {e}");
        }
        send_tts_state(self.transport, &self.session_id, "stop").await?;
        Ok(())
    }

    /// 合成**一句**并下行（含结果缓存命中路径），返回下发帧数。
    ///
    /// 抽成方法是为了让 LLM 失败时的**兜底话术**复用同一条经过验证的下行链路
    /// （结果缓存 / 节拍器 / 首帧时刻 / 打断语义全都一致），而不是另写一份简化实现。
    async fn speak_sentence(
        &mut self,
        sentence: &str,
        first_frame_at: &mut Option<std::time::Instant>,
    ) -> anyhow::Result<usize> {
        let session_id = self.session_id.clone();
        // 流式合成：spawn_blocking 里跑引擎（同步阻塞），分片经 channel 回流；
        // 本 task 边收边重采样/编码/下发——首片即发声（TTFA 从「整句合成完」提前到
        // 「首片合成完」）。abort 时回调返回 false，引擎尽早停止合成。
        // 缓存命中（同一句已合成过：引擎签名+音色+语速+下行参数+文本同键）跳过合成，
        // 已编码帧直接按节拍下发——高频短句 TTFA 降到一次预充突发。
        // 会话级最新快照（热刷新后的值）：发音人/语速/引擎签名都要跟着变，
        // 否则切引擎后缓存键仍是旧签名，可能命中上一个引擎的音频。
        let live = self.engines.live_config();
        let speed = live.tts.speed;
        let speaker = live.tts.speaker;
        let cache_key = TtsCache::make_key(
            // 引擎签名口径的唯一权威在注册表（含密钥字段，仅内存比较、不打日志）
            &crate::plugins::registry::signature_of(
                &live,
                crate::plugins::registry::Capability::Tts,
            ),
            speaker,
            speed,
            self.params.downlink_sr,
            self.params.downlink_frame_ms,
            &sentence,
        );
        let frames = if let Some(cached) = self.engines.tts_cache.get(cache_key) {
            tracing::debug!(
                "session {session_id} TTS 缓存命中（{} 帧直接下发）",
                cached.opus_frames.len()
            );
            send_cached_sentence_audio(
                self.transport,
                &self.params,
                &mut self.downlink_ts,
                &self.abort,
                &self.session_id,
                sentence,
                &cached.opus_frames,
                &mut self.pacer,
                first_frame_at,
            )
            .await?
        } else {
            let tts = self.engines.tts();
            let tts_sr = tts.output_sample_rate();
            let t_tts = std::time::Instant::now();
            let s = sentence.to_string();
            let (tx, chunk_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<f32>>();
            let producer = tokio::task::spawn_blocking(move || {
                tts.synthesize_stream(
                    &s,
                    speed,
                    speaker,
                    Box::new(move |_sr, chunk| tx.send(chunk.to_vec()).is_ok()),
                )
            });
            let mut cache_buf: Vec<Vec<u8>> = Vec::new();
            let frames = send_sentence_audio_stream(
                self.transport,
                &self.params,
                &mut self.downlink_ts,
                &self.abort,
                &self.session_id,
                sentence,
                tts_sr,
                chunk_rx,
                &mut self.pacer,
                Some(&mut cache_buf),
                first_frame_at,
            )
            .await?;
            let synth_result = producer.await;
            match synth_result {
                Ok(Ok(())) => {
                    tracing::debug!(
                        "session {session_id} 句子合成完成（耗时 {}ms）",
                        t_tts.elapsed().as_millis()
                    );
                    // 完整合成成功才入缓存（打断/合成失败的残缺句不缓存）
                    if !cache_buf.is_empty() {
                        self.engines.tts_cache.insert(cache_key, cache_buf);
                    }
                }
                Ok(Err(e)) => tracing::warn!(
                    "session {session_id} 句子合成失败（耗时 {}ms）: {e}",
                    t_tts.elapsed().as_millis()
                ),
                Err(e) => tracing::warn!("session {session_id} 合成任务异常: {e}"),
            }
            frames
        };
        Ok(frames)
    }
}
