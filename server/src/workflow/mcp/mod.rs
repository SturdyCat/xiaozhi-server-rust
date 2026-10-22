//! MCP（Model Context Protocol）客户端原语：**设备是 MCP Server，服务器是 MCP Client**。
//!
//! ## 权威来源
//!
//! - `../xiaozhi-esp32/docs/mcp-protocol_zh.md`（时序图与报文样例）
//! - `../xiaozhi-esp32/main/mcp_server.cc`（`ParseMessage` / `ReplyResult` / `ReplyError` 的实际形态）
//!
//! ## 传输：不新增任何连接
//!
//! JSON-RPC 2.0 消息套在既有那条 WS/MQTT 上（固件 `Protocol::SendMcpMessage`）：
//!
//! ```text
//! server → device: {"session_id":"…","type":"mcp","payload":{"jsonrpc":"2.0","method":"tools/call", …}}
//! device → server: {"session_id":"…","type":"mcp","payload":{"jsonrpc":"2.0","id":1,"result":{…}}}
//! ```
//!
//! 对应 [`crate::workflow::protocol::ServerMessage::Mcp`] 与
//! [`crate::workflow::protocol::ClientMessage::Mcp`]。**方向不能反**：设备在 NAT 后，只能由设备先建连、
//! 服务器在既有通道上发 `tools/call`。
//!
//! ## 设备侧确认过的报文形态（固件 `mcp_server.cc:478-499` / `mcp_server.h:349-419`）
//!
//! 成功：`{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"true"}],"isError":false}}`
//! 失败：`{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Unknown tool: …"}}`
//!
//! ⚠️ 设备对**未知方法**回 `-32601`、对**未知工具**回 `-32602`，都是**正常响应而非断连**：
//! 老固件没有某个工具时，正是靠这条错误识别出来并降级（见 [`crate::workflow::session::command`]）。
//! 同理，`id` 在固件侧被强制要求是**数字**（`mcp_server.cc:409`），字符串 id 会被整条丢弃。
//!
//! ## 模块边界
//!
//! 本模块**只有纯函数**（构造请求 / 解析响应 / 从信封里取配对响应），I/O 与请求-响应关联在
//! [`crate::workflow::session::mcp`]。这样协议口径可以在无网络、无引擎的条件下单测
//! （拆分的另一原因见 `AGENTS.md` §5.10）。
//!
//! ## 两类方法，两种响应视图
//!
//! - `tools/call` 的 `result` 是**工具返回值**（`content[]` 包装），压成
//!   [`McpReply`]（`Ok`/`Err` 两档）即可；
//! - `initialize` / `tools/list` 的 `result` 是**结构化对象**（`serverInfo` / `tools[]`），
//!   压成文本就丢了信息，故保留原始 `result` → [`McpResponse`]。
//!   报文构造与解析在 [`discovery`]。
//!
//! 两者共用同一个信封解析入口 [`parse_response`]，保证"什么算一条响应"只有一处口径。

use std::time::Duration;

use serde_json::{json, Value};

use crate::workflow::protocol::ClientMessage;

pub(crate) mod discovery;
pub(crate) mod registry;

/// JSON-RPC 版本串。固件 `McpServer::ParseMessage` **严格校验**它，写错整条消息会被直接丢弃
/// （且设备只打一行 log，不回错误响应——表现为"服务器发了但没反应"）。
pub const JSONRPC_VERSION: &str = "2.0";

/// 「结束聆听」工具名：server → device 的**跨仓库协议契约**。
///
/// 对应固件 `main/mcp_server.cc` 里注册的 `self.audio_listener.stop`（在 `AddUserOnlyTools` 中，
/// 因此默认不出现在 `tools/list` 里——它是**服务器**用的控制指令，不该让大模型自己决定"别听了"）。
/// 改名必须两边同步：只改一边的后果是设备回 `-32602 Unknown tool`，服务器按"不支持"降级为
/// 断开连接（安全但功能退化，日志里有 warn 可查）。
pub const STOP_LISTENING_TOOL: &str = "self.audio_listener.stop";

/// 一次 `tools/call` 的等待上限。
///
/// 2s 是"远大于设备本地执行耗时（几十微秒）+ 一个网络 RTT，又远小于用户能察觉的卡顿"的取值。
/// 超时不是致命错误：调用方会降级为「断开本次会话」（= 加 MCP 之前的行为）。
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_millis(2_000);

/// 一次工具调用的结果（已把 JSON-RPC 的 `result` / `error` 归一成两档）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpOutcome {
    /// 设备执行了工具。`is_error` 取自固件固定回填的 `result.isError`
    /// （当前固件恒为 `false`：工具回调失败时走的是 JSON-RPC `error` 分支）。
    Ok {
        /// `content[]` 里第一段 `type:"text"` 的正文（工具返回值；图片结果时为 `None`）。
        text: Option<String>,
        /// `result.isError`。
        is_error: bool,
    },
    /// 设备拒绝：未知工具 / 参数不合法 / 工具回调返回 `unexpected`。
    Err {
        /// JSON-RPC 错误码（`-32601` 未知方法、`-32602` 参数或未知工具…）。
        code: i64,
        /// 设备给的可读原因（会进日志，便于定位"工具名写错了"这类问题）。
        message: String,
    },
}

impl McpOutcome {
    /// 设备是否**成功执行**了工具（`error` 分支与 `isError:true` 都不算）。
    pub const fn succeeded(&self) -> bool {
        matches!(
            self,
            McpOutcome::Ok {
                is_error: false,
                ..
            }
        )
    }
}

/// 一次工具调用的响应：`id` 用于与请求配对（见 [`crate::workflow::session::mcp`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpReply {
    pub id: u64,
    pub outcome: McpOutcome,
}

impl McpReply {
    /// 设备是否成功执行了工具。
    pub const fn succeeded(&self) -> bool {
        self.outcome.succeeded()
    }

    /// 一行可读摘要（日志用；**不泄露密钥**——工具结果只可能来自设备自身状态）。
    pub fn summary(&self) -> String {
        match &self.outcome {
            McpOutcome::Ok { text, is_error } => match (text, is_error) {
                (Some(t), false) => format!("id={} ok: {t}", self.id),
                (Some(t), true) => format!("id={} 工具报错(isError=true): {t}", self.id),
                (None, false) => format!("id={} ok", self.id),
                (None, true) => format!("id={} 工具报错(isError=true)", self.id),
            },
            McpOutcome::Err { code, message } => {
                format!("id={} 设备拒绝（code={code}）: {message}", self.id)
            }
        }
    }
}

/// 设备侧一条 MCP 响应，**保留原始 `result`**（供 `initialize` / `tools/list` 这类
/// 返回结构化对象的方法使用）。
///
/// 与 [`McpReply`] 的关系：`tools/call` 只关心"执行成功/失败 + 返回文本"，用 `McpReply`；
/// 能力发现要读 `result` 里的字段，故需要这一档。[`McpResponse::into_reply`] 是两者的桥梁，
/// 信封解析（[`parse_response`]）是共同入口——口径不会漂移。
#[derive(Debug, Clone, PartialEq)]
pub enum McpResponse {
    /// 设备执行了方法，`result` 为**原始** JSON（未做任何裁剪）。
    Ok { id: u64, result: Value },
    /// 设备拒绝：未知方法 / 参数不合法 / 内部分页失败…
    Err { id: u64, code: i64, message: String },
}

impl McpResponse {
    /// 与请求配对的 JSON-RPC `id`。
    pub const fn id(&self) -> u64 {
        match self {
            McpResponse::Ok { id, .. } | McpResponse::Err { id, .. } => *id,
        }
    }

    /// 压成 `tools/call` 视图（`result` → `content[]` 第一段文本）。
    pub fn into_reply(self) -> McpReply {
        match self {
            McpResponse::Ok { id, result } => McpReply {
                id,
                outcome: McpOutcome::Ok {
                    text: first_text(&result),
                    is_error: result
                        .get("isError")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                },
            },
            McpResponse::Err { id, code, message } => McpReply {
                id,
                outcome: McpOutcome::Err { code, message },
            },
        }
    }
}

/// 构造一次 `tools/call` 请求（`payload` 部分，外层信封由
/// [`crate::workflow::protocol::ServerMessage::Mcp`] 负责）。
///
/// `arguments` 必须是对象：固件 `mcp_server.cc:466` 对非对象的 `arguments` 直接回
/// `-32602 Invalid arguments: expected object`。无参工具传 `json!({})`（**不能省略**：
/// 省略是合法的，但显式给空对象让抓包更可读，且与固件 `tool_arguments != nullptr` 分支一致）。
pub fn tools_call(id: u64, name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": JSONRPC_VERSION,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
        "id": id,
    })
}

/// [`parse_response`] 的 **`tools/call` / 日志视图**（把 `result` 压成 `content[]` 文本）。
///
/// 能力发现（`initialize` / `tools/list`）要读 `result` 里的字段，请直接用 [`parse_response`]。
pub fn parse_reply(payload: &Value) -> Option<McpReply> {
    parse_response(payload).map(McpResponse::into_reply)
}

/// 解析设备侧的 MCP 响应，**保留原始 `result`**——信封解析的**唯一出口**。
///
/// 宽容度刻意收得很紧：固件 `ReplyResult`/`ReplyError` 的形态是固定的，放宽解析只会把
/// "设备发了别的东西"误判成"方法调用成功"——那正是本仓库最忌讳的静默不正确。
pub fn parse_response(payload: &Value) -> Option<McpResponse> {
    let obj = payload.as_object()?;
    if obj.get("jsonrpc")?.as_str()? != JSONRPC_VERSION {
        return None;
    }
    // 固件强制数字 id（`mcp_server.cc:409`）；字符串 id 一律不认。
    let id = obj.get("id")?.as_u64()?;

    if let Some(err) = obj.get("error") {
        let err = err.as_object()?;
        return Some(McpResponse::Err {
            id,
            // 固件两处 error 一定带 code；缺 code 的畸形报文按"内部错误"处理而不是丢弃——
            // 丢弃会让调用方一直等到超时，把"设备明确报错"误报成"设备没回应"。
            code: err.get("code").and_then(Value::as_i64).unwrap_or(-32603),
            message: err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("（设备未给出原因）")
                .to_string(),
        });
    }

    Some(McpResponse::Ok {
        id,
        result: obj.get("result")?.clone(),
    })
}

/// 从一条**完整的上行文本帧**里取出与 `id` 配对的 MCP 响应，保留**原始 `result`**。
///
/// 返回 `None` 有两种情况，调用方都必须当作"这不是我要的响应"继续等：
/// ① 不是 `type:"mcp"` 的消息（`listen`/`abort`/音频…）；② 是 mcp 但 `id` 不匹配（迟到的上一次响应）。
///
/// `tools/call` 路径用 [`McpResponse::into_reply`] 压成 [`McpReply`]；本函数是信封解析的唯一出口
/// （[`parse_reply`] 只是 `summarize` 的日志视图，走的也是同一条链）。
pub fn take_response_for(text: &str, id: u64) -> Option<McpResponse> {
    let msg: ClientMessage = serde_json::from_str(text).ok()?;
    match msg {
        ClientMessage::Mcp { payload, .. } => {
            let reply = parse_response(&payload)?;
            (reply.id() == id).then_some(reply)
        }
        _ => None,
    }
}

/// 一行可读摘要（日志用）。
///
/// 设备侧 MCP 消息有两类：**响应**（能配对到 `id`）与**通知**（`notifications/*`，无 `id`）。
/// 日志里只需要"这是哪一类、针对哪个 id、结果如何"，把整包 payload 打出来会淹没现场。
pub fn summarize(payload: &Value) -> String {
    if let Some(reply) = parse_reply(payload) {
        return format!("响应 {}", reply.summary());
    }
    let method = payload
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("(无 method)");
    match payload.get("id").and_then(Value::as_u64) {
        Some(id) => format!("通知/请求 method={method} id={id}"),
        None => format!("通知/请求 method={method}"),
    }
}

/// 取 `result.content[]` 里第一段文本（固件把 `bool`/`int`/`string`/`cJSON` 都包成
/// `{"type":"text","text":"…"}`；图片结果是 `{"type":"image", …}`，故这里返回 `None`）。
fn first_text(result: &Value) -> Option<String> {
    result
        .get("content")?
        .as_array()?
        .iter()
        .find(|item| item.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests;
