//! 会话 → 注册表 / 事件总线的**观测埋点**（[`crate::app::registry`] 的唯一写入侧，
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
use crate::app::events::{ServerEvent, TurnId, TurnTrace};
use crate::app::registry::Phase;
use crate::app::transport::Transport;

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
        self.engines
            .event_bus
            .emit(ServerEvent::TurnLatency {
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
    pub(super) fn log_turn_end(&self, trace: &Arc<TurnTrace>, interrupted: bool) {
        let stages = trace.snapshot();
        tracing::info!(
            turn = trace.turn_id(),
            anchor = stages.anchor,
            interrupted,
            "session {} 本轮分段（锚点 {}）：{}",
            self.session_id,
            stages.anchor,
            stages.summary(),
        );
    }
}
