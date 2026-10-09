//! 语音指令闸门在会话层的落点：ASR → LLM **之间**。
//!
//! 从 `session.rs` 拆出（见 `AGENTS.md` §5.10 文件规模约定）。`Session` 定义在父模块，
//! 子模块可直接访问其私有字段，因此拆分无需放宽任何可见性。
//!
//! 为什么是"拦截 + 断开"而不是"交给 LLM 再解释"：这类话不是提问，送进 LLM 只会得到
//! 一段没用的回复（还要计费、还要合成），正确的处置是结束这次会话。因此命中后
//! **一次 LLM 都不调用**：先说告别语（走既有 TTS 下行链路），再置 `close_requested`，
//! 由主循环收尾时主动关闭承载。

use std::sync::atomic::Ordering;

use crate::app::transport::Transport;

use super::Session;

impl<'a, T: Transport> Session<'a, T> {
    /// 指令闸门：识别文本命中指令词？
    ///
    /// - 返回 `true` = 本轮已被指令消费，调用方**必须**停止把这句话送进 LLM；
    ///   会话将在主循环收尾时断开（`close_requested`）。
    /// - 返回 `false` = 未命中（或闸门未启用、或文本超过 `[command].max_chars` 长度闸门），正常走 LLM。
    ///
    /// 长度闸门在 `CommandGate::hit` 内部，本层不重复判断：长句携带信息（「帮我关闭卧室的灯」），
    /// 放行给 LLM 才是正确处置，而它就在这里自然发生。
    ///
    /// 告别语失败（TTS 构建/网络）**不阻止断开**：用户说了「退下」，会话就该结束，
    /// 不能因为合成失败把这次会话卡在"没反应"的状态里。
    pub(super) async fn handle_command_gate(&mut self, user_text: &str) -> bool {
        let Some(hit) = self.commands.hit(user_text) else {
            return false;
        };
        tracing::info!(
            "session {} 命中指令「{}」（识别文本 {:?}）：跳过 LLM，说告别语后断开本次会话",
            self.session_id,
            hit.keyword,
            user_text
        );
        // ⚠️ 新一轮对话已经开始：先清掉上一轮遗留的打断标志。
        // `abort` 只在 `stream_response` 入口重置，而指令闸门刻意不经过那里；
        // 若上一轮以设备打断结束（abort 仍为 true），告别语会被逐帧打断轮询立刻掐断
        // ——用户会听到"没告别就断线"。
        self.abort.store(false, Ordering::Relaxed);
        if !hit.reply.trim().is_empty() {
            if let Err(e) = self.speak_canned(&hit.reply).await {
                tracing::warn!(
                    "session {} 告别语下发失败（仍按指令断开）: {e}",
                    self.session_id
                );
            }
        }
        self.close_requested = true;
        true
    }
}
