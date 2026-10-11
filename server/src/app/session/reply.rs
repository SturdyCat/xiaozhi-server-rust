//! 会话回复的生成与下发：LLM 流式 → 分句 → 合成 → 下行（含失败兜底话术）。
//!
//! 从 `session.rs` 拆出（见 `AGENTS.md` §5.10 文件规模约定）。`Session` 定义在父模块，
//! **子模块可直接访问其私有字段**，因此拆分无需放宽任何可见性。

use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::app::downlink::{
    drain_reservoir, new_reservoir, send_tts_state, spawn_reservoir_filler,
};
use crate::app::prefetch::{PreparedSentence, SentenceSource};
use crate::app::protocol::ServerMessage;
use crate::app::splitter::SentenceSplitter;
use crate::app::transport::Transport;
use crate::app::tts_cache::TtsCache;
use crate::plugins::llm::LlmEvent;
use crate::plugins::prompt::TurnPrompt;

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
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
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

        // 预取：句子 → 成品（下一句的合成跑在**本句播放期间**，消除句间合成气泡——
        // 在线合成每次调用都是一条短连接，旧串行实现每句边界都要重付一次握手延迟）
        let (prepared, download) = self.spawn_prefetch(rx);
        // 消费者：水库按实时节拍抽取下发（`tts start` 在首项到达时才发）→ 整轮结束 `tts stop`
        // 新一轮回复：节拍器时间表重置（本回复内各句共享同一时间表 → 句间无缝）
        self.pacer.reset();
        let outcome = self.drain_prepared(prepared, download, t0).await?;
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
                    "session {session_id} LLM 调用失败（{}，耗时 {}ms）: {e} —— {}",
                    failure.category(),
                    t0.elapsed().as_millis(),
                    failure.hint()
                );
                // ⚠️ 语音场景**绝不能静默**：用户听不到任何声音 = "AI 没反应"（最差体验，
                // 也是此前运行期失败全部只 warn! 的老问题）。一个字都还没下发时，用一句
                // 兜底话术回应；已经出过声就不再补充（避免"说完半句又道歉"）。
                if !interrupted && frames_total == 0 {
                    self.speak_fallback(FALLBACK_REPLY).await?;
                }
                return Ok(());
            }
            Err(e) => {
                tracing::warn!("session {session_id} LLM 任务异常: {e}");
                if !interrupted && frames_total == 0 {
                    self.speak_fallback(FALLBACK_REPLY).await?;
                }
                return Ok(());
            }
        };
        // 令牌计量：回答"成本多少 / 记忆与灵魂让上下文长了多少"
        self.engines.usage.record(&session_id, llm_out.usage);
        // ⚠️ 这里打印的是**整轮耗时**（t0 → 最后一帧下发完成），不是"LLM 流耗时"：
        // 旧文案写"LLM 流 Xms"会让人误读成 LLM 吞吐（曾据此误判上游速度）。
        // 与"音频 Xms"对比才是判断节流的正确口径：两者接近 = 全程实时无缝；
        // 耗时远大于音频 = 中间被供数拖住了（结合下面的队列余量日志定位是谁慢）。
        let audio_ms = frames_total as u64 * self.params.downlink_frame_ms.max(1) as u64;
        tracing::info!(
            "session {session_id} 本轮完成：耗时 {}ms / 音频 {audio_ms}ms（{frames_total} 帧）/ LLM 输出 {} 字 / {} 个工具调用，下发 {sentences_done} 句{}",
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
    /// 走与正常回复**完全同一条**下行链路（预取 → 节拍器 → tts start/stop），
    /// 因此听感与打断语义一致；合成失败只记日志（调用方各自决定后续行为：
    /// 兜底继续返回、指令闸门照常断开）。
    pub(super) async fn speak_canned(&mut self, text: &str) -> anyhow::Result<()> {
        // 一句独立的话 = 一次独立回复：节拍器时间表重置（指令闸门路径不会经过
        // `stream_response` 的 reset，不重置会沿用上一轮的时间锚点）。
        self.pacer.reset();
        // 自包含：清掉上一轮遗留的打断标志。否则预取 worker 在入口即退出，
        // `tts start`/`stop` 都不发 —— 客户端/测试台会卡在"合成中"，用户则听到静音。
        // 指令闸门路径本来就会清（见 `command.rs`），这里给兜底路径补上同一语义。
        self.abort.store(false, Ordering::Relaxed);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let _ = tx.send(text.to_string());
        drop(tx); // 只有一句 → 关掉句子流，预取 worker 处理完即退出
        let (prepared, download) = self.spawn_prefetch(rx);
        let t0 = Instant::now();
        match self.drain_prepared(prepared, download, t0).await {
            Ok(o) if o.sentences == 0 => {
                tracing::warn!("session {} 话术未下发任何音频: {text}", self.session_id);
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("session {} 话术下发失败: {e}", self.session_id),
        }
        Ok(())
    }

    /// 启动逐句**预取**（见 [`crate::app::prefetch`]）：句子流 → 成品流。
    ///
    /// 每句都按**会话级最新配置**解析引擎/音色/语速/引擎签名与缓存键——与旧实现的
    /// "句间热切换"语义一致（切引擎后不会命中上一个引擎的缓存音频）。
    ///
    /// 同时建出本轮的**下载缓冲**（[`DownloadBudget`]）：合成回调据此按预算暂停/恢复
    /// TTS 流下载（内存换性能 + 下载限速），返回值随行交给 [`Self::drain_prepared`]
    /// 打印每轮观测。
    fn spawn_prefetch(
        &self,
        sentences: tokio::sync::mpsc::UnboundedReceiver<String>,
    ) -> (
        tokio::sync::mpsc::Receiver<PreparedSentence>,
        std::sync::Arc<crate::app::prefetch::DownloadBudget>,
    ) {
        let engines = self.engines.clone();
        let params = self.params;
        let download =
            crate::app::prefetch::DownloadBudget::new(self.params.download_buffer_ms);
        let prepared = crate::app::prefetch::spawn_sentence_prefetch(
            self.abort.clone(),
            self.engines.tts_cache.clone(),
            download.clone(),
            move |sentence: &str| {
                let live = engines.live_config();
                let speed = live.tts.speed;
                let speaker = live.tts.speaker;
                // 引擎签名口径的唯一权威在注册表（含密钥字段，仅内存比较、不打日志）
                let cache_key = TtsCache::make_key(
                    &crate::plugins::registry::signature_of(
                        &live,
                        crate::plugins::registry::Capability::Tts,
                    ),
                    speaker,
                    speed,
                    params.downlink_sr,
                    params.downlink_frame_ms,
                    sentence,
                );
                SentenceSource { tts: engines.tts(), cache_key, speed, speaker }
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
    /// 每轮结束打印三类观测：节拍器的**设备解码队列余量最低估计**（断音现场证据）、
    /// 水库水位/饥饿（上游供数是否慢于播放）、下载缓冲水位/限速（TTS 流下载是否被按预算压住）
    /// 与**本轮吞吐**（音频时长 / 播放窗口，< 1.0× 即上游持续慢于实时——缓冲只能推迟断音）。
    async fn drain_prepared(
        &mut self,
        prepared: tokio::sync::mpsc::Receiver<PreparedSentence>,
        download: std::sync::Arc<crate::app::prefetch::DownloadBudget>,
        t0: Instant,
    ) -> anyhow::Result<DownlinkOutcome> {
        let session_id = self.session_id.clone();
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
        // 收尾：正常路径填充任务已自行收工（写端丢弃）；打断/异常时立刻停掉，
        // 别再白合成、白编码本轮剩余音频。
        filler.abort();
        if drain.tts_started {
            // 打断时也发 stop：设备据此进入"排空 → 聆听"（**不会**清空解码队列，见
            // `downlink.rs` 的节拍器文档：残留量 ≈ 提前量）
            send_tts_state(self.transport, &self.session_id, "stop").await?;
        }
        self.log_downlink(&session_id, &drain, &download, first_frame_at);
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
    fn log_downlink(
        &self,
        session_id: &str,
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
                "session {session_id} 本轮下行**断音**：设备解码队列余量最低估计 {depth}ms（< 1 帧 {frame_ms}ms）\
                 ，真空 {} 次（最长 {}ms），抖动缓冲 {lead}ms。——上游供数慢于播放（LLM 出句 / TTS 合成 RTF）\
                 或网络抖动；可上调 [audio].downlink_lead_ms（当前安全上限 {cap}ms，受设备解码队列包数×帧长限制，\
                 想更深必须先放大固件 MAX_DECODE_PACKETS_IN_QUEUE）、或调大 [audio].downlink_buffer_ms / \
                 downlink_buffer_start_ms（服务器侧水库），或降低合成耗时",
                stats.vacuum_events,
                stats.max_vacuum_ms
            ),
            Some(depth) => tracing::info!(
                "session {session_id} 本轮下行平顺：设备解码队列余量最低估计 {depth}ms，真空 {} 次（最长 {}ms）",
                stats.vacuum_events,
                stats.max_vacuum_ms
            ),
            None => tracing::debug!("session {session_id} 本轮下行过短，未采集队列余量"),
        }
        tracing::info!(
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
                        "session {session_id} 本轮吞吐：音频 {audio_ms}ms / 播放窗口 {window_ms}ms ≈ {rate:.2}× 实时\
                         ——上游**持续**慢于播放，缓冲只能把断音往后推、不能消除。根治方向：提高 [tts].speed\
                         （音频变短，直接缩小缺口）、换更快的 LLM/TTS，或把 [audio].downlink_buffer_start_ms\
                         调大（先攒后播：要整轮不断音需攒 ≈ (1/{rate:.2} − 1) × {audio_ms}ms）"
                    );
                } else {
                    tracing::info!(
                        "session {session_id} 本轮吞吐：音频 {audio_ms}ms / 播放窗口 {window_ms}ms ≈ {rate:.2}× 实时（上游跟得上播放）"
                    );
                }
            }
        }
    }
}
