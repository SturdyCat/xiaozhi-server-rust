//! 会话回复的生成与下发：LLM 流式 → 分句 → 合成 → 下行（含失败兜底话术）。
//!
//! 从 `session.rs` 拆出（见 `AGENTS.md` §5.10 文件规模约定）。`Session` 定义在父模块，
//! **子模块可直接访问其私有字段**，因此拆分无需放宽任何可见性。
//!
//! LLM 生产者（读流 + 分句 + 定时 flush watchdog）进一步抽到 [`producer`]——
//! 本文件只保留编排层（prompt 组装 / 历史与计量 / 兜底与观测）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::downlink::{
    drain_reservoir, new_reservoir, send_tts_state, spawn_reservoir_filler,
};
use crate::app::events::{TurnId, TurnTrace};
use crate::app::prefetch::{PreparedSentence, SentenceSource};
use crate::app::protocol::ServerMessage;
use crate::app::transport::Transport;
use crate::plugins::prompt::TurnPrompt;

use super::producer::{spawn_llm_producer, LlmProducer};
use super::{send_text, Session, FALLBACK_REPLY};

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
            match memory
                .maintain(crate::plugins::memory::MaintainOpts::periodic())
                .await
            {
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

    /// 一条用户话语在技能级联未命中后的**统一响应入口**（注入轮与语音轮共用）。
    ///
    /// - `[llm].enabled = true`（默认）：照常 `stream_response`（LLM→TTS→下行）；
    /// - `false`：**回声模式**——不调 LLM，把 ASR 文本原样 TTS 下发（见
    ///   [`Self::echo_response`]），用于验证 ASR→TTS 链路。
    ///
    /// 档位 NextSession：`eff_config` 在会话开始热刷新，保存后**新会话**生效。
    /// ⚠️ 仅**全局**生效：`llm.enabled` 不在设备 overlay 白名单（`config/overlay.rs`
    /// 的 llm 段只放行 system_prompt/max_history）——它是全局测试开关，不随设备下发。
    pub(super) async fn respond(
        &mut self,
        user_text: &str,
        trace: &Arc<TurnTrace>,
    ) -> anyhow::Result<()> {
        if !self.eff_config.llm.enabled {
            return self.echo_response(user_text, trace).await;
        }
        self.stream_response(user_text, trace).await
    }

    /// 回声模式（`[llm].enabled=false`）：跳过 LLM 与记忆，把用户文本原样送 TTS。
    ///
    /// 帧序列与正常轮对齐：先 `Stt`（设备 UI 显示识别文本）+ `Llm` 心跳——**Llm 必须
    /// 发**：设备端靠收到任意服务器消息刷新 `last_incoming_time_`，缺了它 TTS 合成
    /// 等待期会被 ESP `IsTimeout()` 判断连（见 `stream_response` 内同款注释）；随后
    /// 复用 [`Self::speak_canned`] 的完整下行链路（预取/节拍器/tts start/stop/打断）。
    ///
    /// 测试路径刻意**不写对话历史、不做记忆写入**；对话记录/事件沿用 `speak_canned`
    /// 现状（anchor=canned 的独立一轮）。
    async fn echo_response(
        &mut self,
        user_text: &str,
        trace: &Arc<TurnTrace>,
    ) -> anyhow::Result<()> {
        self.abort.store(false, Ordering::Relaxed);
        Self::send_stt_and_llm_heartbeat(self.transport, &self.session_id, user_text).await?;
        tracing::info!(
            turn = trace.turn_id(),
            "session {} 回声模式（[llm].enabled=false）：跳过 LLM，原话 TTS 下发（{} 字）",
            self.session_id,
            user_text.chars().count()
        );
        self.speak_canned(user_text).await
    }

    /// 下发 `Stt` + `Llm` 心跳（正常轮与回声轮共用的帧序列头部）。
    ///
    /// `Llm` 心跳的语义：协议约定它仅做表情同步、回复正文走 `tts.sentence_start`，
    /// 但设备端把它当"服务器还活着"的信号刷新 `last_incoming_time_`——**不能省**。
    async fn send_stt_and_llm_heartbeat(
        transport: &mut T,
        session_id: &str,
        user_text: &str,
    ) -> anyhow::Result<()> {
        send_text(
            transport,
            &ServerMessage::Stt {
                session_id: session_id.to_string(),
                text: user_text.to_string(),
            },
        )
        .await?;
        send_text(
            transport,
            &ServerMessage::Llm {
                session_id: session_id.to_string(),
                emotion: Some("neutral".to_string()),
                text: None,
            },
        )
        .await
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
        Self::send_stt_and_llm_heartbeat(self.transport, &session_id, user_text).await?;

        let t0 = std::time::Instant::now();
        // 本轮上下文：人格 + 记忆召回（**在 LLM 之前**，且召回失败不影响继续）
        let prompt = self.build_turn_prompt(user_text).await;
        // 定时 flush 等待（缺口③）：每轮从生效配置现读（0 = 关闭）。
        // ⚠️ 仅**全局**生效：`llm.sentence_flush_ms` 不在设备 overlay 白名单
        // （`config/overlay.rs` 的 llm 段只放行 system_prompt/max_history）。
        let flush_wait = Duration::from_millis(self.eff_config.llm.sentence_flush_ms as u64);
        // 生产者：读 LLM 流 → 按句切分（+ watchdog）→ channel。
        // 技能目录非空才声明 `execute_skill` 工具（纯聊天不传 tools：省 token、收误触发面）。
        let tools = self.skills.tools();
        let LlmProducer {
            sentences: rx,
            result: producer,
        } = spawn_llm_producer(
            self.engines.llm(),
            self.history.clone(),
            user_text.to_string(),
            prompt,
            self.abort.clone(),
            super::producer::ProducerTurn {
                trace: trace.clone(),
                session_id: session_id.clone(),
                tools: (!tools.is_empty()).then_some(tools),
            },
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
                let warn_msg = format!(
                    "session {session_id} LLM 调用失败（{}，耗时 {}ms）: {e} —— {}",
                    failure.category(),
                    t0.elapsed().as_millis(),
                    failure.hint()
                );
                let error_msg = format!("LLM 调用失败（{}）", failure.category());
                return self
                    .report_llm_failure(
                        trace,
                        turn,
                        interrupted,
                        frames_total,
                        t0,
                        warn_msg,
                        error_msg,
                    )
                    .await;
            }
            Err(e) => {
                let warn_msg = format!("session {session_id} LLM 任务异常: {e}");
                let error_msg = format!("LLM 任务异常: {e}");
                return self
                    .report_llm_failure(
                        trace,
                        turn,
                        interrupted,
                        frames_total,
                        t0,
                        warn_msg,
                        error_msg,
                    )
                    .await;
            }
        };
        // 令牌计量：回答"成本多少 / 记忆与灵魂让上下文长了多少"。
        // ⚠️ 取消轮（`cancelled`）**照常计量**：上游已收款，统计不能因打断而丢。
        self.engines.usage.record(&session_id, llm_out.usage);
        // 技能回接（级联末级 → 本地执行）：实现与本地闸门共用同一执行出口，见
        // `command.rs::execute_llm_tool_calls`。取消轮 `tool_calls` 恒为空（取消契约）。
        self.execute_llm_tool_calls(&llm_out.tool_calls, turn).await;
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
        self.engines
            .event_bus
            .emit(crate::app::events::ServerEvent::TtsRound {
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

    /// LLM 失败 / 任务异常两条错误分支的共用收尾（保持日志与记录口径完全一致）：
    /// 记错误 → 事件总线 → 分段延迟 emit → 失败轮 log → （尚未出声时）兜底话术 → 记助手轮次。
    ///
    /// ⚠️ 语音场景**绝不能静默**：用户听不到任何声音 = "AI 没反应"（最差体验，也是此前运行期
    /// 失败全部只 `warn!` 的老问题）。一个字都还没下发时才用兜底话术回应，已经出过声就不再补充
    /// （避免"说完半句又道歉"）；兜底话术也记进对话，避免界面出现"用户问了却没有任何回复"。
    ///
    /// `warn_msg` / `error_msg` 由调用方在**进入本函数前**按各自分支格式化（warn 含当时的耗时，
    /// 必须取调用瞬间的值，本函数内不再重算以免计时漂移）。
    async fn report_llm_failure(
        &mut self,
        trace: &Arc<TurnTrace>,
        turn: TurnId,
        interrupted: bool,
        frames_total: usize,
        t0: Instant,
        warn_msg: String,
        error_msg: String,
    ) -> anyhow::Result<()> {
        tracing::warn!(turn, "{warn_msg}");
        self.note_error(error_msg.clone());
        self.engines
            .event_bus
            .emit(crate::app::events::ServerEvent::Error {
                session_id: self.session_id.clone(),
                device_id: self.device_id.clone(),
                message: error_msg,
            });
        // 分段延迟：失败轮照样 emit（锚点轮次发生了就是发生了）
        self.emit_turn_latency(trace, interrupted);
        self.log_turn_end(trace, interrupted);
        if !interrupted && frames_total == 0 {
            self.speak_fallback(FALLBACK_REPLY, turn).await?;
        }
        // 取"说完兜底话术之后"的耗时为这一轮助手轮次的耗时（与正常成功轮口径一致）。
        let elapsed = t0.elapsed().as_millis() as u64;
        self.note_assistant_turn(FALLBACK_REPLY, elapsed);
        self.emit_message_added(turn, "assistant", FALLBACK_REPLY, Some(elapsed));
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
        tracing::info!(
            turn = parent_turn,
            "session {} 已下发兜底话术",
            self.session_id
        );
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
        tracing::info!(
            turn,
            "session {} 下发预制话术（锚点 canned）: {text}",
            self.session_id
        );
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let _ = tx.send(text.to_string());
        drop(tx); // 只有一句 → 关掉句子流，预取 worker 处理完即退出
        let (prepared, download) = self.spawn_prefetch(rx, &trace);
        let t0 = Instant::now();
        match self.drain_prepared(prepared, download, t0, &trace).await {
            Ok(o) if o.sentences == 0 => {
                tracing::warn!(
                    turn,
                    "session {} 话术未下发任何音频: {text}",
                    self.session_id
                );
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
    /// 每句共用会话建立时固定的实际 TTS 租约（NextSession）；音色/语速已应用 overlay。
    /// 失败重建不冒充新配置，旧会话也不会随别的会话刷新而在句间切换引擎。
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
        let lease = self.tts_lease.clone();
        let params = self.params;
        let download = crate::app::prefetch::DownloadBudget::new(self.params.download_buffer_ms);
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
            self.params.output_format(),
            move |sentence: &str| {
                // 缓存键口径的**唯一权威**在 `HotAudioKey::from_lease`。
                //
                // 为什么不让本处自己拼 `TtsCache::make_key`：唤醒问候语路径要把**同一个键**
                // 的成品音频暖进本缓存（读盘复用），两处各写一遍参数顺序，一旦漂移就是
                // "缓存永远不命中"——不报错、只是白合成，是最难查的一类 bug。所以合成路径
                // 与问候路径共用这一个构造入口。
                let key = crate::app::hot_audio::HotAudioKey::from_lease(&lease, sentence);
                let fmt = params.output_format();
                SentenceSource {
                    tts: lease.engine(),
                    // 内存键含整个下行格式（帧按会话格式编码）；磁盘身份不含（存引擎原样 PCM）。
                    cache_key: key.mem_key(fmt),
                    // 完整身份：预取 worker 据此登记热音频（磁盘持久化，跨重启复用）。
                    // `register_identity` 内部校验 `mem_key == cache_key`，漂移即不登记。
                    hot_audio_key: Some(key.clone()),
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
        let (sink, source) = new_reservoir(
            self.params.downlink_buffer_ms,
            self.params.downlink_frame_ms,
        );
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
}
