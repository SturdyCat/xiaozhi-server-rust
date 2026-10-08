//! WebSocket 网关：HTTP 路由、握手与协商、鉴权，然后转入会话状态机
//! （[`crate::app::session::run_session`]）。
//!
//! ## 路由
//! - `GET /api/health`：健康检查，返回 JSON（status/name/version）。
//! - `GET /api/ws`：WebSocket 会话入口（先 [`auth_ok`] 鉴权，再 [`handle_handshake`] 协商）。
//! - `GET/PUT/POST /api/config`：管理页面读写当前配置（语义见下）。
//! - `GET/POST /api/ota(/)`：小智固件 OTA 引导（返回 websocket 接入信息 + 校时 + 版本，
//!   契约见 [`ota`]；固件侧把 OTA_URL 指到本服务即自动接入，无需硬编码 WS 地址）。
//! - 其余路径：静态托管 `[server].admin_dir`（h5App 构建产物，含 `index.html`）；
//!   目录不存在时 [`admin_missing`] 返回友好提示而非崩溃；SPA 未知路径回退 `index.html`。
//!
//! ## 鉴权
//! `[server].expected_token` 为空则不校验；非空时设备走 `Authorization: Bearer <token>`，
//! 浏览器 WebSocket 无法自定义请求头，额外允许 `?token=<token>` 查询参数兜底
//! （管理页面通过 URL 带 token 联调；设备侧仍走 Authorization 头）。见 [`auth_ok`]。
//!
//! ## `/api/config` 读写语义
//! - `GET`：若启动指定了配置文件则实时读盘，否则返回内存中的内置默认配置（/data/models 生产路径）。
//! - `PUT`/`POST`：写回启动加载的配置文件（TOML，**原注释会被覆盖丢失**）。
//!   内置默认启动、`config_path` 为 `None` 时返回 400（无法持久化）。
//! - ⚠️ 引擎相关参数（ASR/TTS/LLM）在启动时构建，改配置后**需重启 server** 才生效；
//!   仅 `[server]` 部分（监听 / token / 管理页目录）下次启动生效。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::{
    extract::{ws::WebSocketUpgrade, ws::WebSocket, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use tower_http::services::{ServeDir, ServeFile};
use uuid::Uuid;

use crate::config::ServerConfig;
use crate::engine::Engines;
use crate::app::protocol::{AudioParams, BinVersion, ClientHello, ClientMessage, ServerMessage};
use crate::app::session::{run_session, send_text, SessionParams};
use crate::app::transport::{IncomingFrame, Transport, WsTransport};

mod config;
mod ota;
use config::{config_meta, get_config, put_config};
// 插件维护
pub(crate) use config::config_is_persistent;
use ota::ota;

/// 构造 Axum 路由：API（健康检查 / WebSocket）之外，其余路径静态托管
/// `config.server.admin_dir` 指向的管理页面（h5App 构建产物，含 index.html）。
/// 该目录不存在时，`/` 及以下返回友好提示而非崩溃。
pub fn router(engines: Arc<Engines>) -> Router {
    let admin_dir = engines.config.server.admin_dir.clone();

    let api = Router::new()
        .route("/api/health", get(health))
        .route("/api/ws", get(ws_handler))
        .route("/api/config", get(get_config).put(put_config).post(put_config))
        // 配置来源元信息（macApp 显示"配置文件路径 + 是否持久化"，排查配置丢失去向）
        .route("/api/config/meta", get(config_meta))
        // 小智固件的 OTA 引导端点（带/不带尾斜杠都注册：固件配置的 URL 以 / 结尾）。
        // ⚠️ 所有接口必须在 /api 之下（接口规范），固件 OTA_URL 指到 /api/ota/ 即可——
        // 该地址完全可配置，无需对齐官方服务器的 /xiaozhi/ota/ 路径。
        .route("/api/ota", get(ota).post(ota))
        .route("/api/ota/", get(ota).post(ota))
        // 固件托管（上传/下载/当前版本查询）+ 发音人目录，并入同一 /api 前缀
        .merge(crate::app::firmware::router())
        .merge(crate::app::voices::router());

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

/// 健康检查：返回 JSON（HEALTHCHECK 仅校验 HTTP 200）。
async fn health() -> Response {
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        serde_json::json!({
            "status": "ok",
            "name": "xiaozhi-server-rust",
            "version": env!("CARGO_PKG_VERSION"),
        })
        .to_string(),
    )
        .into_response()
}

/// 管理页面目录未配置 / 未构建时，给一个明确的提示而不是 404 空白。
async fn admin_missing() -> Response {
    (
        StatusCode::NOT_FOUND,
        "管理页面未找到：请先构建 h5App（web）并将 [server].admin_dir 指向其包含 index.html 的输出目录。",
    )
        .into_response()
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

pub(crate) fn auth_ok(headers: &HeaderMap, query: &HashMap<String, String>, server: &ServerConfig) -> bool {
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
    socket: WebSocket,
    engines: Arc<Engines>,
    headers: &HeaderMap,
) -> anyhow::Result<()> {
    // 设备身份（固件 WS 握手头；测试台 macApp 也带）：接入日志据此核对是哪台设备
    let device_id = headers
        .get("Device-Id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    let client_id = headers
        .get("Client-Id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    // 握手起即收口到传输抽象：hello 收发与会话主循环走同一条 Transport 路径
    let mut transport = WsTransport::new(socket);
    let hello = recv_hello(&mut transport).await?;

    // 协商：上行跟随设备 hello（默认 16k），下行由服务器配置决定（默认 24k）。
    //
    // ⚠️ **二进制协议版本跟随设备 `hello.version`**（上行与下行同版本）：设备按其 NVS
    // `websocket.version` 同时决定收/发帧格式（`websocket_protocol.cc` 的 `version_`），
    // 下行若用服务端配置的另一个版本，设备会把带头的帧当纯 Opus 解析（或反之）→ 解码乱码。
    // 服务端 `[audio].binary_protocol_version` 的作用是**经 OTA 写进设备 NVS**
    // （见 `ota_payload` 的 `websocket.version`），二者因此天然一致；此处以设备上报为准，
    // 兼容"设备 NVS 被手工改成其他版本"的场景。
    let uplink_bin_ver = BinVersion::from_u8(hello.version);
    let downlink_bin_ver = uplink_bin_ver;
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
    send_text(&mut transport, &server_hello)
        .await
        .map_err(|e| anyhow::anyhow!("发送 server hello 失败: {e}"))?;

    let params = SessionParams {
        uplink_bin_ver,
        downlink_bin_ver,
        uplink_sr,
        downlink_sr,
        downlink_frame_ms,
        downlink_lead_ms: engines.config.audio.downlink_lead_ms,
    };
    if hello.test {
        tracing::info!("session {session_id} 测试台连接（hello.test=true，受理 asr_test/tts_test/llm_test）");
    } else {
        // 设备正式会话（ASR/TTS 走这条 WS；按需建立——唤醒/按键触发对话时才连接，开机只走 OTA）
        tracing::info!(
            "session {session_id} 设备接入：device={device_id} client={client_id} \
             上行={uplink_sr}Hz 下行={downlink_sr}Hz/{downlink_frame_ms}ms 上行二进制协议=v{} 下行协议=v{}",
            hello.version,
            engines.config.audio.binary_protocol_version,
        );
    }
    run_session(&mut transport, engines, params, session_id, hello.test).await
}

async fn recv_hello<T: Transport>(transport: &mut T) -> anyhow::Result<ClientHello> {
    let frame = transport.recv().await;
    let s = match frame {
        IncomingFrame::Text(s) => s,
        IncomingFrame::Binary(_) => anyhow::bail!("首个消息不是文本 hello"),
        IncomingFrame::Closed => anyhow::bail!("连接在无 hello 时关闭或出错"),
    };
    let cm: ClientMessage =
        serde_json::from_str(&s).map_err(|e| anyhow::anyhow!("解析 hello 失败: {e}"))?;
    match cm {
        ClientMessage::Hello(h) => Ok(h),
        _ => anyhow::bail!("首个消息不是 hello"),
    }
}
