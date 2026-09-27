//! WebSocket 网关：握手、协议版本/音频参数协商、鉴权，然后转入会话状态机。

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{ws::WebSocketUpgrade, ws::WebSocket, ws::Message, ws::Utf8Bytes, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use futures_util::StreamExt;
use uuid::Uuid;

use crate::config::ServerConfig;
use crate::engine::Engines;
use crate::protocol::{AudioParams, BinVersion, ClientHello, ClientMessage, ServerMessage};
use crate::session::{run_session, SessionParams};

/// 构造 Axum 路由（含健康检查与 WebSocket 端点）。
pub fn router(engines: Arc<Engines>) -> Router {
    Router::new()
        .route("/", get(test_page))
        .route("/api/health", get(health))
        .route("/api/ws", get(ws_handler))
        .with_state(engines)
}

async fn health() -> &'static str {
    "xiaozhi-server-rust ok"
}

/// 内嵌网页测试台（编译期打包，部署后直接访问 http://<host>/ 即可联调）。
async fn test_page() -> Html<&'static str> {
    Html(include_str!("web_test.html"))
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
    // （供 src/web_test.html 网页测试台使用；设备侧仍走 Authorization 头）
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
    let server_hello = ServerMessage::Hello {
        transport: "websocket",
        session_id: Some(session_id),
        audio_params: Some(AudioParams {
            format: "opus".to_string(),
            sample_rate: downlink_sr,
            channels: engines.config.audio.channels,
            frame_duration: downlink_frame_ms,
        }),
    };
    socket
        .send(Message::Text(Utf8Bytes::from(server_hello.to_json())))
        .await
        .map_err(|e| anyhow::anyhow!("发送 server hello 失败: {e}"))?;

    let params = SessionParams {
        uplink_bin_ver,
        downlink_bin_ver,
        uplink_sr,
        downlink_sr,
        downlink_frame_ms,
    };
    run_session(socket, engines, params).await
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
