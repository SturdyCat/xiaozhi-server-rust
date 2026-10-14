//! 会话回复的生成与下发：LLM 流式 → 分句 → 合成 → 下行（含失败兜底话术）。
//!
//! 从 `session.rs` 拆出（见 `AGENTS.md` §5.10 文件规模约定）。`Session` 定义在父模块，
//! **子模块可直接访问其私有字段**，因此拆分无需放宽任何可见性。
//!
//! LLM 生产者（读流 + 分句 + 定时 flush watchdog）进一步抽到 [`producer`]——
//! 本文件只保留编排层（prompt 组装 / 历史与计量 / 兜底与观测）。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::app::downlink::{
    drain_reservoir, new_reservoir, send_tts_state, spawn_reservoir_filler,
};
use crate::app::events::{TurnId, TurnTrace};
use crate::app::prefetch::{PreparedSentence, SentenceSource};
use crate::app::protocol::ServerMessage;
use crate::app::transport::Transport;
use crate::plugins::prompt::TurnPrompt;

use super::producer::{LlmProducer, spawn_llm_producer};
use super::{Session, FALLBACK_REPLY, send_text};

/// 一次回复的下行结果（供日志与兜底判定）。
#[derive(Debug, Default, Clone, Copy)]
struct DownlinkOutcome {
    /// 已下发句数（含缓存命中句）
    sentences: usize,
    /// 已下发音频帧数（整轮）：`frames × 帧长` = 本轮音频时长（与整轮耗时对比判节流）
    frames: usize,
    /// 是否被打断（abort / 承载断开）
    interrupted: bool,
}

impl<'a, T: Transport> Session<'a, T> {
    /// 组装本轮 prompt：人格（`[soul]`）+ 记忆召回（`[memory]`，关键路径**硬预算**）。
    ///
    /// - 配置来自**会话级最新快照**（会话开始时热刷新），因此语义与 TTS/LLM 一致：
    ///   "保存后新会话生效"；
    /// - 人格组装失败（例如旧配置里写了未知变量）**不打断对话**：记日志并退回
    ///   `[llm].system_prompt`（保存期/启动期已校验，这是兜底）；
    /// - 记忆召回自带预算与超时降级，返回 `None` 就是"本轮无记忆"，绝不会抛出来。
    pub(super) async fn build_turn_prompt(&mut self, user_text: &str) -> TurnPrompt {
        let cfg = self.eff_config.clone();
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
        let cfg = self.eff_config.clone();
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
                Err(e) => tracing::warn!("session {sid} 记忆维护异常: {e}"),
            }
        });
    }

    /// 从用户文本开始：stt → LLM 流式按句切分 → 逐句 TTS + 逐帧下行（流水线重叠）。
    ///
    /// 生产者 task 读 LLM SSE 流并按句切分推入 channel（含定时 flush watchdog，缺口③，
    /// 见 [`producer`]）；消费者（本 task）逐句合成 + 下发，读流与合成/下发**重叠**。
    /// abort 贯穿全链路：LLM 读流回调、合成间隙、逐帧下行均轮询打断标志，置位即停。
    ///
    /// 分段延迟（缺口②）：`trace` 由调用方创建（锚点 = speech_end / inject），
    /// producer 回调 mark 首 token / 首句，prefetch 回调 mark 首个 PCM，
    /// [`Self::drain_prepared`] mark 首帧下发，各结束分支 emit。
    ///
    /// ⚠️ `pub(super)`：调用方在**父模块**（消息循环）。父模块看不到子模块的私有项，
    /// 故此处放宽到 `pub(super)`（最小可见性，而非对外 `pub`）。
    pub(super) async fn stream_response(
        &mut self,
        user_text: &str,
        trace: &Arc<TurnTrace>,
    ) -> anyhow::Result<()> {
        let session_id = self.session_id.clone();
        let turn = trace.turn_id();
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
        // 定时 flush 等待（缺口③）：每轮从生效配置现读（0 = 关闭）。
        // ⚠️ 仅**全局**生效：`llm.sentence_flush_ms` 不在设备 overlay 白名单
        // （`config/overlay.rs` 的 llm 段只放行 system_prompt/max_history）。
        let flush_wait = Duration::from_millis(self.eff_config.llm.sentence_flush_ms as u64);
        // 生产者：读 LLM 流 → 按句切分（+ watchdog）→ channel
        let LlmProducer { sentences: rx, result: producer } = spawn_llm_producer(
            self.engines.llm(),
            self.history.clone(),
            user_text.to_string(),
            prompt,
            self.abort.clone(),
            super::producer::ProducerTurn { trace: trace.clone(), session_id: session_id.clone() },
            flush_wait,
        );

        // 预取：句子 → 成品（下一句的合成跑在**本句播放期间**，消除句间合成气泡——
        // 在线合成每次调用都是一条短连接，旧串行实现每句边界都要重付一次握手延迟）
        let (prepared, download) = self.spawn_prefetch(rx, trace);
        // 观测：进入"说话中"（管理页「会话」的实时阶段）。整轮只置一次——
        // 本轮结束（成功或兜底）会由 `note_assistant_turn` 翻回"空闲"。
        self.note_phase(crate::app::registry::Phase::Speaking);
        // 消费者：水库按实时节拍抽取下发（`tts start` 在首项到达时才发）→ 整轮结束 `tts stop`
        // 新一轮回复：节拍器时间表重置（本回复内各句共享同一时间表 → 句间无缝）
        self.pacer.reset();
        let outcome = self.drain_prepared(prepared, download, t0, trace).await?;
        let DownlinkOutcome {
            sentences: sentences_done,
            frames: frames_total,
            interrupted,
            ..
        } = outcome;

        // 汇总 LLM 结果；仅未打断且流正常结束时压入历史，避免脏历史
        let llm_out = match producer.await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                // 具名分类：日志/上报/后续路由都按分类走，而不是解析错误串
                let failure = crate::plugins::llm::LlmFailure::classify(&e);
                tracing::warn!(
                    turn,
                    "session {session_id} LLM 调用失败（{}，耗时 {}ms）: {e} —— {}",
                    failure.category(),
                    t0.elapsed().as_millis(),
                    failure.hint()
                );
                // ⚠️ 语音场景**绝不能静默**：用户听不到任何声音 = "AI 没反应"（最差体验，
                // 也是此前运行期失败全部只 warn! 的老问题）。一个字都还没下发时，用一句
                // 兜底话术回应；已经出过声就不再补充（避免"说完半句又道歉"）。
                self.note_error(format!("LLM 调用失败（{}）", failure.category()));
                self.engines.event_bus.emit(crate::app::events::ServerEvent::Error {
                    session_id: self.session_id.clone(),
                    device_id: self.device_id.clone(),
                    message: format!("LLM 调用失败（{}）", failure.category()),
                });
                // 分段延迟：失败轮照样 emit（锚点轮次发生了就是发生了）
                self.emit_turn_latency(trace, interrupted);
                self.log_turn_end(trace, interrupted);
                if !interrupted && frames_total == 0 {
                    self.speak_fallback(FALLBACK_REPLY, turn).await?;
                }
                // 兜底话术也是"说过的一句"：记进对话，避免界面出现"用户问了却没有任何回复"
                self.note_assistant_turn(FALLBACK_REPLY, t0.elapsed().as_millis() as u64);
                self.emit_message_added(
                    turn,
                    "assistant",
                    FALLBACK_REPLY,
                    Some(t0.elapsed().as_millis() as u64),
                );
                return Ok(());
            }
            Err(e) => {
                tracing::warn!(turn, "session {session_id} LLM 任务异常: {e}");
                self.note_error(format!("LLM 任务异常: {e}"));
                self.engines.event_bus.emit(crate::app::events::ServerEvent::Error {
                    session_id: self.session_id.clone(),
                    device_id: self.device_id.clone(),
                    message: format!("LLM 任务异常: {e}"),
                });
                self.emit_turn_latency(trace, interrupted);
                self.log_turn_end(trace, interrupted);
                if !interrupted && frames_total == 0 {
                    self.speak_fallback(FALLBACK_REPLY, turn).await?;
                }
                self.note_assistant_turn(FALLBACK_REPLY, t0.elapsed().as_millis() as u64);
                self.emit_message_added(
                    turn,
                    "assistant",
                    FALLBACK_REPLY,
                    Some(t0.elapsed().as_millis() as u64),
                );
                return Ok(());
            }
        };
        // 令牌计量：回答"成本多少 / 记忆与灵魂让上下文长了多少"。
        // ⚠️ 取消轮（`cancelled`）**照常计量**：上游已收款，统计不能因打断而丢。
        self.engines.usage.record(&session_id, llm_out.usage);
        // 观测：这一轮"问了什么、答了什么、整轮多久"（管理页「对话记录」）。
        // 刻意**不**受 `interrupted` 影响：被打断的这轮同样是发生过的一轮，
        // 记录里丢掉它会让界面上的对话与用户记忆对不上。
        self.note_assistant_turn(&llm_out.text, t0.elapsed().as_millis() as u64);
        // 事件总线（P0-5 叠加层）：助手整轮回复入对话（与 note_assistant_turn 并列）。
        // `turn_id` 与本轮用户消息同一个值——"问了什么"与"这一轮慢在哪"由此可拼。
        self.emit_message_added(
            turn,
            "assistant",
            &llm_out.text,
            Some(t0.elapsed().as_millis() as u64),
        );
        // 分段延迟（缺口②）：正常结束的整轮 emit
        self.emit_turn_latency(trace, interrupted);
        self.log_turn_end(trace, interrupted);
        // ⚠️ 这里打印的是**整轮耗时**（t0 → 最后一帧下发完成），不是"LLM 流耗时"：
        // 旧文案写"LLM 流 Xms"会让人误读成 LLM 吞吐（曾据此误判上游速度）。
        // 与"音频 Xms"对比才是判断节流的正确口径：两者接近 = 全程实时无缝；
        // 耗时远大于音频 = 中间被供数拖住了（结合下面的队列余量日志定位是谁慢）。
        let audio_ms = frames_total as u64 * self.params.downlink_frame_ms.max(1) as u64;
        // 事件总线（P0-5 叠加层）：一轮 TTS 下行完成（句数/帧数/是否打断/音频时长，供大盘聚合）。
        self.engines.event_bus.emit(crate::app::events::ServerEvent::TtsRound {
            session_id: self.session_id.clone(),
            device_id: self.device_id.clone(),
            turn_id: turn,
            sentences: sentences_done as u32,
            frames: frames_total as u32,
            interrupted,
            audio_ms,
        });
        tracing::info!(
            turn,
            "session {session_id} 本轮完成：耗时 {}ms / 音频 {audio_ms}ms（{frames_total} 帧）/ LLM 输出 {} 字 / {} 个工具调用，下发 {sentences_done} 句{}",
            t0.elapsed().as_millis(),
            llm_out.text.chars().count(),
            //取消轮恒为 0（引擎按取消契约丢弃，见 `LlmTurnResult`）：这里只反映"真正要执行的工具调用"。
            llm_out.tool_calls.len(),
            if interrupted || llm_out.cancelled {
                "（被打断）"
            } else {
                ""
            }
        );
        if !interrupted && !llm_out.text.trim().is_empty() {
            self.history
                .push(("user".to_string(), user_text.to_string()));
            self.history
                .push(("assistant".to_string(), llm_out.text.clone()));
            // 按 [llm].max_history 截断多轮历史，只保留最近 N 条（role, content）。
            let max_history = self.eff_config.llm.max_history;
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
    ///
    /// `parent_turn` = 触发兜底的**那一轮**的 id：兜底话术自身是独立一轮（`speak_canned`
    /// 自建 trace），但这条日志属于"原轮次发生了什么"的因果链，故记原轮次的 id。
    async fn speak_fallback(&mut self, text: &str, parent_turn: TurnId) -> anyhow::Result<()> {
        self.speak_canned(text).await?;
        tracing::info!(turn = parent_turn, "session {} 已下发兜底话术", self.session_id);
        Ok(())
    }

    /// 主动说一句**非 LLM 生成**的话（LLM 失败兜底 / 指令闸门的告别语 / 唤醒问候语）。
    ///
    /// 走与正常回复**完全同一条**下行链路（预取 → 节拍器 → tts start/stop），
    /// 因此听感与打断语义一致；合成失败只记日志（调用方各自决定后续行为：
    /// 兜底继续返回、指令闸门照常断开）。
    ///
    /// 分段延迟（缺口②）：自建 trace（anchor=`canned`），只采 `first_audio_ms`
    /// ——预制话术没有 LLM/分句阶段；与正常回复各自成事件，靠 anchor 区分。
    pub(super) async fn speak_canned(&mut self, text: &str) -> anyhow::Result<()> {
        // 一句独立的话 = 一次独立回复：节拍器时间表重置（指令闸门路径不会经过
        // `stream_response` 的 reset，不重置会沿用上一轮的时间锚点）。
        self.pacer.reset();
        // 自包含：清掉上一轮遗留的打断标志。否则预取 worker 在入口即退出，
        // `tts start`/`stop` 都不发 —— 客户端/测试台会卡在"合成中"，用户则听到静音。
        // 指令闸门路径本来就会清（见 `command.rs`），这里给兜底路径补上同一语义。
        self.abort.store(false, Ordering::Relaxed);
        let trace = TurnTrace::new(crate::app::events::ANCHOR_CANNED);
        let turn = trace.turn_id();
        tracing::info!(turn, "session {} 下发预制话术（锚点 canned）: {text}", self.session_id);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let _ = tx.send(text.to_string());
        drop(tx); // 只有一句 → 关掉句子流，预取 worker 处理完即退出
        let (prepared, download) = self.spawn_prefetch(rx, &trace);
        let t0 = Instant::now();
        match self.drain_prepared(prepared, download, t0, &trace).await {
            Ok(o) if o.sentences == 0 => {
                tracing::warn!(turn, "session {} 话术未下发任何音频: {text}", self.session_id);
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(turn, "session {} 话术下发失败: {e}", self.session_id),
        }
        // 分段延迟：canned 轮 emit（anchor=canned，first_audio 之外的阶段为 None）
        self.emit_turn_latency(&trace, false);
        self.log_turn_end(&trace, false);
        Ok(())
    }

    /// 启动逐句**预取**（见 [`crate::app::prefetch`]）：句子流 → 成品流。
    ///
    /// 每句都按**会话级最新配置**解析引擎/音色/语速/引擎签名与缓存键——与旧实现的
    /// "句间热切换"语义一致（切引擎后不会命中上一个引擎的缓存音频）。
    ///
    /// 同时建出本轮的**下载缓冲**（[`DownloadBudget`]）：合成回调据此按预算暂停/恢复
    /// TTS 流下载（内存换性能 + 下载限速），返回值随行交给 [`Self::drain_prepared`]。
    ///
    /// 缺口①：TTS 闸门取自 `engines.inference`（进程级，与生产/测试台共用）。
    /// 缺口②：`trace` → first-only 的 `tts_first_pcm` 回调闭包（原子读，无锁）。
    fn spawn_prefetch(
        &self,
        sentences: tokio::sync::mpsc::UnboundedReceiver<String>,
        trace: &Arc<TurnTrace>,
    ) -> (
        tokio::sync::mpsc::Receiver<PreparedSentence>,
        Arc<crate::app::prefetch::DownloadBudget>,
    ) {
        let engines = self.engines.clone();
        let params = self.params;
        let eff_config = self.eff_config.clone();
        let download =
            crate::app::prefetch::DownloadBudget::new(self.params.download_buffer_ms);
        // 首个非缓存句的首个 PCM 分片 → mark_tts_first_pcm（first-only 用原子标志，
        // 避免每分片加锁；缓存命中句不走合成回调，天然不触发——与"缓存轮该阶段为
        // None"的口径一致）
        let pcm_once = Arc::new(AtomicBool::new(false));
        let trace_for_pcm = trace.clone();
        let first_pcm: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            if !pcm_once.swap(true, Ordering::Relaxed) {
                trace_for_pcm.mark_tts_first_pcm();
            }
        });
        let prepared = crate::app::prefetch::spawn_sentence_prefetch(
            self.abort.clone(),
            self.engines.tts_cache.clone(),
            download.clone(),
            self.engines.inference.tts(),
            Some(first_pcm),
            move |sentence: &str| {
                let live = eff_config.clone();
                // 缓存键口径的**唯一权威**在 `HotAudioKey::from_session`。
                //
                // 为什么不让本处自己拼 `TtsCache::make_key`：唤醒问候语路径要把**同一个键**
                // 的成品音频暖进本缓存（读盘复用），两处各写一遍参数顺序，一旦漂移就是
                // "缓存永远不命中"——不报错、只是白合成，是最难查的一类 bug。所以合成路径
                // 与问候路径共用这一个构造入口。
                let key = crate::app::hot_audio::HotAudioKey::from_session(
                    &live,
                    params.downlink_sr,
                    params.downlink_frame_ms,
                    sentence,
                );
                SentenceSource {
                    tts: engines.tts(),
                    cache_key: key.mem_key(),
                    speed: key.speed,
                    speaker: key.speaker,
                }
            },
            sentences,
        );
        (prepared, download)
    }

    /// 消费预取成品：水库填充 → 按实时节拍下发 → `tts stop`。
    ///
    /// 分工（见 `app/downlink` 模块文档）：
    /// - **填充任务**（[`spawn_reservoir_filler`]）在后台把每句合成/编码后的帧按序推入
    ///   **服务器内存水库**（`[audio].downlink_buffer_ms`，默认 30s），满则等待（背压）；
    /// - **本函数**（发送端）按实时节拍从水库取帧下发——上游停顿由水库存货吸收，
    ///   设备解码队列不再被抽干（这是"长回复断续"的正面解）。
    ///
    /// 缺口②：首帧实际下发（`first_frame_at`）→ `trace.mark_first_audio_at`。
    async fn drain_prepared(
        &mut self,
        prepared: tokio::sync::mpsc::Receiver<PreparedSentence>,
        download: Arc<crate::app::prefetch::DownloadBudget>,
        t0: Instant,
        trace: &Arc<TurnTrace>,
    ) -> anyhow::Result<DownlinkOutcome> {
        let session_id = self.session_id.clone();
        let turn = trace.turn_id();
        // 水库容量与起跑闸都取**会话级快照**（`[audio]` 是 restart_only，会话内不变）
        let (sink, source) =
            new_reservoir(self.params.downlink_buffer_ms, self.params.downlink_frame_ms);
        let filler = spawn_reservoir_filler(
            self.abort.clone(),
            self.engines.tts_cache.clone(),
            self.params,
            prepared,
            sink,
        );
        // 首帧实际下发时刻（TTFA 观测用；pacing 后"句子下发完成"≠首帧出声时刻）
        let mut first_frame_at: Option<Instant> = None;
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
        // 缺口②：首帧实际下发 → mark。⚠️ 必须传 sender 记下的**首帧时刻**（`first_frame_at`
        // out-param）：此刻 drain 已返回、末帧已发完，用"现在"记会失真成整轮耗时。
        trace.mark_first_audio_at(first_frame_at);
        // 收尾：正常路径填充任务已自行收工（写端丢弃）；打断/异常时立刻停掉，
        // 别再白合成、白编码本轮剩余音频。
        filler.abort();
        if drain.tts_started {
            // 打断时也发 stop：设备据此进入"排空 → 聆听"（**不会**清空解码队列，
            // 见 `downlink.rs` 的节拍器文档：残留量 ≈ 提前量）
            send_tts_state(self.transport, &self.session_id, "stop").await?;
        }
        self.log_downlink(&session_id, turn, &drain, &download, first_frame_at);
        Ok(DownlinkOutcome {
            sentences: drain.sentences,
            frames: drain.frames,
            interrupted: drain.interrupted,
        })
    }

    /// 每轮下行汇总日志：① 设备解码队列余量最低估计（断音现场证据）；
    /// ② 水库水位/背压/饥饿（上游供数速度——判断"该调大水库还是该治上游"）；
    /// ③ 下载缓冲水位/限速（TTS 流下载是否被预算压住）；
    /// ④ **本轮吞吐**（音频时长 / 播放窗口）：< 1.0× 即上游持续慢于实时。
    ///
    /// 四条全部带 `turn=`：它们是"这一轮慢在哪一段"的**下半程**证据（与 `observe.rs`
    /// 的 `log_turn_end` 上半程摘要配对），`grep turn=N` 一次捞全。
    fn log_downlink(
        &self,
        session_id: &str,
        turn: TurnId,
        drain: &crate::app::downlink::ReservoirDrain,
        download: &crate::app::prefetch::DownloadBudget,
        first_frame_at: Option<Instant>,
    ) {
        let stats = self.pacer.stats();
        let res = drain.reservoir;
        let frame_ms = self.params.downlink_frame_ms.max(1) as i64;
        let lead = self.params.downlink_lead_ms as i64;
        // 安全上限随设备队列包数与帧长而定（见 `[audio].downlink_queue_packets`）
        let cap = self.engines.live_config().audio.max_lead_ms();
        match stats.queue_min_ms {
            Some(depth) if depth < frame_ms => tracing::warn!(
                turn,
                "session {session_id} 本轮下行**断音**：设备解码队列余量最低估计 {depth}ms（< 1 帧 {frame_ms}ms）\
                 ，真空 {} 次（最长 {}ms），抖动缓冲 {lead}ms。——上游供数慢于播放（LLM 出句 / TTS 合成 RTF）\
                 或网络抖动；可上调 [audio].downlink_lead_ms（当前安全上限 {cap}ms，受设备解码队列包数×帧长限制，\
                 想更深必须先放大固件 MAX_DECODE_PACKETS_IN_QUEUE）、或调大 [audio].downlink_buffer_ms / \
                 downlink_buffer_start_ms（服务器侧水库），或降低合成耗时",
                stats.vacuum_events,
                stats.max_vacuum_ms
            ),
            Some(depth) => tracing::info!(
                turn,
                "session {session_id} 本轮下行平顺：设备解码队列余量最低估计 {depth}ms，真空 {} 次（最长 {}ms）",
                stats.vacuum_events,
                stats.max_vacuum_ms
            ),
            None => tracing::debug!(turn, "session {session_id} 本轮下行过短，未采集队列余量"),
        }
        tracing::info!(
            turn,
            "session {session_id} 本轮水库：容量 {}ms（峰值水位 {}ms / {}KB），起跑闸等待 {}ms（水位 {}ms），\
             背压 {}ms，上游断供 {} 次（累计 {}ms）{}",
            res.cap_ms,
            res.peak_ms,
            res.peak_bytes / 1024,
            res.gate_ms,
            res.gate_depth_ms,
            res.backpressure_ms,
            res.starve_events,
            res.starve_ms,
            if res.starve_ms > 0 {
                "——上游供数慢于播放：调大 [audio].downlink_buffer_ms / downlink_buffer_start_ms 可再吸收一段"
            } else {
                ""
            }
        );
        // ③ 下载缓冲（PCM）：预算 = 上游 PCM 积压上限；限速等待 > 0 = 服务端按预算压住了下载
        //    速率（上游下载快于消费，内存因此有界——这正是"根据缓存需要控制下载流速"）
        let dl = download.stats();
        tracing::info!(
            turn,
            "session {session_id} 本轮下载缓冲：预算 {}ms（峰值水位 {}ms），下载限速等待 {}ms / {} 次{}",
            dl.budget_ms,
            dl.peak_ms,
            dl.throttle_ms,
            dl.throttle_events,
            if dl.throttle_ms > 0 {
                "——TTS 流下载快于消费，已按预算压住（内存有界）"
            } else {
                ""
            }
        );
        // ④ 本轮吞吐：音频时长 / **播放窗口**（首帧 → 末帧，TTFA 不计入）。
        //    < 1.0× 说明上游**持续**慢于实时：缺口是累积的，缓冲只能推迟断音、不能消除
        //    （见 `config::AudioConfig::downlink_buffer_ms` 文档的"所需缓冲"公式）。
        let audio_ms = drain.frames as u64 * self.params.downlink_frame_ms.max(1) as u64;
        if let Some(first) = first_frame_at {
            let window_ms = first.elapsed().as_millis() as u64;
            if window_ms > 0 && audio_ms > 0 {
                let rate = audio_ms as f64 / window_ms as f64;
                if rate < 0.98 {
                    tracing::warn!(
                        turn,
                        "session {session_id} 本轮吞吐：音频 {audio_ms}ms / 播放窗口 {window_ms}ms ≈ {rate:.2}× 实时\
                         ——上游**持续**慢于播放，缓冲只能把断音往后推、不能消除。根治方向：提高 [tts].speed\
                         （音频变短，直接缩小缺口）、换更快的 LLM/TTS，或把 [audio].downlink_buffer_start_ms\
                         调大（先攒后播：要整轮不断音需攒 ≈ (1/{rate:.2} − 1) × {audio_ms}ms）"
                    );
                } else {
                    tracing::info!(
                        turn,
                        "session {session_id} 本轮吞吐：音频 {audio_ms}ms / 播放窗口 {window_ms}ms ≈ {rate:.2}× 实时（上游跟得上播放）"
                    );
                }
            }
        }
    }
}
