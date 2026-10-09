//! 记忆端点组：状态 / 测试召回 / 立即维护 / 清空。
//!
//! 为什么必须有「测试召回」：记忆是**看不见的**功能，没有当场反馈，用户只会得到
//! "开了没用"的结论（`docs/soul-and-graph-memory-plan.md` §0 结论 3）。这个端点让管理页
//! 输入一句话就能看到：命中几条、匹配多少词项、耗时多少、**将注入什么正文**。
//!
//! 端点：
//! - `GET  /api/memory/status`：状态卡（后端/库大小/条数/待抽取/隔离/各设备作用域 +
//!   能力 `phase` 与降级原因）；
//! - `POST /api/memory/recall`：按一句话试召回（`scope`/`device_id` 可省 → 跨作用域）；
//! - `POST /api/memory/maintain`：立即维护（保留策略 + 隔离释放），可先 `dry_run`；
//! - `POST /api/memory/clear`：清空（**必须**显式给 `scope` 或 `all: true`，防误删）。

use std::sync::Arc;

use axum::{
    extract::{Json, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use crate::engine::Engines;

/// `GET /api/memory/status`。
pub(super) async fn memory_status(State(engines): State<Arc<Engines>>) -> Response {
    let mem = engines.memory();
    let stats = tokio::task::spawn_blocking(move || mem.stats())
        .await
        .unwrap_or_default();
    let cfg = engines.live_config();
    let mut body = stats.to_json();
    // 配置摘要：状态页要能回答"我到底配了什么"，而不必再点开配置页
    body["config"] = serde_json::json!({
        "enabled": cfg.memory.enabled,
        "engine": cfg.memory.engine_id(),
        "db_path": cfg.memory.db_path,
        "recall_enabled": cfg.memory.recall_enabled,
        "extraction_enabled": cfg.memory.extraction_enabled,
        "inject_position": cfg.memory.inject_position().as_str(),
        "recall_max_nodes": cfg.memory.recall_max_nodes,
        "recall_max_tokens": cfg.memory.recall_max_tokens,
        "recall_budget_ms": cfg.memory.recall_budget_ms,
        "fresh_turn_count": cfg.memory.fresh_turn_count,
        "retention": {
            "keep": cfg.memory.retention.keep,
            "recent_turns": cfg.memory.retention.recent_turns,
            "retention_days": cfg.memory.retention.retention_days,
            "dry_run": cfg.memory.retention.dry_run,
        },
    });
    // 能力阶段 + 降级原因（来自插件宿主；UI 据此显示「已启用但降级」）
    body["capability"] = capability_json(&engines, "memory");
    body["aiui_note"] = serde_json::json!(aiui_note(&cfg));
    body["ok"] = serde_json::json!(true);
    json_response(body)
}

/// `POST /api/memory/recall`：`{ "text": "...", "scope": "..."?, "device_id": "..."? }`。
pub(super) async fn memory_recall(
    State(engines): State<Arc<Engines>>,
    Json(raw): Json<serde_json::Value>,
) -> Response {
    let text = raw
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if text.is_empty() {
        return bad_request("请提供 text（要试召回的一句话）");
    }
    let cfg = engines.live_config();
    let scope = raw
        .get("scope")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            raw.get("device_id")
                .and_then(|v| v.as_str())
                .map(|d| cfg.memory.scope_for(d))
        });
    let mem = engines.memory();
    let result = mem.recall(scope.as_deref(), &text).await;
    let mut body = result.to_json();
    body["ok"] = serde_json::json!(true);
    body["scope"] = serde_json::json!(scope);
    body["note"] =
        serde_json::json!("preview 即本轮将注入 system prompt 的正文（作为背景资料，不是指令）");
    json_response(body)
}

/// `POST /api/memory/maintain`：`{ "force": true (默认), "dry_run": false }`。
pub(super) async fn memory_maintain(
    State(engines): State<Arc<Engines>>,
    Json(raw): Json<serde_json::Value>,
) -> Response {
    let force = raw.get("force").and_then(|v| v.as_bool()).unwrap_or(true);
    let dry_run = raw
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mem = engines.memory();
    let r = mem
        .maintain(crate::plugins::memory::MaintainOpts { force, dry_run })
        .await;
    match r {
        Ok(out) => {
            let mut body = out.to_json();
            body["ok"] = serde_json::json!(true);
            json_response(body)
        }
        Err(e) => server_error(&format!("{e:#}")),
    }
}

/// `POST /api/memory/clear`：`{ "scope": "..." }` 或 `{ "all": true }`（强制显式）。
pub(super) async fn memory_clear(
    State(engines): State<Arc<Engines>>,
    Json(raw): Json<serde_json::Value>,
) -> Response {
    let all = raw.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
    let scope = raw
        .get("scope")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if scope.is_none() && !all {
        return bad_request(
            "清空记忆必须显式指定 scope（如 {\"scope\":\"xiaozhi:<device_id>\"}）或 all:true；\
             本端点不支持「什么都不填」的默认清空。",
        );
    }
    let mem = engines.memory();
    match mem.clear(scope.clone()).await {
        Ok(n) => json_response(serde_json::json!({
            "ok": true,
            "deleted": n,
            "scope": scope,
            "all": all,
            "message": format!("已清空 {n} 条轮次记忆{}", if all { "（全部设备）" } else { "" }),
        })),
        Err(e) => server_error(&format!("{e:#}")),
    }
}

/// AIUI 全链路模式下的显式提示：云端闭环绕过本地 LLM，人格/记忆**不生效**。
/// 不提示的话用户会报"人格/记忆配了没用"（`docs/soul-and-graph-memory-plan.md` §9 风险 4）。
fn aiui_note(cfg: &crate::config::Config) -> Option<String> {
    cfg.aiui.enabled.then(|| {
        "⚠️ 当前启用了 AIUI 全链路：对话走云端闭环，本地 LLM 不参与，人格与记忆都不会生效。"
            .to_string()
    })
}

/// 从宿主状态里取某能力的 JSON（含 `phase`/`last_error`）。
fn capability_json(engines: &Engines, id: &str) -> serde_json::Value {
    engines
        .host
        .capabilities_json()
        .get("capabilities")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.iter().find(|c| c["id"] == id))
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

fn json_response(body: serde_json::Value) -> Response {
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        body.to_string(),
    )
        .into_response()
}

fn bad_request(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        json_response(serde_json::json!({ "ok": false, "message": msg })),
    )
        .into_response()
}

fn server_error(msg: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        json_response(serde_json::json!({ "ok": false, "message": msg })),
    )
        .into_response()
}
