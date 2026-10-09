//! 令牌用量端点（`docs/plugin-architecture-unification.md` §4.3）。
//!
//! - `GET /api/usage`：全局累计 + 最近各会话（最多 64 条）
//! - `GET /api/session/{id}/usage`：单个会话
//!
//! 只报令牌数，**不做金额换算**（单价随模型/渠道变化）。价值在于回答两个此前无从回答的问题：
//! 成本多少、以及"记忆/灵魂注入让 prompt 长了多少"（`usage.pressure` = 输入侧总量，不含输出）。

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use crate::engine::Engines;

/// `GET /api/usage`
pub(super) async fn usage(State(engines): State<Arc<Engines>>) -> Response {
    let mut body = engines.usage.to_json();
    body["ok"] = serde_json::json!(true);
    json_response(StatusCode::OK, body)
}

/// `GET /api/session/{id}/usage`
pub(super) async fn session_usage(
    State(engines): State<Arc<Engines>>,
    Path(session_id): Path<String>,
) -> Response {
    match engines.usage.session(&session_id) {
        Some(snapshot) => {
            let mut body = snapshot.to_json(Some(&session_id));
            body["ok"] = serde_json::json!(true);
            json_response(StatusCode::OK, body)
        }
        None => json_response(
            StatusCode::NOT_FOUND,
            serde_json::json!({
                "ok": false,
                "session_id": session_id,
                "message": "没有该会话的用量记录：会话 id 拼写错误，或该会话尚未产生任何已计量的轮次（上游可能未返回 usage），或已被淘汰（只保留最近 64 个会话）。",
            }),
        ),
    }
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
