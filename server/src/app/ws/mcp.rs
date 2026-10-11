//! 设备 MCP 工具端点（管理界面「MCP 工具」页的数据源）。
//!
//! - `GET /api/mcp/tools`：按设备列出**最近一次能力发现**的结果——工具名、描述（= 喂给大模型的
//!   提示词原文）、`inputSchema`、是否 `user_only`（不对 AI 暴露），以及板型 / 固件版本 /
//!   协议版本 / 探测时间 / 是否在线。
//!
//! 为什么要有这个端点：MCP 工具清单**只存在于固件里**（`main/boards/<板>/<板>.cc` 的
//! `InitializeTools()` 里 `AddTool`），服务器不问就不知道；而"这台设备到底能被 AI 指挥做什么"
//! 是排查"配了没用"的第一手信息——此前只能翻设备串口日志或读固件源码。
//!
//! ⚠️ **只读投影**：不改任何运行期行为，也不参与配置回传（与 `GET /api/plugins` 同契约）。
//! 数据来自**会话开始时的探测**（[`crate::app::session::mcp::discover_tools`]），
//! 因此设备空闲时这里显示的是上次结果（带 `age_ms` 与 `online` 供判断新鲜度）。

use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use crate::engine::Engines;

/// `GET /api/mcp/tools`
pub(super) async fn mcp_tools(State(engines): State<Arc<Engines>>) -> Response {
    let mut body = engines.mcp.to_json();
    body["ok"] = serde_json::json!(true);
    json_response(StatusCode::OK, body)
}

fn json_response(code: StatusCode, body: serde_json::Value) -> Response {
    match serde_json::to_string(&body) {
        Ok(json) => (
            code,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            json,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("序列化失败: {e}"),
        )
            .into_response(),
    }
}
