//! 固件托管的 **HTTP 层**（路由 + 四个处理器）。
//!
//! 拆出来的理由：逻辑文件 ≤ 600 行（见仓库根 `AGENTS.md`）；同时"存储/匹配"（父模块 `firmware.rs`）
//! 与"HTTP 契约"（本模块）本来就是两件事——前者要能被单元测试直接调，后者才关心状态码与鉴权。
//!
//! 四个端点（契约见 `server/src/app/firmware/AGENTS.md`，原功能长文 §6）：
//! - `POST /api/ota/firmware`：上传 app bin（body = 裸二进制；管理动作，需鉴权）；
//! - `GET  /api/ota/firmware`：托管清单（管理页数据源）；
//! - `GET  /api/ota/firmware/latest?project=&model=&board=`：设备下载（**不鉴权**）；
//! - `POST /api/ota/firmware/delete` / `DELETE /api/ota/firmware`：删除（管理动作，需鉴权）。
//!
//! 鉴权口径与 `/api/config` 完全一致（[`auth_ok`]）：`expected_token` 为空则不校验。

use super::*;

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use sha2::{Digest, Sha256};

use crate::gateway::ws::auth_ok;
use crate::runtime::Engines;

fn json_headers() -> [(header::HeaderName, &'static str); 1] {
    [(header::CONTENT_TYPE, "application/json; charset=utf-8")]
}

// ============================================================
// HTTP 处理器
// ============================================================

/// 上传固件：`POST /api/ota/firmware`，body 为裸 app bin。
/// 型号/功能/版本**优先从包内元数据解析**，查询参数 `project`/`model`/`board`/`version`
/// 仅作兜底覆盖（非 ESP-IDF 镜像或字段为空时）。**同 key 上传即覆盖**。
async fn upload(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    let server = &engines.config.server;
    if !auth_ok(&headers, &query, server) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    if body.is_empty() {
        return (StatusCode::BAD_REQUEST, "请求体为空（应上传 app bin）").into_response();
    }
    if body.len() > MAX_FIRMWARE_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("固件超过上限 {}MB", MAX_FIRMWARE_BYTES / 1024 / 1024),
        )
            .into_response();
    }

    let parsed = parse_app_image(&body);
    let (project, model, board, version) = match resolve_target(&query, parsed.as_ref()) {
        Ok(v) => v,
        Err(msg) => return (StatusCode::BAD_REQUEST, msg).into_response(),
    };

    let sha = {
        let mut h = Sha256::new();
        h.update(&body);
        hex(&h.finalize())
    };
    let size = body.len() as u64;
    let dir = entry_dir(&firmware_dir(), &project, &model, &board);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("创建固件目录 {} 失败: {e}", dir.display()),
        )
            .into_response();
    }
    if let Err(e) = write_atomic(&dir.join(BIN_FILENAME), &body) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("写入固件失败: {e}"),
        )
            .into_response();
    }
    let meta = FirmwareMeta {
        project,
        model,
        board,
        version,
        size,
        sha256: sha,
        idf_ver: parsed
            .as_ref()
            .map(|p| p.idf_ver.clone())
            .unwrap_or_default(),
        built_at: parsed
            .as_ref()
            .map(|p| p.built_at.clone())
            .unwrap_or_default(),
        uploaded_at: now_secs(),
    };
    let meta_json = serde_json::to_string_pretty(&meta).unwrap_or_default();
    if let Err(e) = write_atomic(&dir.join(META_FILENAME), meta_json.as_bytes()) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("固件已写入但元数据更新失败: {e}"),
        )
            .into_response();
    }
    tracing::info!(
        "固件已托管：project={} model={} board={} version={} size={} sha256={}（{}）",
        meta.project,
        meta.model,
        if meta.board.is_empty() {
            "（通用）"
        } else {
            &meta.board
        },
        meta.version,
        meta.size,
        meta.sha256,
        dir.display()
    );
    let mut resp = meta.to_json();
    if let Some(v) = resp.as_object_mut() {
        // 型号/版本是否来自包内元数据（管理页据此提示"可人工覆盖"）
        v.insert(
            "metadata_from".into(),
            serde_json::json!(if parsed.is_some() { "package" } else { "query" }),
        );
        v.insert("path".into(), serde_json::json!(dir.display().to_string()));
    }
    (json_headers(), resp.to_string()).into_response()
}

/// 定出这次上传的托管坐标：**查询参数优先，包内元数据兜底**。
///
/// 抽成纯函数是为了能直接单测（HTTP 处理器需要 `Engines` 才能构造，跑不起来）：
/// 覆盖优先级与"缺字段怎么报错"是上传契约里最容易写错的两件事。
/// 返回 `Err` 时是给用户看的**可行动**文案（说清缺什么、怎么补）。
fn resolve_target(
    query: &HashMap<String, String>,
    parsed: Option<&AppImageInfo>,
) -> Result<(String, String, String, String), String> {
    // 查询参数 > 包内元数据（显式覆盖优先；**空串/空白视为"未给"**，回落包内元数据——
    // 客户端或 curl 传 `?version=` 时不该把可识别的包变成"无法确定版本"）
    let pick = |key: &str, from_pkg: Option<&str>| -> Option<String> {
        query
            .get(key)
            .map(String::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| from_pkg.map(str::trim).filter(|s| !s.is_empty()))
            .map(str::to_owned)
    };
    let project =
        pick("project", parsed.map(|p| p.project.as_str())).and_then(|s| sanitize_segment(&s));
    let model = pick("model", parsed.map(|p| p.model.as_str())).and_then(|s| sanitize_segment(&s));
    let version =
        pick("version", parsed.map(|p| p.version.as_str())).and_then(|s| sanitize_segment(&s));
    let board = match query
        .get("board")
        .map(|b| b.trim())
        .filter(|b| !b.is_empty())
    {
        Some(b) => match sanitize_segment(b) {
            Some(v) => v,
            None => return Err("board 非法（只允许字母数字与 . - _，≤64 字符）".into()),
        },
        None => String::new(),
    };
    match (project, model, version) {
        (Some(project), Some(model), Some(version)) => Ok((project, model, board, version)),
        _ => Err(format!(
            "无法确定固件的 功能/型号/版本（包内元数据{}）；请显式给出 ?project=&model=&version=…",
            if parsed.is_some() {
                "字段为空"
            } else {
                "解析失败（非 ESP-IDF app 镜像）"
            }
        )),
    }
}

/// 列出全部托管条目（管理页数据源）：`GET /api/ota/firmware`。
async fn list() -> Response {
    let root = firmware_dir();
    let entries: Vec<serde_json::Value> = scan().iter().map(|m| m.to_json()).collect();
    (
        json_headers(),
        serde_json::json!({
            "dir": root.display().to_string(),
            "count": entries.len(),
            "entries": entries,
        })
        .to_string(),
    )
        .into_response()
}

/// 删除一个条目（管理动作，需鉴权）：`POST /api/ota/firmware/delete`（客户端走 POST）
/// 与 `DELETE /api/ota/firmware`（curl 用户）同一处理器。
async fn remove_entry(
    State(engines): State<Arc<Engines>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if !auth_ok(&headers, &query, &engines.config.server) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let project = query.get("project").map(|s| s.trim()).unwrap_or("");
    let model = query.get("model").map(|s| s.trim()).unwrap_or("");
    let board = query.get("board").map(|s| s.trim()).unwrap_or("");
    if project.is_empty() || model.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "缺少 project/model 查询参数（标识要删除的固件）",
        )
            .into_response();
    }
    if !remove(project, model, board) {
        return (
            StatusCode::NOT_FOUND,
            format!("未找到 project={project} model={model} board={board} 的托管固件"),
        )
            .into_response();
    }
    tracing::info!("固件已删除：project={project} model={model} board={board}");
    (json_headers(), serde_json::json!({"ok": true}).to_string()).into_response()
}

/// 下载托管固件（**设备动作，不鉴权**——设备侧无法带 Authorization 头）：
/// `GET /api/ota/firmware/latest?project=&model=&board=` → 裸 app bin。
/// 无 project/model 时只在"恰好一个条目"下给出（兼容旧 URL / 人工核对），避免猜错型号。
async fn download_latest(Query(query): Query<HashMap<String, String>>) -> Response {
    let root = firmware_dir();
    let project = query.get("project").map(String::as_str).unwrap_or("");
    let model = query.get("model").map(String::as_str).unwrap_or("");
    let board = query.get("board").map(String::as_str).unwrap_or("");
    let meta = if project.is_empty() || model.is_empty() {
        let all = scan();
        match all.len() {
            1 => all.into_iter().next(),
            _ => None,
        }
    } else {
        find(project, model, board)
    };
    let Some(meta) = meta else {
        return (
            StatusCode::NOT_FOUND,
            format!(
                "未找到匹配的托管固件（project={project} model={model} board={board}）；\
                 请在 /api/ota/firmware 查看已托管条目，或上传后再试"
            ),
        )
            .into_response();
    };
    let path = bin_path_in(&root, &meta);
    match std::fs::read(&path) {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/octet-stream"),
                (header::CONTENT_LENGTH, &bytes.len().to_string()),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            format!(
                "元数据存在（project={} model={} version={}）但 {} 缺失——请重新上传",
                meta.project,
                meta.model,
                meta.version,
                path.display()
            ),
        )
            .into_response(),
    }
}

/// 固件托管子路由（并入主 router；上传体积上限仅作用于本组）。
pub fn router() -> Router<Arc<Engines>> {
    Router::new()
        .route(
            "/api/ota/firmware",
            get(list)
                .post(upload)
                .delete(remove_entry)
                .layer(DefaultBodyLimit::max(MAX_FIRMWARE_BYTES)),
        )
        .route("/api/ota/firmware/latest", get(download_latest))
        // 删除的管理入口：Kuikly 客户端 NetworkModule 只暴露 GET/POST（无 DELETE），
        // 故另给一个 POST 入口（标识走 query）；curl 用户两种方法都可用。
        .route("/api/ota/firmware/delete", post(remove_entry))
}

#[cfg(test)]
mod tests;
