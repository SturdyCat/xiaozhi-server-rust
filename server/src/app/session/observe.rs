//! 会话 → 注册表的**观测埋点**（[`crate::app::registry`] 的唯一写入侧）。
//!
//! ## 为什么单独一个文件
//!
//! 埋点是横切关注点：它散在 ASR 出口、下行入口、消息分发三处，每处只有一行。把这些调用
//! **具名**（`note_user_turn` / `note_assistant_turn` / `note_phase` / `note_error`），
//! 而不是让调用点直接碰 `engines.registry`，换来两件事：
//!
//! 1. **会话 id 的取值口径只有一处**（`self.session_id`）——不会出现某个调用点顺手传了设备号
//!    或另一条会话的 id，而那类 bug 的表现是"界面上的对话串了"，极难查；
//! 2. 注册表 API 改名/拆表时只改本文件，流水线一行不动。
//!
//! ## 边界
//!
//! 这些方法**只记录、不决策**，也没有返回值：注册表内部对锁中毒是容错的（见其模块注释），
//! 因此调用点不需要（也不应该）处理任何错误——观测面绝不允许把语音链路带崩。

use super::Session;
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
}
