//! MCP **能力发现**：`initialize` + `tools/list` 的报文构造与解析。
//!
//! 目的：让管理界面能回答「这台 ESP 支持哪些 MCP 工具、每个工具能干什么」。设备是 MCP Server，
//! 工具清单只存在于固件里，服务器不主动问就永远不知道——因此会话建立后要问一次。
//!
//! ## 权威来源（固件逐行核对）
//!
//! - `main/mcp_server.cc` 的 `ParseMessage`：方法分发（`initialize` / `tools/list` / `tools/call`，
//!   其余回 `-32601`）；`notifications/*` **直接 return**（不校验 id、不回包）。
//! - `main/mcp_server.cc` 的 `GetToolsList`：`tools/list` 分页，`max_payload_size = 8000`，
//!   `nextCursor` = **下一页第一个工具的名字**（游标语义为"包含"，非"跳过"）。
//! - `main/mcp_server.h` 的 `McpTool::to_json`：`{name, description, inputSchema}`，
//!   且 `user_only` 工具额外带 `annotations.audience = ["user"]`——**这是判断"是否暴露给 AI"
//!   的唯一可靠信号**（`withUserTools=false` 时固件根本不返回这些工具）。
//!
//! ## 本模块只有纯函数
//!
//! 与 [`super`] 同口径：报文口径可在无网络、无设备、无引擎的条件下单测（`AGENTS.md` §5.10）。
//! I/O 与分页循环在 [`crate::app::session::mcp::discover_tools`]。

use serde_json::{json, Value};

use super::JSONRPC_VERSION;

/// 一次发现最多拉取的页数。
///
/// 固件按 8000 字节分页，而 `tools/list` 全部工具（含 `user_only`）在真机上只有个位数，
/// 正常 1~2 页。这个上界是防"设备回了 `nextCursor` 却不推进"（或恶意设备）导致死循环——
/// 触顶时**保留已取到的工具**并把原因写进 `error`（不静默截断）。
pub const MAX_PAGES: usize = 8;

/// 服务器作为 MCP 客户端声明的协议版本（与固件 `initialize` 回包一致）。
///
/// 固件只在 `params.capabilities.vision` 里取摄像头视觉地址，其余字段一律忽略；
/// 但**按 MCP 规范发完整 `initialize` 参数**才是正确行为（换其它 MCP Server 实现时也成立）。
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// 构造 `initialize` 请求（服务器 → 设备）。
///
/// `client_capabilities` 当前恒为 `{}`：服务器没有可宣告的能力（`vision` 是**设备**要用的
/// 视觉理解地址，由服务器下发而不是宣告；本项目未接摄像头视觉，故不下发）。
pub fn initialize(id: u64, client_capabilities: Value) -> Value {
    json!({
        "jsonrpc": JSONRPC_VERSION,
        "method": "initialize",
        "params": {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": client_capabilities,
            "clientInfo": {
                "name": env!("CARGO_PKG_NAME"),
                "version": env!("CARGO_PKG_VERSION"),
            },
        },
        "id": id,
    })
}

/// 构造 `notifications/initialized` 通知（服务器 → 设备）。
///
/// 通知**没有 `id`**（JSON-RPC 2.0 语义：不期待响应）。固件对 `notifications/*` 直接 return，
/// 因此这条对设备零影响；发它的价值是协议正确性（换实现时不会被当成缺步骤）。
pub fn initialized_notification() -> Value {
    json!({
        "jsonrpc": JSONRPC_VERSION,
        "method": "notifications/initialized",
    })
}

/// 构造 `tools/list` 请求（服务器 → 设备）。
///
/// `cursor` 为空 = 第一页；非空 = 上一页回的 `nextCursor`（**包含**语义：从该工具开始返回）。
/// `with_user_tools = true` 会**一并返回 `user_only` 工具**（管理界面要看全量，
/// 并用 `annotations.audience` 标出哪些不对 AI 暴露）。
pub fn tools_list(id: u64, cursor: &str, with_user_tools: bool) -> Value {
    json!({
        "jsonrpc": JSONRPC_VERSION,
        "method": "tools/list",
        "params": { "cursor": cursor, "withUserTools": with_user_tools },
        "id": id,
    })
}

/// 设备自述（`initialize` 的 `result`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpServerInfo {
    /// 设备声明的 MCP 协议版本（固件硬编码 `2024-11-05`）。
    pub protocol_version: String,
    /// 固件 `serverInfo.name` = **板型名**（`BOARD_NAME` 宏，如 `bread-compact-wifi`）。
    pub name: String,
    /// 固件 `serverInfo.version` = **固件版本**（`esp_app_get_description()->version`）。
    pub version: String,
    /// `capabilities.tools` 存在（设备支持工具能力）。缺失不代表不能调 `tools/list`，
    /// 但缺失时管理界面应提示"设备未声明工具能力"。
    pub tools_capability: bool,
}

impl McpServerInfo {
    /// 一行摘要（日志用）。
    pub fn summary(&self) -> String {
        format!(
            "板型={} 固件={} 协议={}",
            if self.name.is_empty() {
                "(未报)"
            } else {
                &self.name
            },
            if self.version.is_empty() {
                "(未报)"
            } else {
                &self.version
            },
            if self.protocol_version.is_empty() {
                "(未报)"
            } else {
                &self.protocol_version
            },
        )
    }
}

/// 解析 `initialize` 的 `result`。
///
/// **宽容读取**（与信封解析的"严格"相反，这是刻意的）：设备自述缺字段不该让整个发现失败——
/// 拿不到板型/版本只是少一行信息，工具清单才是目的。故所有字段都有默认值，本函数不会失败。
pub fn parse_server_info(result: &Value) -> McpServerInfo {
    let server = result.get("serverInfo");
    let str_at = |v: Option<&Value>, key: &str| -> String {
        v.and_then(|v| v.get(key))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    McpServerInfo {
        protocol_version: result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        name: str_at(server, "name"),
        version: str_at(server, "version"),
        tools_capability: result
            .get("capabilities")
            .and_then(|c| c.get("tools"))
            .is_some_and(Value::is_object),
    }
}

/// 一个设备工具（固件 `McpTool::to_json` 的可读投影）。
#[derive(Debug, Clone, PartialEq)]
pub struct McpToolInfo {
    /// 工具名（跨仓库契约：如 `self.audio_speaker.set_volume`）。
    pub name: String,
    /// 工具描述——**它同时是喂给大模型的提示词**，故管理界面要显示原文。
    pub description: String,
    /// 原始 `inputSchema`（`{type:"object", properties:{…}, required:[…]}`）。
    /// 保留原样而不是拆成字段：客户端的呈现方式（列表/表格）不该反向约束协议层。
    pub input_schema: Value,
    /// `user_only`：固件用 `AddUserOnlyTool` 注册（`annotations.audience` 含 `"user"`），
    /// **默认不出现在 `tools/list` 里、不会被大模型看到**（服务器按名字直接调用）。
    pub user_only: bool,
}

impl McpToolInfo {
    /// `GET /api/mcp/tools` 的一行。
    ///
    /// 除原始 `input_schema` 外还给两份**投影**（服务端是字段唯一权威，客户端不解析 JSON Schema）：
    /// `params` 结构化（将来转 LLM `ToolSpec` 也用得上）与 `signature` 一行可读摘要
    /// （`curl /api/mcp/tools | jq` 就能看懂"这个工具要什么参数"）。
    pub fn to_json(&self) -> Value {
        let params = schema_params(&self.input_schema);
        json!({
            "name": self.name,
            "description": self.description,
            "user_only": self.user_only,
            "signature": signature_of(&params),
            "params": params.iter().map(SchemaParam::to_json).collect::<Vec<_>>(),
            "input_schema": self.input_schema,
        })
    }
}

/// `inputSchema.properties` 里一个参数的**可读投影**。
///
/// 固件每个参数只发这几个键（`main/mcp_server.h:173-205`，逐行核对）：
/// `type`（`boolean` / `integer` / `string`）、`default`、`minimum`/`maximum`（integer）、
/// `maxLength`（string）。⚠️ **没有参数级 description**——固件只给工具级 description，
/// 所以界面不要给参数留"说明"位置（会永远空着）。
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaParam {
    pub name: String,
    /// `boolean` / `integer` / `string`（设备未声明时为 `any`）。
    pub ty: String,
    /// 必填判定 = 没有 `default`（固件 `PropertyList::GetRequired()` 就是这个口径）。
    pub required: bool,
    /// 默认值（JSON 原样）。
    pub default: Option<Value>,
    pub minimum: Option<i64>,
    pub maximum: Option<i64>,
    pub max_length: Option<u64>,
}

impl SchemaParam {
    /// 一行可读摘要：`volume: integer（必填，0~100）`。
    pub fn signature(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.required {
            parts.push("必填".to_string());
        } else if let Some(d) = &self.default {
            parts.push(format!("默认 {}", render_value(d)));
        }
        match (self.minimum, self.maximum) {
            (Some(a), Some(b)) => parts.push(format!("{a}~{b}")),
            (Some(a), None) => parts.push(format!("≥{a}")),
            (None, Some(b)) => parts.push(format!("≤{b}")),
            (None, None) => {}
        }
        if let Some(n) = self.max_length {
            parts.push(format!("最长 {n} 字"));
        }
        if parts.is_empty() {
            format!("{}: {}", self.name, self.ty)
        } else {
            format!("{}: {}（{}）", self.name, self.ty, parts.join("，"))
        }
    }

    /// `GET /api/mcp/tools` 的一行（缺的键一律给 `null`，客户端不必区分"缺失"与"空"）。
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "type": self.ty,
            "required": self.required,
            "default": self.default,
            "minimum": self.minimum,
            "maximum": self.maximum,
            "max_length": self.max_length,
        })
    }
}

/// 从 `inputSchema` 取出参数列表。
///
/// ⚠️ 顺序是**字母序**而不是固件声明序：`serde_json` 默认不开 `preserve_order`，
/// 对象键在解析时即按 BTreeMap 排序。对 1~3 个参数的工具来说这只影响观感，
/// 换来的是"不为了展示顺序给整个服务端开启一个全局 feature"。
pub fn schema_params(schema: &Value) -> Vec<SchemaParam> {
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    props
        .iter()
        .map(|(name, p)| SchemaParam {
            name: name.clone(),
            ty: p
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("any")
                .to_string(),
            required: required.contains(&name.as_str()),
            default: p.get("default").cloned(),
            minimum: p.get("minimum").and_then(Value::as_i64),
            maximum: p.get("maximum").and_then(Value::as_i64),
            max_length: p.get("maxLength").and_then(Value::as_u64),
        })
        .collect()
}

/// 整个工具的一行参数摘要（无参数时给一句明确的话，而不是空串）。
fn signature_of(params: &[SchemaParam]) -> String {
    if params.is_empty() {
        return "（无参数）".to_string();
    }
    params
        .iter()
        .map(SchemaParam::signature)
        .collect::<Vec<_>>()
        .join("；")
}

/// 默认值的可读形式（字符串加引号，免得空串看起来像"没有默认值"）。
fn render_value(v: &Value) -> String {
    match v {
        Value::String(s) => format!("\"{s}\""),
        other => other.to_string(),
    }
}

/// 一页 `tools/list` 的解析结果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolsPage {
    pub tools: Vec<McpToolInfo>,
    /// 非空 = 还有下一页（值 = 下一页第一个工具名，游标**包含**该工具）。
    pub next_cursor: Option<String>,
    /// 被跳过的畸形条目数（缺 `name` 的项无法调用，只能跳过）。
    /// **必须回报**：静默丢工具会让管理界面显示的能力与实际不符（本仓库忌讳静默不正确）。
    pub skipped: usize,
}

/// 解析 `tools/list` 的 `result`（`tools[]` + `nextCursor`）。
pub fn parse_tools(result: &Value) -> ToolsPage {
    let mut page = ToolsPage::default();
    if let Some(items) = result.get("tools").and_then(Value::as_array) {
        for item in items {
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            if name.is_empty() {
                page.skipped += 1;
                continue;
            }
            page.tools.push(McpToolInfo {
                name,
                description: item
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                input_schema: item
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
                user_only: is_user_only(item),
            });
        }
    }
    page.next_cursor = result
        .get("nextCursor")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string);
    page
}

/// `annotations.audience` 是否含 `"user"`（固件 `McpTool::to_json` 对 `user_only` 工具的唯一标记）。
fn is_user_only(item: &Value) -> bool {
    item.get("annotations")
        .and_then(|a| a.get("audience"))
        .and_then(Value::as_array)
        .is_some_and(|audience| audience.iter().any(|v| v.as_str() == Some("user")))
}

#[cfg(test)]
mod tests;
