//! Explicit local admin-only mode. Never selected as a fallback for failed production boot.
use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::{bail, Result};
use axum::{
    extract::{Query, Request, State},
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::runtime::Engines;

pub const DISABLED: &str = "开发模式仅运行管理台与真实管理 API，未启用模型推理、语音会话或训练；完整语音服务请退出 --dev 并配置模型后启动";

#[derive(Debug, Default, PartialEq)]
pub struct Options {
    pub enabled: bool,
    pub port: Option<u16>,
    pub config_db: Option<String>,
}

impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut args = args.into_iter();
        let mut out = Self::default();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--dev" => out.enabled = true,
                "--config-db" => {
                    let value = args
                        .next()
                        .filter(|v| !v.is_empty() && !v.starts_with("--"));
                    out.config_db =
                        Some(value.ok_or_else(|| anyhow::anyhow!("--config-db 需要文件路径"))?);
                }
                "--port" => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("--port 需要端口"))?;
                    let port = value.parse::<u16>()?;
                    if port == 0 {
                        bail!("--port 必须为 1～65535");
                    }
                    out.port = Some(port);
                }
                _ => bail!("未知参数 {arg}；支持 --dev、--port <端口>、--config-db <路径>"),
            }
        }
        if out.port.is_some() && !out.enabled {
            bail!("--port 仅用于 --dev；生产端口请修改配置库");
        }
        if out.enabled && out.config_db.as_deref() == Some(":memory:") {
            bail!("--dev 需要独立的文件配置库，不能使用 :memory:");
        }
        Ok(out)
    }

    /// Development deliberately ignores a possibly production XIAOZHI_CONFIG_DB environment.
    pub fn dev_db(&self) -> String {
        self.config_db.clone().unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../.dev/data.db")
                .to_string_lossy()
                .into_owned()
        })
    }

    pub fn apply(&self, config: &mut crate::config::Config) {
        if !self.enabled {
            return;
        }
        config.server.port = self.port.unwrap_or(8000);
        // Device registry, firmware and file APIs derive their root from memory.db_path.
        // A separate ConfigStore alone does not isolate these files.
        config.memory.db_path = std::path::absolute(self.dev_db())
            .expect("无法解析开发数据目录")
            .to_string_lossy()
            .into_owned();
        // Isolated HTTP admin serving; no real-model path or cloud credential is fabricated.
        if std::env::var("XIAOZHI_ADMIN_DIR")
            .ok()
            .filter(|s| !s.is_empty())
            .is_none()
        {
            config.server.admin_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../client/apps/h5App/web")
                .to_string_lossy()
                .into_owned();
        }
    }
}

/// Diagnostics have their own provider factories, so they must be blocked before dispatch.
fn blocked(path: &str) -> bool {
    path == "/api/ws"
        || path == "/api/test"
        || path.starts_with("/api/test/")
        || path == "/api/nlu/model/build"
        || path.starts_with("/api/nlu/model/build/")
        || matches!(path, "/control" | "/data" | "/em/control" | "/em/data")
        || path.ends_with("/inject")
}

pub async fn guard(State(engines): State<Arc<Engines>>, req: Request, next: Next) -> Response {
    if !engines.dev_mode {
        return next.run(req).await;
    }
    let mut response = if blocked(req.uri().path().trim_end_matches('/')) {
        let query = Query::<HashMap<String, String>>::try_from_uri(req.uri())
            .map(|q| q.0)
            .unwrap_or_default();
        if !crate::gateway::ws::auth_ok(req.headers(), &query, &engines.config.server) {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error":"unauthorized"})),
            )
                .into_response()
        } else {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "error": DISABLED, "error_code":"dev_inference_disabled", "mode":"dev",
                    "inference_enabled": false
                })),
            )
                .into_response()
        }
    } else if req.uri().path() == "/api/health" {
        Json(json!({"status":"ok", "name":"xiaozhi-server-rust",
            "version":env!("CARGO_PKG_VERSION"), "mode":"dev", "inference_enabled":false
        }))
        .into_response()
    } else {
        next.run(req).await
    };
    response
        .headers_mut()
        .insert("x-xiaozhi-mode", HeaderValue::from_static("dev"));
    response
}

#[cfg(test)]
mod tests;
