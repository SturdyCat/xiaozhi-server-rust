//! 文件管理的 **HTTP 层**（路由 + 处理器）。
//!
//! 路径校验、受保护判定与操作契约（状态码决策）都在父模块 [`super`]（纯函数可直测）；
//! 本模块只做参数提取、[`auth_ok`] 鉴权与 JSON 形状。错误一律 `{"error":"…"}`。
//!
//! 端点（契约冻结，Kotlin 客户端按此实现）：
//! - `GET  /api/files?path=`：列表（path 缺省 = 根）；
//! - `POST /api/files/mkdir`：建目录（body `{"path"}`，父目录一并建，幂等）；
//! - `POST /api/files/upload?path=&overwrite=`：上传裸字节（可 `?token=` 走查询鉴权）；
//! - `POST /api/files/delete`（body `{"path"}`）/ `DELETE /api/files?path=`：删除；
//! - `GET  /api/files/download?path=`：裸字节下载（`Content-Disposition: attachment`）。
//!
//! 鉴权口径与 `/api/config` 完全一致（[`auth_ok`]）：`expected_token` 为空则不校验；
//! EchoMuse 开关**不**作用于此处（通用存储管理）。

use super::*;

use std::collections::HashMap;

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Query, State},
    http::{header, HeaderMap},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde_json::Value;

use crate::gateway::ws::auth_ok;

/// 删除/建目录的 body 只是 `{"path"}`，给个小上限防滥用（上传上限在 upload 路由）。
const SMALL_BODY_LIMIT: usize = 64 * 1024;

fn json_headers() -> [(header::HeaderName, &'static str); 1] {
    [(header::CONTENT_TYPE, "application/json; charset=utf-8")]
}

/// 统一错误形状：`{"error":"…"}` + 契约状态码。
fn err(op: super::OpError) -> Response {
    (
        op.status,
        json_headers(),
        serde_json::json!({ "error": op.message }).to_string(),
    )
        .into_response()
}

fn ok_json(value: Value) -> Response {
    (json_headers(), value.to_string()).into_response()
}

/// 提取并校验 query 里的相对路径；`missing` = 参数缺失/空白时的文案。
fn rel_from_query(
    query: &HashMap<String, String>,
    missing: &str,
) -> Result<String, super::OpError> {
    match query
        .get("path")
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty())
    {
        Some(raw) => validate_rel(raw).map_err(bad),
        None => Err(bad(missing)),
    }
}

// ============================================================
// 处理器
// ============================================================

/// `GET /api/files?path=`：列目录（path 缺省 = 根）。响应
/// `{"root":"/abs/root","path":<规整后 rel>,"entries":[{name,dir,size,modified,protected}]}`，
/// entries 目录在前、按名称排序；`modified` 为 Unix 秒。
async fn list<S: Context>(
    State(ctx): State<S>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if !auth_ok(&headers, &query, &ctx.config().server) {
        return err(super::OpError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        ));
    }
    let rel = match query
        .get("path")
        .map(|raw| validate_rel(raw))
        .unwrap_or(Ok(String::new()))
    {
        Ok(rel) => rel,
        Err(message) => return err(bad(message)),
    };
    let root = storage_root(ctx.config());
    match op_list(&root, &rel) {
        Ok((root, entries)) => {
            let entries: Vec<Value> = entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "name": e.name,
                        "dir": e.dir,
                        "size": e.size,
                        "modified": e.modified,
                        "protected": e.protected,
                    })
                })
                .collect();
            ok_json(serde_json::json!({
                "root": root.display().to_string(),
                "path": rel,
                "entries": entries,
            }))
        }
        Err(e) => err(e),
    }
}

/// `POST /api/files/mkdir`：body `{"path":"a/b"}`。父目录一并创建；目录已存在也
/// 200（幂等）；同名文件已存在 409。
async fn mkdir<S: Context>(
    State(ctx): State<S>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    if !auth_ok(&headers, &query, &ctx.config().server) {
        return err(super::OpError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        ));
    }
    let raw = match json_field(&body, "path") {
        Ok(raw) => raw,
        Err(e) => return err(e),
    };
    let rel = match validate_rel(&raw) {
        Ok(rel) => rel,
        Err(message) => return err(bad(message)),
    };
    let root = storage_root(ctx.config());
    match op_mkdir(&root, &rel) {
        Ok(()) => ok_json(serde_json::json!({ "ok": true, "path": rel })),
        Err(e) => err(e),
    }
}

/// `POST /api/files/upload?path=<必填>&overwrite=<"1"/"true">`：body 为裸文件字节。
/// 自动建父目录；已存在目标未带 overwrite 409；根目录/受保护名/空 body/超限各自 400/
/// 403/400/413。成功 `{"ok":true,"path":<rel>,"size":N}`。
async fn upload<S: Context>(
    State(ctx): State<S>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    if !auth_ok(&headers, &query, &ctx.config().server) {
        return err(super::OpError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        ));
    }
    if body.is_empty() {
        return err(bad("请求体为空（应上传文件原始字节）"));
    }
    if body.len() > MAX_FILE_BYTES {
        return err(OpError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("文件超过上限 {}MB", MAX_FILE_BYTES / 1024 / 1024),
        ));
    }
    let rel = match rel_from_query(&query, "缺少 path 查询参数（上传目标，相对存储根）")
    {
        Ok(rel) if !rel.is_empty() => rel,
        Ok(_) => return err(bad("上传目标不能是根目录")),
        Err(e) => return err(e),
    };
    let overwrite = query
        .get("overwrite")
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    let root = storage_root(ctx.config());
    match op_upload(&root, &rel, &body, overwrite) {
        Ok(size) => {
            if rel.split('/').next() == Some(OWW_MODELS_DIR) {
                tracing::info!("唤醒词模型已上传（设备设置页下拉可列出）: {rel}（{size}B）");
            } else {
                tracing::info!("文件已上传: {rel}（{size}B）");
            }
            ok_json(serde_json::json!({ "ok": true, "path": rel, "size": size }))
        }
        Err(e) => err(e),
    }
}

/// `POST /api/files/delete`（body `{"path"}`，Kuikly 客户端入口）与
/// `DELETE /api/files?path=`（curl/脚本）同一处理器。query 与 body 同时给出时以
/// query 为准；删除目录即递归。
async fn delete_entry<S: Context>(
    State(ctx): State<S>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    if !auth_ok(&headers, &query, &ctx.config().server) {
        return err(super::OpError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        ));
    }
    let raw = match query
        .get("path")
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .map(Ok)
        .unwrap_or_else(|| json_field(&body, "path"))
    {
        Ok(raw) => raw,
        Err(e) => return err(e),
    };
    let rel = match validate_rel(&raw) {
        Ok(rel) => rel,
        Err(message) => return err(bad(message)),
    };
    let root = storage_root(ctx.config());
    match op_delete(&root, &rel) {
        Ok(()) => {
            tracing::info!("文件/目录已删除: {rel}");
            ok_json(serde_json::json!({ "ok": true }))
        }
        Err(e) => err(e),
    }
}

/// `GET /api/files/download?path=<必填>`：裸字节 + `Content-Disposition: attachment`。
async fn download<S: Context>(
    State(ctx): State<S>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if !auth_ok(&headers, &query, &ctx.config().server) {
        return err(super::OpError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
        ));
    }
    let rel = match rel_from_query(&query, "缺少 path 查询参数（下载目标，相对存储根）")
    {
        Ok(rel) if !rel.is_empty() => rel,
        Ok(_) => return err(bad("下载目标不能是根目录")),
        Err(e) => return err(e),
    };
    let root = storage_root(ctx.config());
    match op_download(&root, &rel) {
        Ok((name, bytes)) => {
            // 头部值防注入：引号/换行/反斜杠替换为下划线（文件名只是提示，不是寻址依据）
            let safe: String = name
                .chars()
                .map(|c| match c {
                    '"' | '\r' | '\n' | '\\' => '_',
                    _ => c,
                })
                .collect();
            let mut resp =
                ([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response();
            let disposition =
                header::HeaderValue::from_str(&format!("attachment; filename=\"{safe}\""))
                    .unwrap_or(header::HeaderValue::from_static("attachment"));
            resp.headers_mut()
                .insert(header::CONTENT_DISPOSITION, disposition);
            resp
        }
        Err(e) => err(e),
    }
}

/// 读 body JSON 的字符串字段；空/缺失/非字符串统一 `Err(400)`（含 body 非 JSON）。
fn json_field(body: &[u8], key: &str) -> Result<String, OpError> {
    if body.is_empty() {
        return Err(bad(format!("请求体为空（应为目标 JSON {{\"{key}\":…}}）")));
    }
    let value: Value =
        serde_json::from_slice(body).map_err(|e| bad(format!("请求体不是合法 JSON: {e}")))?;
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| bad(format!("请求体缺少字符串字段 \"{key}\"")))
}

// ============================================================
// 路由
// ============================================================

/// 文件管理子路由（并入主 router）。Kuikly NetworkModule 只有 GET/POST：
/// 删除另给 `POST /api/files/delete` 入口（与 OTA 固件删除同一惯例）。
pub(super) fn router<S: Context>() -> Router<S> {
    Router::new()
        .route("/api/files", get(list::<S>).delete(delete_entry::<S>))
        .route(
            "/api/files/mkdir",
            post(mkdir::<S>).layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)),
        )
        .route(
            "/api/files/upload",
            post(upload::<S>).layer(DefaultBodyLimit::max(MAX_FILE_BYTES)),
        )
        .route("/api/files/download", get(download::<S>))
        .route(
            "/api/files/delete",
            post(delete_entry::<S>).layer(DefaultBodyLimit::max(SMALL_BODY_LIMIT)),
        )
}
