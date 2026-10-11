//! 语音指令闸门在会话层的落点：ASR → LLM **之间**。
//!
//! 从 `session.rs` 拆出（见 `AGENTS.md` §5.10 文件规模约定）。`Session` 定义在父模块，
//! 子模块可直接访问其私有字段，因此拆分无需放宽任何可见性。
//!
//! ## 命中后做什么：先说告别语，再用 MCP 让**设备自己**结束聆听
//!
//! 这类话不是提问，送进 LLM 只会得到一段没用的回复（还要计费、还要合成），因此命中后
//! **一次 LLM 都不调用**：先说告别语（走既有 TTS 下行链路），随后按 MCP
//! （**设备是 MCP Server，服务器是 MCP Client**，见 [`crate::app::mcp`]）发一条
//! `tools/call`，让设备退出聆听态、回到待唤醒。
//!
//! **为什么不再是"直接断开连接"**：断开等于把这条音频通道扔掉，用户下次唤醒必须重新建连
//! （握手 → 服务器 hello → 会话状态重建），现场表现就是"退下之后它又重连了一次"。
//! 让设备自己结束聆听则**音频通道保留**，下次唤醒直接复用——这是本次改动的全部目的。
//!
//! ## 三条降级路径（都必须保留）
//!
//! | 情形 | 表现 | 处置 |
//! |---|---|---|
//! | 设备未声明 `features.mcp`（极老固件） | [`crate::app::session::mcp::McpCallError::Unsupported`]，**一个报文都不发** | 断开 |
//! | 固件没刷上 `self.audio_listener.stop` | 设备回 `-32602 Unknown tool`（正常响应，不算传输错误） | 断开 |
//! | 设备不回包 / 等待期间断开 | 超时 / `Closed` | 断开 |
//!
//! 降级 = 旧行为（`close_requested`），所以"服务器先上线、固件后烧录"不会让功能变差，
//! 只会在日志里留一行 warn 说明为什么没走 MCP。**任何情况下都不静默失败。**
//!
//! ## 与断开路径的关系
//!
//! 降级后仍由主循环收尾完成收尾动作（跳过 VAD flush → 等排空 → `Transport::close()`），
//! 即 [`Session::run`] 里那段 `close_requested` 分支——本模块不自己关 socket。

use std::sync::atomic::Ordering;

use crate::app::mcp::STOP_LISTENING_TOOL;
use crate::app::transport::Transport;

use super::Session;

impl<'a, T: Transport> Session<'a, T> {
    /// 指令闸门：识别文本命中指令词？
    ///
    /// - 返回 `true` = 本轮已被指令消费，调用方**必须**停止把这句话送进 LLM；
    ///   会话随后要么被设备侧结束聆听（音频通道保留），要么被降级为断开（`close_requested`）。
    /// - 返回 `false` = 未命中（或闸门未启用、或文本超过 `[command].max_chars` 长度闸门），正常走 LLM。
    ///
    /// 长度闸门在 `CommandGate::hit` 内部，本层不重复判断：长句携带信息（「帮我关闭卧室的灯」），
    /// 放行给 LLM 才是正确处置，而它就在这里自然发生。
    ///
    /// 告别语失败（TTS 构建/网络）**不阻止后续处置**：用户说了「退下」，会话就该结束聆听，
    /// 不能因为合成失败把这次会话卡在"没反应"的状态里。
    pub(super) async fn handle_command_gate(&mut self, user_text: &str) -> bool {
        let Some(hit) = self.commands.hit(user_text) else {
            return false;
        };
        tracing::info!(
            "session {} 命中指令「{}」（识别文本 {:?}）：跳过 LLM，说告别语后通知设备结束聆听",
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
                    "session {} 告别语下发失败（仍按指令结束聆听）: {e}",
                    self.session_id
                );
            }
        }
        // 告别语已经下发完毕（`speak_canned` 内部走 `tts start` → 音频 → `tts stop`）。
        // 设备处理完 `tts stop` 即进入 listening，因此**紧接着**下发的 MCP 指令正好命中
        // 固件 `HandleStopListeningEvent` 的 `kDeviceStateListening` 分支（两者在同一条
        // 连接上按序到达，无需人为 sleep——加了反而会把麦克风短暂打开）。
        self.stand_down_via_mcp().await;
        true
    }

    /// 用 MCP 让设备结束聆听；**任何失败都降级为「断开本次会话」**（= 加 MCP 之前的行为）。
    ///
    /// 独立成函数的原因：它是本模块唯一的副作用出口，降级判定集中在一处，
    /// 将来接「LLM 工具调用闭环」（`docs/esp-compat-and-latency-plan.md` §3.3）时可复用同一口径。
    async fn stand_down_via_mcp(&mut self) {
        match self
            .mcp_call_tool(STOP_LISTENING_TOOL, serde_json::json!({}))
            .await
        {
            Ok(reply) if reply.succeeded() => {
                tracing::info!(
                    "session {} 已通过 MCP 通知设备结束聆听（音频通道保留，下次唤醒复用连接）: {}",
                    self.session_id,
                    reply.summary()
                );
                // 丢弃 VAD 里可能残留的半句音频：用户已明确退下，不该把它带进下一轮对话。
                // （与 `run()` 命中指令时跳过 flush 同一口径——区别只是这里会话还活着。）
                self.vad.flush(&mut |_| {});
            }
            Ok(reply) => {
                // 设备明确拒绝：最常见的原因是固件还没刷上 `self.audio_listener.stop`
                // （回 `-32602 Unknown tool`）。日志必须把工具名与设备原话都带上，
                // 否则现场只能看到"退下后断线了"而查不出原因。
                tracing::warn!(
                    "session {} 设备拒绝结束聆听指令（确认固件是否包含 {}），降级为断开本次会话: {}",
                    self.session_id,
                    STOP_LISTENING_TOOL,
                    reply.summary()
                );
                self.close_requested = true;
            }
            Err(e) => {
                tracing::warn!(
                    "session {} MCP 结束聆听指令未能完成，降级为断开本次会话: {e}",
                    self.session_id
                );
                self.close_requested = true;
            }
        }
    }
}
