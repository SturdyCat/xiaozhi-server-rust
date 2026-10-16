//! 会话级 MCP 客户端 I/O：`tools/call` 的发送与**请求-响应关联**，以及会话开始时的
//! **能力发现**（`initialize` + `tools/list`）。
//!
//! 协议原语（构造请求 / 解析响应）在 [`crate::app::mcp`]；本模块只解决一件事：**响应从哪来**。
//!
//! ## 为什么必须有"等待泵"，而不是 `await` 一次了事
//!
//! 设备的响应是**上行帧**，只有会话主循环（[`super::Session::run`]）在读它。而指令闸门里发起的
//! `tools/call` 本身就跑在主循环**内部**（`handle_binary` → `recognize_segments` →
//! `handle_command_gate`）：如果在这里干等一个"由主循环投递"的信号，就是自己等自己 = 死锁。
//! 因此等待必须在**同一任务内抽帧**——收到配对 `id` 的响应即返回，其余帧原样交回调用方补派发
//! （见 [`await_tool_reply`]），绝不静默丢弃（本仓库忌讳静默失败）。
//!
//! ## 两种抽帧策略（差一帧都不能混用）
//!
//! | 场景 | 等待期间的上行音频 | 理由 |
//! |---|---|---|
//! | `tools/call`（指令闸门「退下」） | **丢弃** | 用户已明确要求退下，不该再为残留音频开一轮对话 |
//! | 能力发现（会话最开头） | **缓冲后补派发** | 那时收到的音频是用户**正要说的第一句话**，丢了就是掐头 |
//!
//! 两者共用同一个泵（[`pump_reply`]）与同一套超时/断开判定，只有音频处置不同。
//!
//! ## 与 `docs/esp-compat-and-latency-plan.md` §3.3 的关系
//!
//! §3.3 为「LLM 工具调用闭环」设计的是 `pending_mcp: Mutex<HashMap<id, oneshot::Sender>>`——
//! 那条路径上 LLM 工具调用发生在**独立 task**里，主循环继续跑，所以需要跨 task 关联。
//! 本模块服务的是**主循环内**的控制指令（退下）与**主循环之前**的能力发现，用抽帧泵即可，
//! 无需 oneshot 与共享表；两者可以共存（`mcp_next_id` 已是自增的，将来接闭环时按 §3.3 再加 pending 表）。

use std::time::Duration;

use serde_json::{json, Value};

use crate::app::mcp::discovery::{self, McpServerInfo, McpToolInfo};
use crate::app::mcp::registry::{self, ProbeResult};
use crate::app::mcp::{self, McpReply, McpResponse};
use crate::app::protocol::ServerMessage;
use crate::app::transport::{IncomingFrame, Transport};

use super::{send_text, Session};

/// 一次 `tools/call` 没能拿到"设备执行成功"的原因（调用方据此降级，见 `session/command.rs`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum McpCallError {
    /// 设备 hello 未声明 `features.mcp`（极老固件）：本次会话不启用 MCP，直接走降级路径。
    Unsupported,
    /// 设备在超时内没有回包（含设备不回任何东西、或响应 `id` 一直对不上）。
    Timeout(Duration),
    /// 等待期间承载断开。
    Closed,
    /// 请求本身没发出去（承载写失败）。
    Send(String),
}

impl std::fmt::Display for McpCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpCallError::Unsupported => {
                write!(
                    f,
                    "设备 hello 未声明 features.mcp（老固件），本次会话不启用 MCP"
                )
            }
            McpCallError::Timeout(d) => write!(f, "等待设备 MCP 响应超时（{}ms）", d.as_millis()),
            McpCallError::Closed => write!(f, "等待设备 MCP 响应期间承载断开"),
            McpCallError::Send(e) => write!(f, "MCP 请求下发失败: {e}"),
        }
    }
}

/// 等待期间收到的**非本次响应**帧的处置策略（见模块文档的对照表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinaryPolicy {
    /// 丢弃上行音频。
    Drop,
    /// 原样缓冲，交调用方补派发。
    Keep,
}

/// 抽帧泵：抽上行帧，直到出现与 `id` 配对的 MCP 响应 / 超时 / 断开。
///
/// 返回 `(结果, 等待期间收到的「非本次响应」的帧)`。第二个返回值由调用方按正常会话消息
/// 补派发（`listen stop` 要记日志、`abort` 要置打断位、音频要回灌 VAD……），**不能丢**。
///
/// 刻意不依赖 [`Session`]：`Session` 需要真实模型才能构造（`engine.rs` 的必需能力含 VAD 模型），
/// 抽成自由函数后本逻辑可在内存传输上单测。
async fn pump_reply<T: Transport>(
    transport: &mut T,
    id: u64,
    timeout: Duration,
    binary: BinaryPolicy,
) -> (Result<McpResponse, McpCallError>, Vec<IncomingFrame>) {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut deferred: Vec<IncomingFrame> = Vec::new();
    loop {
        let Some(remaining) = deadline.checked_duration_since(tokio::time::Instant::now()) else {
            return (Err(McpCallError::Timeout(timeout)), deferred);
        };
        match tokio::time::timeout(remaining, transport.recv()).await {
            Err(_elapsed) => return (Err(McpCallError::Timeout(timeout)), deferred),
            Ok(IncomingFrame::Closed) => return (Err(McpCallError::Closed), deferred),
            Ok(IncomingFrame::Text(s)) => match mcp::take_response_for(&s, id) {
                Some(reply) => return (Ok(reply), deferred),
                None => deferred.push(IncomingFrame::Text(s)),
            },
            Ok(frame @ IncomingFrame::Binary(_)) => {
                if binary == BinaryPolicy::Keep {
                    deferred.push(frame);
                }
            }
        }
    }
}

/// [`pump_reply`] 的「丢弃音频」封装：只取文本帧作为遗留消息（`tools/call` 路径）。
pub(super) async fn await_tool_reply<T: Transport>(
    transport: &mut T,
    id: u64,
    timeout: Duration,
) -> (Result<McpReply, McpCallError>, Vec<String>) {
    let (result, deferred) = pump_reply(transport, id, timeout, BinaryPolicy::Drop).await;
    let texts = deferred
        .into_iter()
        .filter_map(|f| match f {
            IncomingFrame::Text(s) => Some(s),
            // Drop 策略下二进制帧根本不会进遗留列表；这里兜底而不是 `unreachable!()`，
            // 免得将来改策略时把"丢一帧"变成 panic（会话级 panic 会打断整条连接）。
            _ => None,
        })
        .collect();
    (result.map(McpResponse::into_reply), texts)
}

/// 一次 MCP 能力发现的**总预算**（`initialize` + 全部 `tools/list` 页共用）。
///
/// 取值理由：探测发生在会话**最开头**，期间上行帧全部缓冲后补派发，所以"慢"的代价是首轮
/// 响应延迟（用户这时通常还在说第一句话），而不是丢音频。2s 远大于一次往返（局域网 ~5ms、
/// 公网 ~100ms 级），又短到"设备根本不回包"时只拖 2s 就照常跑流水线。
pub(super) const DISCOVERY_BUDGET: Duration = Duration::from_millis(2_000);

/// 能力发现的结果（会话侧据此写注册表）。
#[derive(Debug, Clone, Default)]
pub(super) struct Discovery {
    /// 设备自述（板型 / 固件版本 / 协议版本）。
    pub server: McpServerInfo,
    pub tools: Vec<McpToolInfo>,
    /// 成功取到的页数。
    pub pages: usize,
    /// 未完成的原因（`None` = 完整成功；部分成功也会带原因）。
    pub error: Option<String>,
}

/// 探测设备 MCP 能力：`initialize` → `notifications/initialized` → `tools/list` 分页。
///
/// - 用 `withUserTools: true` 拉**全量**工具（管理界面要看全），并靠固件
///   `annotations.audience` 标出哪些 `user_only`（不对大模型暴露）；
/// - **任何一步失败都不致命**：把原因写进 [`Discovery::error`] 并返回已取到的部分，
///   由调用方照常进入会话主循环（绝不允许"设备没答上来"变成"这条连接不可用"）；
/// - `next_id` 由调用方持有并在此自增，避免"提前返回时把 id 计数器重置"（会让后续响应配错）。
pub(super) async fn discover_tools<T: Transport>(
    transport: &mut T,
    session_id: &str,
    next_id: &mut u64,
    budget: Duration,
) -> (Discovery, Vec<IncomingFrame>) {
    let deadline = tokio::time::Instant::now() + budget;
    let mut out = Discovery::default();
    let mut deferred: Vec<IncomingFrame> = Vec::new();

    if let Some(e) = budget_exhausted(deadline, budget) {
        out.error = Some(e);
        return (out, deferred);
    }

    // 1) initialize：拿设备自述（板型/固件版本），并确认设备实现了 MCP 方法分发。
    let id = take_id(next_id);
    if let Err(e) = send_mcp(transport, session_id, discovery::initialize(id, json!({}))).await {
        out.error = Some(format!("initialize 下发失败: {e}"));
        return (out, deferred);
    }
    let (result, frames) = pump_reply(transport, id, remaining(deadline), BinaryPolicy::Keep).await;
    deferred.extend(frames);
    match result {
        Ok(McpResponse::Ok { result, .. }) => out.server = discovery::parse_server_info(&result),
        Ok(McpResponse::Err { code, message, .. }) => {
            out.error = Some(format!("设备拒绝 initialize（code={code}）: {message}"));
            return (out, deferred);
        }
        Err(e) => {
            out.error = Some(format!("initialize 未完成: {e}"));
            return (out, deferred);
        }
    }

    // 2) notifications/initialized：协议要求的收尾通知（无 id、不等回包）。
    // 固件对 notifications/* 直接 return，所以发失败也不影响设备——记 warn 即可。
    if let Err(e) = send_mcp(transport, session_id, discovery::initialized_notification()).await {
        tracing::warn!("session {session_id} 发送 notifications/initialized 失败: {e}");
    }

    // 3) tools/list 分页：cursor 为空 = 第一页；非空 = 上一页的 nextCursor（包含语义）。
    let mut cursor = String::new();
    loop {
        if out.pages >= discovery::MAX_PAGES {
            out.error = Some(format!(
                "tools/list 页数达到上限 {}，已停止（保留已取到的 {} 个工具）",
                discovery::MAX_PAGES,
                out.tools.len()
            ));
            break;
        }
        if let Some(e) = budget_exhausted(deadline, budget) {
            out.error = Some(format!("{e}（已取到 {} 个工具）", out.tools.len()));
            break;
        }
        let id = take_id(next_id);
        if let Err(e) = send_mcp(
            transport,
            session_id,
            discovery::tools_list(id, &cursor, true),
        )
        .await
        {
            out.error = Some(format!("tools/list 下发失败: {e}"));
            break;
        }
        let (result, frames) =
            pump_reply(transport, id, remaining(deadline), BinaryPolicy::Keep).await;
        deferred.extend(frames);
        match result {
            Ok(McpResponse::Ok { result, .. }) => {
                let page = discovery::parse_tools(&result);
                if page.skipped > 0 {
                    // 不静默：缺 name 的条目无法调用，跳过但要让运维看得见
                    tracing::warn!(
                        "session {session_id} tools/list 有 {} 个条目缺 name，已跳过",
                        page.skipped
                    );
                }
                out.tools.extend(page.tools);
                out.pages += 1;
                match page.next_cursor {
                    Some(next) => cursor = next,
                    None => break,
                }
            }
            Ok(McpResponse::Err { code, message, .. }) => {
                out.error = Some(format!("设备拒绝 tools/list（code={code}）: {message}"));
                break;
            }
            Err(e) => {
                out.error = Some(format!(
                    "tools/list 未完成（已取到 {} 个工具）: {e}",
                    out.tools.len()
                ));
                break;
            }
        }
    }

    (out, deferred)
}

/// 自增取号（回绕：u64 现实上到不了，但"宁可溢出 panic 也不降级"不是本仓库的取舍）。
fn take_id(next_id: &mut u64) -> u64 {
    let id = *next_id;
    *next_id = (*next_id).wrapping_add(1);
    id
}

/// 预算用尽时给一句可行动的说明（而不是让下游报"超时 0ms"这种看不懂的数字）。
fn budget_exhausted(deadline: tokio::time::Instant, budget: Duration) -> Option<String> {
    remaining(deadline)
        .is_zero()
        .then(|| format!("MCP 探测预算（{}ms）用尽，已停止", budget.as_millis()))
}

/// 距 deadline 还剩多久（用尽返回 0）。
fn remaining(deadline: tokio::time::Instant) -> Duration {
    deadline
        .checked_duration_since(tokio::time::Instant::now())
        .unwrap_or(Duration::ZERO)
}

/// 发一条 MCP 报文（外层信封由 [`ServerMessage::Mcp`] 负责）。
async fn send_mcp<T: Transport>(
    transport: &mut T,
    session_id: &str,
    payload: Value,
) -> anyhow::Result<()> {
    send_text(
        transport,
        &ServerMessage::Mcp {
            session_id: Some(session_id.to_string()),
            payload,
        },
    )
    .await
}

impl<'a, T: Transport> Session<'a, T> {
    /// 调一次设备工具（MCP `tools/call`）并等待设备结果。
    ///
    /// - 设备未声明 `features.mcp` → [`McpCallError::Unsupported`]（零网络开销，不发任何东西）；
    /// - 设备回了 `error`（未知工具等）**不算错误**：返回 `Ok(McpReply)`，由调用方看
    ///   [`McpReply::succeeded`] 决定是否降级——"设备明确拒绝"和"设备没回应"是两回事，日志要分开。
    pub(super) async fn mcp_call_tool(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Result<McpReply, McpCallError> {
        if !self.mcp_enabled {
            return Err(McpCallError::Unsupported);
        }
        let id = take_id(&mut self.mcp_next_id);
        send_mcp(
            self.transport,
            &self.session_id,
            mcp::tools_call(id, name, arguments),
        )
        .await
        .map_err(|e| McpCallError::Send(e.to_string()))?;

        let (result, deferred) =
            await_tool_reply(self.transport, id, mcp::DEFAULT_CALL_TIMEOUT).await;
        for text in deferred {
            if let Err(e) = self.handle_text(&text).await {
                tracing::warn!(
                    "session {} 等待 MCP 响应期间处理上行消息失败: {e}",
                    self.session_id
                );
            }
        }
        result
    }

    /// 会话开始时的 MCP 能力发现，结果写入 [`crate::app::mcp::registry`]（管理界面数据源）。
    ///
    /// 返回探测期间**缓冲的上行帧**，由 [`Session::run`] 按原顺序补派发——探测恰好发生在
    /// 设备刚开始上行音频的时刻，丢帧就是掐掉用户第一句话的开头。
    pub(super) async fn discover_mcp_tools(&mut self) -> Vec<IncomingFrame> {
        let (found, deferred) = discover_tools(
            self.transport,
            &self.session_id,
            &mut self.mcp_next_id,
            DISCOVERY_BUDGET,
        )
        .await;
        match &found.error {
            None => tracing::info!(
                "session {} MCP 探测完成：{}，工具 {} 个（{} 页）",
                self.session_id,
                found.server.summary(),
                found.tools.len(),
                found.pages
            ),
            Some(e) => tracing::warn!(
                "session {} MCP 探测未完成：{e}（已取到 {} 个工具）",
                self.session_id,
                found.tools.len()
            ),
        }
        self.engines.mcp.record(ProbeResult {
            device_id: self.device_id.clone(),
            session_id: self.session_id.clone(),
            probed_at_ms: registry::now_ms(),
            board: found.server.name,
            firmware_version: found.server.version,
            protocol_version: found.server.protocol_version,
            tools: found.tools,
            error: found.error,
        });
        deferred
    }
}

#[cfg(test)]
mod tests;
