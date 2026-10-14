//! 规则 NLU 在会话层的落点：ASR → LLM **之间**。
//!
//! 从 `session.rs` 拆出（见 `AGENTS.md` §5.10 文件规模约定）。`Session` 定义在父模块，
//! 子模块可直接访问其私有字段，因此拆分无需放宽任何可见性。
//!
//! ## 命中后做什么
//!
//! NLU 是 `[command]` 指令闸门的**超集**：识别文本先过本地规则引擎，命中已知意图即本地执行、
//! **一次 LLM 都不调用**；只有未知意图才放行给 LLM。两条高危/本地动作：
//!
//! - `end_session`（退下/闭嘴/关闭，配置仍在 `[command]` 段）：说告别语（复用既有 TTS 下行链路），
//!   随后按 MCP 让**设备自己**结束聆听（设备是 MCP Server、服务器是 MCP Client，见 [`crate::app::mcp`]）。
//! - 其他本地指令（如「把{room}的灯{action}」，配置在 `[nlu]` 段）：说应答（若有）后即本地消费，
//!   不再送 LLM；实际设备控制（kind → 设备 MCP 工具）为预留扩展位。
//!
//! 为什么 end_session 不再是"直接断开连接"：断开等于把这条音频通道扔掉，用户下次唤醒必须重新建连
//! （握手 → 服务器 hello → 会话状态重建），现场表现就是"退下之后它又重连了一次"。
//! 让设备自己结束聆听则**音频通道保留**，下次唤醒直接复用——这是本次改动的全部目的。
//!
//! ## end_session 的四条降级路径（都必须保留）
//!
//! | 情形 | 表现 | 处置 |
//! |---|---|---|
//! | 设备未声明 `features.mcp`（极老固件） | [`crate::app::session::mcp::McpCallError::Unsupported`]，**一个报文都不发** | 断开 |
//! | 清单明确不含 stop 工具（P1-10：探测成功但 `tools/list` 里没有） | **零次 `tools/call`**，直接跳过 | 断开 |
//! | 固件没刷上 `self.audio_listener.stop` 且清单未知 | 设备回 `-32602 Unknown tool`（正常响应，不算传输错误） | 断开 |
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
use crate::plugins::nlu::IntentAction;

use super::Session;

impl<'a, T: Transport> Session<'a, T> {
    /// 规则 NLU 闸门：识别文本是否命中已知意图？
    ///
    /// - 返回 `true` = 本轮已被意图消费，调用方**必须**停止把这句话送进 LLM；
    ///   会话随后要么被设备侧结束聆听（音频通道保留），要么被降级为断开（`close_requested`），
    ///   要么作为本地指令被应答后继续留在通道上。
    /// - 返回 `false` = 未命中（或引擎未启用 / 文本未过长度闸门），正常走 LLM。
    ///
    /// 长度闸门在底层 `CommandGate::hit`（end_session）内部；自定义意图靠模板 + 槽位值表约束，
    /// 漏触发优于误触发（安全优先），因此"长句含指令词但不是指令"自然放行给 LLM。
    ///
    /// 告别语/应答失败（TTS 构建/网络）**不阻止后续处置**：用户说了「退下」，会话就该结束聆听，
    /// 不能因为合成失败把这次会话卡在"没反应"的状态里。
    pub(super) async fn handle_nlu_gate(&mut self, user_text: &str) -> bool {
        let Some(m) = self.nlu.parse(user_text) else {
            return false;
        };
        tracing::info!(
            "session {} 命中 NLU 意图「{}」（识别文本 {:?}，槽位 {:?}）：跳过 LLM，本地消费",
            self.session_id,
            m.intent,
            user_text,
            m.slots
        );
        // ⚠️ 新一轮对话已经开始：先清掉上一轮遗留的打断标志。
        // `abort` 只在 `stream_response` 入口重置，而 NLU 闸门刻意不经过那里；
        // 若上一轮以设备打断结束（abort 仍为 true），应答会被逐帧打断轮询立刻掐断
        // ——用户会听到"没说完整就断线"。
        self.abort.store(false, Ordering::Relaxed);
        // 命中后要说的话（告别语 / 应答）：非空才下发，否则直接执行副作用。
        if !m.reply.trim().is_empty() {
            if let Err(e) = self.speak_canned(&m.reply).await {
                tracing::warn!(
                    "session {} NLU 意图「{}」应答下发失败（仍按意图处置）: {e}",
                    self.session_id,
                    m.intent
                );
            }
        }
        match m.action {
            IntentAction::EndSession => {
                // 复用现有 MCP 结束聆听 + 四条降级路径（见 `stand_down_via_mcp`）。
                self.stand_down_via_mcp().await;
            }
            IntentAction::LocalCommand { kind } => {
                // 本地指令：已本地消费（应答已下发），不再送 LLM、不结束聆听、不关闭连接。
                // 预留：未来按 `kind` 派发到设备 MCP 工具（如灯光控制）。
                tracing::info!(
                    "session {} 本地指令「{}」(kind={}) 已本地消费，会话继续",
                    self.session_id,
                    m.intent,
                    kind
                );
            }
        }
        true
    }

    /// 用 MCP 让设备结束聆听；**任何失败都降级为「断开本次会话」**（= 加 MCP 之前的行为）。
    ///
    /// 独立成函数的原因：它是本模块唯一的副作用出口，降级判定集中在一处，
    /// 将来接「LLM 工具调用闭环」（`docs/esp-compat-and-latency-plan.md` §3.3）时可复用同一口径。
    ///
    /// P1-10：调用前先查会话开始时**已发现**的工具清单（内存读，零 I/O）——
    /// 清单明确不含 stop 工具时直接降级，省一次注定失败的调用（2s 超时或 `-32602` 往返）；
    /// 清单未知（未登记 / 探测失败）时保持既有试探路径，行为不劣化。
    async fn stand_down_via_mcp(&mut self) {
        if self.engines.mcp.has_tool(&self.device_id, STOP_LISTENING_TOOL) == Some(false) {
            tracing::warn!(
                "session {} 设备 {} 的工具清单不含 {}（会话开始时已发现），跳过调用直接降级为断开本次会话",
                self.session_id,
                self.device_id,
                STOP_LISTENING_TOOL
            );
            self.close_requested = true;
            return;
        }
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
