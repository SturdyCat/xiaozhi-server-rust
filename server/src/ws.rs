//! WebSocket 网关：HTTP 路由、握手与协商、鉴权，然后转入会话状态机
//! （[`crate::session::run_session`]）。
//!
//! ## 路由
//! - `GET /api/health`：健康检查，返回 `xiaozhi-server-rust ok`。
//! - `GET /api/ws`：WebSocket 会话入口（先 [`auth_ok`] 鉴权，再 [`handle_handshake`] 协商）。
//! - `GET/PUT/POST /api/config`：管理页面读写当前配置（语义见下）。
//! - 其余路径：静态托管 `[server].admin_dir`（h5App 构建产物，含 `index.html`）；
//!   目录不存在时 [`admin_missing`] 返回友好提示而非崩溃；SPA 未知路径回退 `index.html`。
//!
//! ## 鉴权
//! `[server].expected_token` 为空则不校验；非空时设备走 `Authorization: Bearer <token>`，
//! 浏览器 WebSocket 无法自定义请求头，额外允许 `?token=<token>` 查询参数兜底
//! （管理页面通过 URL 带 token 联调；设备侧仍走 Authorization 头）。见 [`auth_ok`]。
//!
//! ## `/api/config` 读写语义
//! - `GET`：若启动指定了配置文件则实时读盘，否则返回内存中的内置默认配置（/models 生产路径）。
//! - `PUT`/`POST`：写回启动加载的配置文件（TOML，**原注释会被覆盖丢失**）。
//!   内置默认启动、`config_path` 为 `None` 时返回 400（无法持久化）。
//! - ⚠️ 引擎相关参数（ASR/TTS/LLM）在启动时构建，改配置后**需重启 server** 才生效；
//!   仅 `[server]` 部分（监听 / token / 管理页目录）下次启动生效。

use anyhow::Context;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::{
    extract::{ws::WebSocketUpgrade, ws::WebSocket, ws::Message, Json, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use futures_util::StreamExt;
use tower_http::services::{ServeDir, ServeFile};
use uuid::Uuid;

use crate::config::{Config, ServerConfig};
use crate::engine::Engines;
use crate::protocol::{AudioParams, BinVersion, ClientHello, ClientMessage, ServerMessage};
use crate::session::{run_session, send_text, SessionParams};

/// 构造 Axum 路由：API（健康检查 / WebSocket）之外，其余路径静态托管
/// `config.server.admin_dir` 指向的管理页面（h5App 构建产物，含 index.html）。
/// 该目录不存在时，`/` 及以下返回友好提示而非崩溃。
pub fn router(engines: Arc<Engines>) -> Router {
    let admin_dir = engines.config.server.admin_dir.clone();

    let api = Router::new()
        .route("/api/health", get(health))
        .route("/api/ws", get(ws_handler))
        .route("/api/config", get(get_config).put(put_config).post(put_config));

    let app = if Path::new(&admin_dir).is_dir() {
        // SPA：未知路径回退到 index.html，交给前端路由处理。
        let admin = ServeDir::new(&admin_dir)
            .precompressed_gzip()
            .not_found_service(ServeFile::new(Path::new(&admin_dir).join("index.html")));
        api.fallback_service(admin)
    } else {
        api.fallback(admin_missing)
    };

    app.with_state(engines)
}

async fn health() -> &'static str {
    "xiaozhi-server-rust ok"
}

/// 管理页面目录未配置 / 未构建时，给一个明确的提示而不是 404 空白。
async fn admin_missing() -> Response {
    (
        StatusCode::NOT_FOUND,
        "管理页面未找到：请先构建 h5App（web）并将 [server].admin_dir 指向其包含 index.html 的输出目录。",
    )
        .into_response()
}

/// 读取当前配置（供管理页面 UI 填充表单）。
/// 启动时指定了配置文件则实时从磁盘读取，否则返回内存中的内置默认配置（/models 生产路径）。
async fn get_config(State(engines): State<Arc<Engines>>) -> Response {
    let cfg = match &engines.config_path {
        Some(p) => match Config::load(p) {
            Ok(c) => c,
            Err(e) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, format!("读取配置失败: {e}")).into_response()
            }
        },
        None => (*engines.config).clone(),
    };
    match serde_json::to_string(&cfg) {
        Ok(json) => (
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            json,
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("序列化配置失败: {e}")).into_response(),
    }
}

/// 配置持久化（PUT /api/config 与 main.rs 的监听地址自愈共用）：
/// 序列化为 TOML 并原子写回（先写同目录临时文件再 rename，避免写一半被杀导致配置损坏）。
pub(crate) fn persist_config(path: &str, cfg: &Config) -> anyhow::Result<()> {
    let toml_str = toml::to_string_pretty(cfg).context("序列化配置失败")?;
    let tmp = format!("{}.{}.tmp", path, uuid::Uuid::new_v4());
    if let Err(e) = std::fs::write(&tmp, &toml_str) {
        let hint = if e.kind() == std::io::ErrorKind::PermissionDenied {
            "（配置文件或所在目录为只读，无法写入。请检查部署挂载是否误加了 :ro，或改用可写路径后重启 server。）"
        } else {
            ""
        };
        anyhow::bail!("写入配置失败: {e}{hint}");
    }
    std::fs::rename(&tmp, path).context("写入配置失败")?;
    Ok(())
}

/// 保存配置（供管理页面 UI 提交）。写回启动时加载的配置文件（TOML，原注释会丢失）。
/// 注意：server 进程内引擎（ASR/TTS/LLM）在启动时构建，改配置后需重启 server 才对引擎生效；
/// 仅 [server] 部分（监听地址 / token / 管理页目录）可在下次启动时生效。
async fn put_config(State(engines): State<Arc<Engines>>, Json(body): Json<Config>) -> Response {
    let path = match &engines.config_path {
        Some(p) => p.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                "server 以内置默认配置启动，未指定配置文件，无法持久化。请用 --config 指定 config.toml 后重启。",
            )
            .into_response()
        }
    };
    match persist_config(&path, &body) {
        Ok(()) => (
            StatusCode::OK,
            format!("配置已保存到 {path}；引擎相关参数需重启 server 生效。"),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if !auth_ok(&headers, &query, &engines.config.server) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    ws.on_upgrade(move |socket| async move {
        if let Err(e) = handle_handshake(socket, engines, &headers).await {
            tracing::warn!("会话异常: {e}");
        }
    })
}

fn auth_ok(headers: &HeaderMap, query: &HashMap<String, String>, server: &ServerConfig) -> bool {
    if server.expected_token.is_empty() {
        return true;
    }
    let expected = format!("Bearer {}", server.expected_token);
    if let Some(auth) = headers.get(header::AUTHORIZATION) {
        if auth.to_str().is_ok_and(|v| v == expected) {
            return true;
        }
    }
    // 兜底：浏览器 WebSocket 无法自定义请求头，允许 ?token= 查询参数
    // （管理页面通过 URL 带 token 联调；设备侧仍走 Authorization 头）
    query.get("token").is_some_and(|t| t == &server.expected_token)
}

async fn handle_handshake(
    mut socket: WebSocket,
    engines: Arc<Engines>,
    _headers: &HeaderMap,
) -> anyhow::Result<()> {
    let hello = recv_hello(&mut socket).await?;

    // 协商：上行跟随设备 hello（默认 16k），下行由服务器配置决定（默认 24k）。
    let uplink_bin_ver = BinVersion::from_u8(hello.version);
    let downlink_bin_ver = BinVersion::from_u8(engines.config.audio.binary_protocol_version);
    let uplink_sr = hello
        .audio_params
        .as_ref()
        .map(|a| a.sample_rate)
        .unwrap_or(16_000);
    let downlink_sr = engines.config.audio.downlink_sample_rate;
    let downlink_frame_ms = engines.config.audio.downlink_frame_duration_ms;
    let session_id = Uuid::new_v4().to_string();

    // 服务器 hello：写入下行音频参数（设备据此解码播放）。
    // session_id 与会话主循环共用同一个（对齐设备侧关联语义），故 clone 一份进 hello。
    let server_hello = ServerMessage::Hello {
        transport: "websocket",
        session_id: Some(session_id.clone()),
        audio_params: Some(AudioParams {
            sample_rate: downlink_sr,
            channels: engines.config.audio.channels,
            frame_duration: downlink_frame_ms,
            ..AudioParams::default()
        }),
    };
    send_text(&mut socket, &server_hello)
        .await
        .map_err(|e| anyhow::anyhow!("发送 server hello 失败: {e}"))?;

    let params = SessionParams {
        uplink_bin_ver,
        downlink_bin_ver,
        uplink_sr,
        downlink_sr,
        downlink_frame_ms,
    };
    if hello.test {
        tracing::info!("session {session_id} 测试台连接（hello.test=true，受理 asr_test/tts_test/llm_test）");
    }
    run_session(socket, engines, params, session_id, hello.test).await
}

async fn recv_hello(socket: &mut WebSocket) -> anyhow::Result<ClientHello> {
    let item = socket.next().await;
    let msg = match item {
        Some(Ok(m)) => m,
        _ => anyhow::bail!("连接在无 hello 时关闭或出错"),
    };
    match msg {
        Message::Text(t) => {
            let s = t.to_string();
            let cm: ClientMessage = serde_json::from_str(&s)
                .map_err(|e| anyhow::anyhow!("解析 hello 失败: {e}"))?;
            match cm {
                ClientMessage::Hello(h) => Ok(h),
                _ => anyhow::bail!("首个消息不是 hello"),
            }
        }
        _ => anyhow::bail!("首个消息不是文本 hello"),
    }
}
