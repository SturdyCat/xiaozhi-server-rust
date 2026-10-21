//! 技能在会话层的落点：ASR → LLM **之间**的本地执行 + LLM 工具调用的回接执行。
//!
//! 从 `session.rs` 拆出（见 `AGENTS.md` §5.10 文件规模约定）。`Session` 定义在父模块，
//! 子模块可直接访问其私有字段，因此拆分无需放宽任何可见性。
//!
//! ## 级联中本模块的职责
//!
//! 终端 → ASR → **规则 NLU** → **轻量模型** → 云 LLM → TTS 的第二、三级之间的"执行枢纽"：
//!
//! - 本地两级（规则 / 轻量模型，见 [`crate::plugins::skill::SkillRegistry::parse_staged`]）
//!   命中即本地执行、**一次 LLM 都不调用**；
//! - 云 LLM 若把这句话识别为技能（`execute_skill` 工具调用），同样回到这份目录
//!   校验后**复用同一个执行出口**（[`Self::execute_invocation`]）——三级识别源对副作用
//!   只有提名权，执行收敛在一处。
//!
//! ## 执行语义（与原 `[command]` 闸门一致）
//!
//! - `end_session`（退下/闭嘴/关闭，配置仍在 `[command]` 段）：说告别语（复用既有 TTS
//!   下行链路），随后按 MCP 让**设备自己**结束聆听（设备是 MCP Server、服务器是 MCP
//!   Client，见 [`crate::app::mcp`]）。
//! - `reply` / `local_command`：说应答（若有）后本地消费，不再送 LLM；
//! - `mcp:<工具名>`（来自服务端配置的白名单，LLM 不能指定）：说应答后把槽位作为
//!   JSON 参数派发到**设备声明的**同名 MCP 工具；设备工具清单明确不含该工具时
//!   零调用跳过（与 end_session 的 P1-10 口径一致）。
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
//! `mcp:<工具>` 派发失败（设备拒绝/超时）**不断开**：指令没执行成用户还能再说，
//! 断开反而把"没反应"变成"失联"；只 warn 留痕。
//!
//! ## 与断开路径的关系
//!
//! 降级后仍由主循环收尾完成收尾动作（跳过 VAD flush → 等排空 → `Transport::close()`），
//! 即 [`Session::run`] 里那段 `close_requested` 分支——本模块不自己关 socket。

use std::sync::atomic::Ordering;

use crate::app::mcp::STOP_LISTENING_TOOL;
use crate::app::transport::Transport;
use crate::plugins::skill::{SkillAction, SkillInvocation};

use super::Session;

impl<'a, T: Transport> Session<'a, T> {
    /// 本地技能闸门（级联第二、三级）：识别文本是否被规则/轻量模型消费？
    ///
    /// - 返回 `true` = 本轮已被技能消费，调用方**必须**停止把这句话送进 LLM；
    ///   会话随后要么被设备侧结束聆听（音频通道保留），要么被降级为断开（`close_requested`），
    ///   要么作为本地指令被应答/派发后继续留在通道上。
    /// - 返回 `false` = 本地两级都未命中（或目录为空），正常走云 LLM。
    ///
    /// 应答失败（TTS 构建/网络）**不阻止后续处置**：用户说了「退下」，会话就该结束聆听，
    /// 不能因为合成失败把这次会话卡在"没反应"的状态里。
    pub(super) async fn handle_skill_gate(&mut self, user_text: &str) -> bool {
        let Some((stage, inv)) = self.skills.parse_staged(user_text) else {
            return false;
        };
        tracing::info!(
            "session {} 命中技能「{}」（层级 {}，识别文本 {:?}，槽位 {:?}）：跳过 LLM，本地执行",
            self.session_id,
            inv.name,
            stage.as_str(),
            user_text,
            inv.slots
        );
        self.execute_invocation(&inv, stage.as_str()).await;
        true
    }

    /// 技能执行的**唯一**副作用出口（规则 / 轻量模型 / 云 LLM 三级识别源共用）：
    /// 先清上一轮遗留的打断标志，再说应答（若有），最后执行副作用。
    ///
    /// ⚠️ 清 abort 的原因：`abort` 只在 `stream_response` 入口重置，而技能闸门刻意
    /// 不经过那里；若上一轮以设备打断结束（abort 仍为 true），应答会被逐帧打断轮询
    /// 立刻掐断——用户会听到"没说完整就断线"。LLM 路径入口已清过，这里再清是幂等的。
    pub(super) async fn execute_invocation(&mut self, inv: &SkillInvocation, source: &str) {
        self.abort.store(false, Ordering::Relaxed);
        // 命中后要说的话（告别语 / 应答）：非空才下发，否则直接执行副作用。
        // 成品音频由预取/水库链路在完整合成成功时落盘 hot-audio/（内容寻址缓存，
        // 无需会话层另行登记）。
        if !inv.reply.trim().is_empty() {
            if let Err(e) = self.speak_canned(&inv.reply).await {
                tracing::warn!(
                    "session {} 技能「{}」（{source}）应答下发失败（仍按技能处置）: {e}",
                    self.session_id,
                    inv.name
                );
            }
        }
        match &inv.action {
            SkillAction::EndSession => {
                // 复用现有 MCP 结束聆听 + 四条降级路径（见 `stand_down_via_mcp`）。
                self.stand_down_via_mcp().await;
            }
            SkillAction::LocalCommand { kind } if kind == "reply" => {
                // 兼容应答技能：已本地消费（应答已下发），不再送 LLM、不结束聆听、不关闭连接。
                tracing::info!(
                    "session {} 技能「{}」（{source}）已本地消费，会话继续",
                    self.session_id,
                    inv.name
                );
            }
            SkillAction::LocalCommand { kind } => {
                // `mcp:<工具名>`：白名单来自服务端配置（编译期已校验），槽位作 JSON 参数。
                let Some(tool) = kind.strip_prefix("mcp:") else {
                    tracing::warn!(
                        "session {} 技能「{}」动作 kind 非法（{kind}），已忽略副作用",
                        self.session_id,
                        inv.name
                    );
                    return;
                };
                self.dispatch_device_tool(tool, inv).await;
            }
        }
    }

    /// 云 LLM 回提的技能执行（级联末级 → 本地执行出口的回接）。
    ///
    /// LLM 若把这句话识别为技能（`execute_skill` 工具调用），回到技能目录校验后
    /// 复用与本地闸门完全相同的 [`Self::execute_invocation`]（应答 / 设备工具派发 /
    /// 结束聆听）；白名单外工具、JSON 不合规、槽位校验失败一律拒绝并 warn——
    /// **LLM 输出不可信**，目录是唯一安全边界。
    ///
    /// 取消轮 `tool_calls` 恒为空（引擎取消契约，见 `plugins::llm::LlmTurnResult`），
    /// 打断后不会补执行。
    pub(super) async fn execute_llm_tool_calls(
        &mut self,
        calls: &[crate::plugins::llm::ToolCall],
        turn: crate::app::events::TurnId,
    ) {
        for call in calls {
            match self.skills.from_tool_call(call) {
                Ok(inv) => {
                    tracing::info!(
                        turn,
                        "session {} LLM 提名技能「{}」，回交技能目录执行",
                        self.session_id,
                        inv.name
                    );
                    self.execute_invocation(&inv, "llm").await;
                }
                Err(e) => {
                    tracing::warn!(
                        turn,
                        "session {} 拒绝 LLM 工具调用（工具 {}，参数 {}）: {e}",
                        self.session_id,
                        call.name,
                        call.arguments
                    );
                }
            }
        }
    }

    /// 把技能槽位派发到设备声明的 MCP 工具。失败**不断开**（见模块文档）。
    async fn dispatch_device_tool(&mut self, tool: &str, inv: &SkillInvocation) {
        if self.engines.mcp.has_tool(&self.device_id, tool) == Some(false) {
            tracing::warn!(
                "session {} 设备 {} 的工具清单不含 {tool}（会话开始时已发现），技能「{}」跳过派发",
                self.session_id,
                self.device_id,
                inv.name
            );
            return;
        }
        let arguments: serde_json::Map<String, serde_json::Value> = inv
            .slots
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        match self
            .mcp_call_tool(tool, serde_json::Value::Object(arguments))
            .await
        {
            Ok(reply) if reply.succeeded() => {
                tracing::info!(
                    "session {} 技能「{}」已派发设备工具 {tool}（槽位 {:?}）: {}",
                    self.session_id,
                    inv.name,
                    inv.slots,
                    reply.summary()
                );
            }
            Ok(reply) => {
                tracing::warn!(
                    "session {} 设备拒绝技能「{}」的工具调用 {tool}（确认固件是否支持）: {}",
                    self.session_id,
                    inv.name,
                    reply.summary()
                );
            }
            Err(e) => {
                tracing::warn!(
                    "session {} 技能「{}」派发设备工具 {tool} 未完成（不影响会话继续）: {e}",
                    self.session_id,
                    inv.name
                );
            }
        }
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
        if self
            .engines
            .mcp
            .has_tool(&self.device_id, STOP_LISTENING_TOOL)
            == Some(false)
        {
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
