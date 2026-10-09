//! 能力清单与配置元数据端点（`docs/plugin-architecture-unification.md` §3.4）。
//!
//! - `GET /api/plugins`：能力 + 实现 + **实际状态**（`enabled` 与 `phase` 分离，
//!   并带 `last_error`）。此前管理页只能靠"保存后能不能用"猜后端，运行期失败全部静默。
//! - `GET /api/config/schema`：字段元数据（标签/类型/默认值提示/热生效语义/是否密钥）。
//!
//! ⚠️ 两者都是**只读投影**：不改变任何运行期行为（P1 的契约），也不做自动表单布局
//! （见 `docs/plugin-architecture-unification.md` §11.1：DSH 的 `autoGenerate` 至今没有
//! 客户端使用，我们只借"服务端作为字段唯一权威"这一点）。

use std::sync::Arc;

use axum::{
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};

use crate::app::ws::config::config_is_persistent;
use crate::engine::Engines;
use crate::plugins::registry;

/// `GET /api/plugins`：能力清单 + 状态。
pub(super) async fn list_plugins(State(engines): State<Arc<Engines>>) -> Response {
    let mut body = engines.host.capabilities_json();
    body["ok"] = serde_json::json!(true);
    // 配置来源一并带上：状态解读常需知道"这些值来自哪个文件、重启会不会丢"
    body["config"] = serde_json::json!({
        "path": engines.config_path,
        "persistent": engines.config_path.as_deref().map(config_is_persistent),
    });
    json_response(body)
}

/// `GET /api/config/schema`：字段元数据（按能力聚合）。
pub(super) async fn config_schema(State(_engines): State<Arc<Engines>>) -> Response {
    let mut body = registry::schema_json();
    body["ok"] = serde_json::json!(true);
    json_response(body)
}

fn json_response(body: serde_json::Value) -> Response {
    match serde_json::to_string(&body) {
        Ok(json) => (
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            json,
        )
            .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("序列化失败: {e}"),
        )
            .into_response(),
    }
}
