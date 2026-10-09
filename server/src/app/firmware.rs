//! 固件 OTA 托管：`idf.py build` 产物（ESP32 app .bin）经 HTTP 上传到服务端持久卷，
//! 设备经 `/api/ota` 响应的 `firmware.{version,url}` 感知新版本并流式拉取自升级。
//!
//! ## 存储
//! 目录由 `XIAOZHI_FIRMWARE_DIR` 指定（默认 `/data/firmware`——复用 compose 的 /data/models
//! 持久卷，本地开发可 env 覆盖）。**只保留最新版**：固定文件 `firmware.bin` + 元数据
//! `latest.json` `{version, size, sha256, uploaded_at}`，上传即覆盖（无历史版本堆积），
//! `latest.json` 是 `/api/ota` 检查的"最新版"来源。
//!
//! ## 上传（管理动作，expected_token 非空时需 Bearer 鉴权）
//! ```bash
//! curl -H "Authorization: Bearer <expected_token>" \
//!      --data-binary @build/xiaozhi-esp32.bin \
//!      "http://<host>:8000/api/ota/firmware?version=2.0.4"
//! ```
//! ⚠️ version 必须是设备 `IsNewVersionAvailable` 判定下的**更新版本**（数字点分段，
//! 如 2.0.3 → 2.0.4），否则设备检查后不会升级。重复上传同版本 = 覆盖。
//!
//! ## 下载（设备动作）
//! `GET /api/ota/firmware/latest` → 裸 app bin 流。设备侧 `Ota::Upgrade` 依赖
//! **200 + Content-Length 非零**（缺失即判定失败），Body::from 全量字节天然满足。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::engine::Engines;
use crate::app::ws::auth_ok;

/// 单个固件上传上限（ESP32 app 一般 1.5~4MB，留足余量）。
const MAX_FIRMWARE_BYTES: usize = 64 * 1024 * 1024;
/// 固件二进制固定文件名（只保留最新版：上传即覆盖此文件）。
const BIN_FILENAME: &str = "firmware.bin";

/// 托管固件元数据（latest.json）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirmwareMeta {
    pub version: String,
    pub size: u64,
    pub sha256: String,
    /// 上传时刻（Unix 秒）。
    pub uploaded_at: u64,
}

/// 固件存储目录：`XIAOZHI_FIRMWARE_DIR`（默认 /data/firmware，随 compose 卷持久化）。
fn firmware_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("XIAOZHI_FIRMWARE_DIR").unwrap_or_else(|_| "/data/firmware".to_string()),
    )
}

/// 版本串白名单：数字点分段（可带字母/连字符后缀），拦截路径分隔符与控制字符——
/// 版本直接拼进存储文件名与下载 URL，不可放行 `/`、`\`、`?` 等。
fn sanitize_version(raw: &str) -> Option<String> {
    let v = raw.trim();
    if v.is_empty() || v.len() > 64 {
        return None;
    }
    if !v
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        return None;
    }
    Some(v.to_string())
}

/// 当前托管的最新固件元数据（/api/ota 检查用）；无托管或元数据损坏 → None。
pub fn latest() -> Option<FirmwareMeta> {
    let path = firmware_dir().join("latest.json");
    let data = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&data) {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::warn!("latest.json 解析失败（视为无托管固件）: {e}");
            None
        }
    }
}

/// 上传固件：`POST /api/ota/firmware?version=x.y.z`，body 为裸 app bin。
/// **上传即覆盖**（只保留最新版）并刷新 latest.json（临时文件 + rename，防写一半断电留半包）。
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
    let Some(version) = query.get("version").and_then(|v| sanitize_version(v)) else {
        return (
            StatusCode::BAD_REQUEST,
            "缺少合法的 version 查询参数（数字点分段，如 ?version=2.0.4）",
        )
            .into_response();
    };
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

    let sha = {
        let mut h = Sha256::new();
        h.update(&body);
        hex(&h.finalize())
    };
    let size = body.len() as u64;

    let dir = firmware_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("创建固件目录 {} 失败: {e}", dir.display()),
        )
            .into_response();
    }
    let bin_path = dir.join(BIN_FILENAME);
    // 临时文件 + rename：断电不留半包；rename 失败（目录挂载下不会发生）回退原地写
    let tmp_path = dir.join(format!("{}.{}.tmp", BIN_FILENAME, std::process::id()));
    if let Err(e) = std::fs::write(&tmp_path, &body) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("写入固件失败: {e}"),
        )
            .into_response();
    }
    if std::fs::rename(&tmp_path, &bin_path).is_err() {
        let _ = std::fs::remove_file(&tmp_path);
        if let Err(e) = std::fs::write(&bin_path, &body) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("写入固件失败（rename 回退也失败）: {e}"),
            )
                .into_response();
        }
    }
    let meta = FirmwareMeta {
        version: version.clone(),
        size,
        sha256: sha.clone(),
        uploaded_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    };
    let meta_path = dir.join("latest.json");
    if let Err(e) = std::fs::write(&meta_path, serde_json::to_string_pretty(&meta).unwrap_or_default())
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("固件已写入但元数据更新失败: {e}"),
        )
            .into_response();
    }
    tracing::info!(
        "固件已托管（仅保留最新版）：version={version} size={size} sha256={sha}（{}）",
        bin_path.display()
    );
    (header_json(), serde_json::json!({
        "version": version,
        "size": size,
        "sha256": sha,
    }).to_string()).into_response()
}

/// 查询当前托管版本（管理/核对用）：GET /api/ota/firmware。
async fn info() -> Response {
    match latest() {
        Some(m) => (header_json(), serde_json::to_string(&m).unwrap_or_default()).into_response(),
        None => (StatusCode::NOT_FOUND, "尚未托管任何固件").into_response(),
    }
}

/// 下载最新固件：GET /api/ota/firmware/latest → 裸 app bin（Content-Length 完整，
/// 设备 Ota::Upgrade 依赖非零 Content-Length 判定有效性）。只保留最新版，无历史版本。
async fn download_latest() -> Response {
    let Some(meta) = latest() else {
        return (StatusCode::NOT_FOUND, "尚未托管任何固件").into_response();
    };
    let path = firmware_dir().join(BIN_FILENAME);
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
                "元数据存在（version={}）但 {} 缺失——请重新上传",
                meta.version,
                BIN_FILENAME
            ),
        )
            .into_response(),
    }
}

fn header_json() -> [(header::HeaderName, &'static str); 1] {
    [(header::CONTENT_TYPE, "application/json; charset=utf-8")]
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 固件托管子路由（并入主 router；上传限流仅作用于本组）。
pub fn router() -> Router<Arc<Engines>> {
    Router::new()
        .route(
            "/api/ota/firmware",
            get(info).post(upload).layer(DefaultBodyLimit::max(MAX_FIRMWARE_BYTES)),
        )
        .route("/api/ota/firmware/latest", get(download_latest))
}

#[cfg(test)]
mod tests;
