//! WebSocket 监听：宿主只做握手与帧分帧，帧语义在插件侧。
//!
//! 事件：`io.ws.opened` / `io.ws.frame` / `io.ws.closed`。
//! 出站走 `net-send`（字节按二进制帧发出）。
//!
//! 边界：本版只支持 JSON 无关的裸帧转发，不做子协议协商、不做 permessage-deflate。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Result;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::ConnectInfo;
use axum::routing::any;
use axum::Router;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::mpsc;

use super::NetManager;
use crate::plugins::wasm::contract::EndpointSpecDto;
use crate::plugins::wasm::event_bus::Envelope;

/// 出站队列深度（与 TCP 一致）。
const OUTBOUND_QUEUE: usize = 64;

pub(super) fn spawn(
    net: &Arc<NetManager>,
    _endpoint_id: u64,
    plugin_id: &str,
    spec: &EndpointSpecDto,
) -> Result<(u16, tokio::sync::oneshot::Sender<()>)> {
    let addr = format!("{}:{}", spec.bind, spec.port);
    let listener = std::net::TcpListener::bind(&addr)
        .map_err(|e| anyhow::anyhow!("WS 端点绑定失败 {addr}：{e}"))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let path = spec.path.clone().unwrap_or_else(|| "/".to_string());

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let net = Arc::clone(net);
    let plugin = plugin_id.to_string();
    let Some(handle) = net.runtime().cloned() else {
        anyhow::bail!("宿主未绑定 tokio 运行时，无法启动监听循环");
    };

    handle.spawn(async move {
        let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
            return;
        };
        let counter = Arc::new(AtomicU64::new(1));

        // 处理器按插件与端点克隆一份上下文；路由只挂一个精确路径。
        let handler = {
            let net = Arc::clone(&net);
            let plugin = plugin.clone();
            let counter = Arc::clone(&counter);
            move |ConnectInfo(peer): ConnectInfo<SocketAddr>, ws: WebSocketUpgrade| {
                let net = Arc::clone(&net);
                let plugin = plugin.clone();
                let counter = Arc::clone(&counter);
                async move {
                    let conn_id = counter.fetch_add(1, Ordering::Relaxed);
                    ws.on_upgrade(move |socket| serve_conn(net, plugin, conn_id, peer, socket))
                }
            }
        };

        let app = Router::new().route(&path, any(handler));
        let served = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        })
        .await;
        if let Err(e) = served {
            tracing::warn!(target: "plugins.wasm.net", error = %e, path = %path, "WS 监听退出");
        }
    });

    Ok((port, shutdown_tx))
}

/// 单连接：入站帧 → `io.ws.frame`；出站队列 → 二进制帧。
async fn serve_conn(
    net: Arc<NetManager>,
    plugin_id: String,
    conn_id: u64,
    peer: SocketAddr,
    socket: WebSocket,
) {
    let peer_s = peer.to_string();
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(OUTBOUND_QUEUE);
    net.register_conn(conn_id, &plugin_id, &peer_s, tx);

    net.deliver(
        &plugin_id,
        Envelope::json(
            "io.ws.opened",
            &json!({ "connection_id": conn_id, "peer": peer_s }),
        ),
    );

    let reason = loop {
        tokio::select! {
            outbound = rx.recv() => {
                match outbound {
                    Some(bytes) => {
                        if sink.send(Message::Binary(bytes.into())).await.is_err() {
                            break "host";
                        }
                    }
                    None => break "host",
                }
            }
            inbound = stream.next() => {
                match inbound {
                    Some(Ok(Message::Binary(bytes))) => {
                        emit_frame(&net, &plugin_id, conn_id, true, &bytes);
                    }
                    Some(Ok(Message::Text(text))) => {
                        emit_frame(&net, &plugin_id, conn_id, false, text.as_str().as_bytes());
                    }
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) => break "peer",
                    Some(Err(_)) => break "peer",
                    None => break "peer",
                }
            }
        }
    };

    net.unregister_conn(conn_id);
    let _ = sink.close().await;
    net.deliver(
        &plugin_id,
        Envelope::json(
            "io.ws.closed",
            &json!({ "connection_id": conn_id, "peer": peer_s, "reason": reason }),
        ),
    );
}

fn emit_frame(net: &Arc<NetManager>, plugin_id: &str, conn_id: u64, binary: bool, payload: &[u8]) {
    let body = json!({
        "connection_id": conn_id,
        "binary": binary,
        "payload": base64::engine::general_purpose::STANDARD.encode(payload),
    });
    net.deliver(
        plugin_id,
        Envelope {
            topic: "io.ws.frame".to_string(),
            correlation_id: Some(conn_id.to_string()),
            payload: body.to_string().into_bytes(),
        },
    );
}
