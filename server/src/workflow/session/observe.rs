//! 会话 → 注册表 / 事件总线的**观测埋点**（[`crate::device::registry`] 的唯一写入侧，
//! `app/events` 事件负载的唯一组装处）。
//!
//! ## 为什么单独一个文件
//!
//! 埋点是横切关注点：它散在 ASR 出口、下行入口、消息分发三处，每处只有一行。把这些调用
//! **具名**（`note_user_turn` / `note_assistant_turn` / `note_phase` / `note_error` /
//! `emit_*` / `log_turn_end`），而不是让调用点直接碰 `engines.registry`，换来两件事：
//!
//! 1. **会话 id 的取值口径只有一处**（`self.session_id`）——不会出现某个调用点顺手传了设备号
//!    或另一条会话的 id，而那类 bug 的表现是"界面上的对话串了"，极难查；
//! 2. 注册表 API 改名/拆表时只改本文件，流水线一行不动。
//!
//! 同理，**轮次 id（`turn_id`）的取值口径也只有一处**：从 `TurnTrace` 取（一个 trace 就是
//! 一轮，见 `app/events`），因此事件负载与 `tracing` 日志必然指向同一轮。
//!
//! ## 边界
//!
//! 这些方法**只记录、不决策**，也没有返回值：注册表内部对锁中毒是容错的（见其模块注释），
//! 因此调用点不需要（也不应该）处理任何错误——观测面绝不允许把语音链路带崩。

use std::sync::Arc;

use super::Session;
use crate::device::registry::Phase;
use crate::observability::events::{ServerEvent, TurnId, TurnTrace};
use crate::workflow::protocol::Transport;

impl<'a, T: Transport> Session<'a, T> {
    /// 记一条用户轮次：识别出非空文本、即将交给大模型/指令闸门。
    pub(super) fn note_user_turn(&self, text: &str) {
        self.engines
            .registry
            .record_user_turn(&self.session_id, text);
    }

    /// 记一条助手轮次：整轮已下发完成（或由兜底话术替代）。
    ///
    /// `elapsed_ms` = 整轮耗时（LLM + 合成 + 下行），界面上用来判断"这轮慢不慢"。
    pub(super) fn note_assistant_turn(&self, text: &str, elapsed_ms: u64) {
        self.engines
            .registry
            .record_assistant_turn(&self.session_id, text, elapsed_ms);
    }

    /// 更新会话阶段（聆听中 / 思考中 / 说话中）。
    pub(super) fn note_phase(&self, phase: Phase) {
        self.engines.registry.set_phase(&self.session_id, phase);
    }

    /// 记一条会话级异常：会话结束时会带到设备画像上，因此断电后仍能回答"上次为什么断"。
    pub(super) fn note_error(&self, error: impl Into<String>) {
        self.engines.registry.record_error(&self.session_id, error);
    }

    /// 会话内统一的分段延迟事件出口（缺口②）：session_id/device_id/turn_id 取值口径
    /// 只有一处，与 `note_*` 同理（横切埋点具名收敛，不散落在流水线各分支）。
    ///
    /// 只记录、无返回值：事件总线对慢消费者/无订阅者的容错语义见 `app/events`。
    pub(super) fn emit_turn_latency(&self, trace: &Arc<TurnTrace>, interrupted: bool) {
        self.engines.event_bus.emit(ServerEvent::TurnLatency {
            session_id: self.session_id.clone(),
            device_id: self.device_id.clone(),
            turn_id: trace.turn_id(),
            stages: trace.snapshot(),
            interrupted,
        });
    }

    /// 会话内统一的消息入对话事件出口：`turn_id` 由调用方给出（用户轮与其助手回复共享
    /// 同一个值，见 [`ServerEvent::MessageAdded`]）。
    ///
    /// 与 [`Self::emit_turn_latency`] 同理把 id 取值口径收敛到一处：三个 emit 点
    /// （用户侧 `decide_user_text`、助手侧正常/兜底两处）手写 `turn_id` 迟早会漏一个。
    pub(super) fn emit_message_added(
        &self,
        turn_id: TurnId,
        role: &str,
        text: &str,
        elapsed_ms: Option<u64>,
    ) {
        self.engines.event_bus.emit(ServerEvent::MessageAdded {
            session_id: self.session_id.clone(),
            device_id: self.device_id.clone(),
            turn_id: Some(turn_id),
            role: role.to_string(),
            text: text.to_string(),
            elapsed_ms,
        });
    }

    /// 轮次结束日志（缺口②的**现场可读形态**）：一行带齐 `turn=` 与全部分段偏移。
    ///
    /// 为什么放在 `observe.rs`：这是"埋点横切、具名收敛"的同一类职责——轮次收尾的
    /// 可观测出口只有一处，`reply.rs` 的三条结束分支不必各自拼一份分段摘要。
    /// 有了它，`grep turn=3` 捞出的一轮日志**最后一行就直接回答「慢在哪一段」**。
    ///
    /// `anchor` 与 `interrupted` 同时作为结构化字段与文案出现：前者供订阅者/检索，
    /// 后者让纯文本日志（`journalctl` 不带 JSON）也不丢口径。
    ///
    /// **debug 档**（日志降噪）：分段延迟是排障数据，不是"收到了什么事件/说了什么"——
    /// 默认 info 档不出现，`RUST_LOG=debug` 全量可见（见 `main.rs::default_log_filter`）。
    pub(super) fn log_turn_end(&self, trace: &Arc<TurnTrace>, interrupted: bool) {
        let stages = trace.snapshot();
        tracing::debug!(
            turn = trace.turn_id(),
            anchor = stages.anchor,
            interrupted,
            "session {} 本轮分段（锚点 {}）：{}",
            self.session_id,
            stages.anchor,
            stages.summary(),
        );
    }

    /// 每轮下行汇总日志（与 [`Self::log_turn_end`] 同批并入 `observe.rs`：纯观测、
    /// 不决策，见本文件"为什么单独一个文件"）：① 设备解码队列余量最低估计（断音现场证据）；
    /// ② 水库水位/背压/饥饿（上游供数速度——判断"该调大水库还是该治上游"）；
    /// ③ 下载缓冲水位/限速（TTS 流下载是否被预算压住）；
    /// ④ **本轮吞吐**（音频时长 / 播放窗口）：< 1.0× 即上游持续慢于实时。
    ///
    /// 四条全部带 `turn=`：它们是"这一轮慢在哪一段"的**下半程**证据（与上面的
    /// `log_turn_end` 上半程摘要配对），`grep turn=N` 一次捞全。
    ///
    /// **日志分级（降噪）**：默认 info 档只在**异常**时出现——断音 / 吞吐 <0.98×（warn）；
    /// 健康轮的平顺/水库/下载缓冲/吞吐细节全部 debug（`RUST_LOG=debug` 全量可见）。
    /// 正常对话在 info 档只剩：唤醒上报 → 用户 → 回复 三类关键行。
    pub(super) fn log_downlink(
        &self,
        session_id: &str,
        turn: TurnId,
        drain: &crate::workflow::downlink::ReservoirDrain,
        download: &crate::workflow::pipeline::prefetch::DownloadBudget,
        first_frame_at: Option<std::time::Instant>,
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
            Some(depth) => tracing::debug!(
                turn,
                "session {session_id} 本轮下行平顺：设备解码队列余量最低估计 {depth}ms，真空 {} 次（最长 {}ms）",
                stats.vacuum_events,
                stats.max_vacuum_ms
            ),
            None => tracing::debug!(turn, "session {session_id} 本轮下行过短，未采集队列余量"),
        }
        tracing::debug!(
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
        tracing::debug!(
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
                    tracing::debug!(
                        turn,
                        "session {session_id} 本轮吞吐：音频 {audio_ms}ms / 播放窗口 {window_ms}ms ≈ {rate:.2}× 实时（上游跟得上播放）"
                    );
                }
            }
        }
    }
}
