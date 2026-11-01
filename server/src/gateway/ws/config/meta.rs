//! Configuration persistence metadata and SQLite administration endpoints.
use super::{json_msg, json_ok};
use crate::runtime::Engines;
use axum::{
    extract::{Json, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) fn config_is_persistent(path: &str) -> bool {
    match std::fs::read_to_string("/proc/self/mountinfo") {
        Ok(mi) => is_persistent_in(&mi, path),
        Err(_) => true,
    }
}
pub(crate) fn is_persistent_in(mountinfo: &str, path: &str) -> bool {
    for line in mountinfo.lines() {
        let mut fields = line.split(' ');
        for _ in 0..4 {
            let _ = fields.next();
        }
        let Some(target) = fields.next() else {
            continue;
        };
        if target == path || path.starts_with(&format!("{target}/")) {
            return true;
        }
    }
    false
}
pub(crate) async fn config_meta(State(engines): State<Arc<Engines>>) -> Response {
    let backend = engines.config_store.backend();
    let path = engines.config_path();
    let (loaded_from, persistent) = match path {
        Some(p) => (backend, Some(config_is_persistent(p))),
        None => ("default", None),
    };
    let hint = match persistent {
        Some(false) => "⚠️ 配置库不在挂载卷上：重建容器会丢失（请挂载 ./server-data:/data）",
        _ => "",
    };
    json_ok(
        serde_json::json!({"config_path":path,"backend":backend,"loaded_from":loaded_from,"persistent":persistent,"hint":hint}),
    )
}
fn db_auth_ok(engines: &Engines, headers: &HeaderMap, query: &HashMap<String, String>) -> bool {
    engines.config.server.expected_token.is_empty()
        || crate::gateway::ws::auth_ok(headers, query, &engines.config.server)
}
pub(crate) async fn db_query(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    if !db_auth_ok(&engines, &headers, &query) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let sql = match body.get("sql").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "请求需含 \"sql\" 字段（如 {\"sql\":\"SELECT * FROM cfg_documents\"}）",
            )
                .into_response()
        }
    };
    match engines.config_store.query(&sql) {
        Ok(rows) => json_ok(serde_json::json!({"rows":rows,"count":rows.len()})),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            json_msg(false, &format!("{e:#}"), None),
        )
            .into_response(),
    }
}
pub(crate) async fn db_backup(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if !db_auth_ok(&engines, &headers, &query) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let Some(loc) = engines.config_store.location() else {
        return (
            StatusCode::BAD_REQUEST,
            json_msg(
                false,
                "内置默认配置无数据库可备份（请用 SQLite 配置后端）。",
                None,
            ),
        )
            .into_response();
    };
    let parent = std::path::Path::new(loc)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let dest = match parent
        .join(format!("xiaozhi-config-backup-{}.db", now_ts()))
        .to_str()
    {
        Some(d) => d.to_string(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                json_msg(false, "生成备份路径失败（路径含非 UTF-8 字符）。", None),
            )
                .into_response()
        }
    };
    match engines.config_store.backup(&dest) {
        Ok(()) => json_ok(
            serde_json::json!({"backup_path":dest,"note":"可将此文件用 sqlite3 / DBeaver / TablePlus 等数据库客户端直接打开查看或导出。"}),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            json_msg(false, &format!("备份数据库失败: {e:#}"), None),
        )
            .into_response(),
    }
}
fn now_ts() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::is_persistent_in;
    #[test]
    fn persistence_mount_matching() {
        let mi = "1 0 8:1 / /data rw - ext4 x\n2 1 8:1 / /etc rw - ext4 x";
        assert!(is_persistent_in(mi, "/data/config.toml"));
        assert!(!is_persistent_in(mi, "/datax/config.toml"));
    }
}
