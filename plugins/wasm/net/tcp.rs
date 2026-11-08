//! 原始 TCP 监听：宿主只做连接管理与字节搬运，协议解析完全在插件侧。
//!
//! 事件：`io.tcp.opened` / `io.tcp.data`（原始字节）/ `io.tcp.closed`。

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use anyhow::Result;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use super::NetManager;
use crate::plugins::wasm::contract::EndpointSpecDto;
use crate::plugins::wasm::event_bus::Envelope;

/// 单连接读缓冲。
const READ_CHUNK: usize = 16 * 1024;
/// 空闲超时（无数据即关闭）。
const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

pub(super) fn spawn(
    net: &Arc<NetManager>,
    _endpoint_id: u64,
    plugin_id: &str,
    spec: &EndpointSpecDto,
) -> Result<(u16, tokio::sync::oneshot::Sender<()>)> {
    let addr = format!("{}:{}", spec.bind, spec.port);
    let listener = std::net::TcpListener::bind(&addr)
        .map_err(|e| anyhow::anyhow!("TCP 端点绑定失败 {addr}：{e}"))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

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
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let Ok((stream, peer)) = accepted else { continue };
                    let conn_id = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let net2 = Arc::clone(&net);
                    let plugin2 = plugin.clone();
                    let peer_s = peer.to_string();
                    tokio::spawn(async move {
                        let _ = serve_conn(net2, &plugin2, conn_id, stream, peer_s).await;
                    });
                }
            }
        }
    });

    Ok((port, shutdown_tx))
}

async fn serve_conn(
    net: Arc<NetManager>,
    plugin_id: &str,
    conn_id: u64,
    stream: tokio::net::TcpStream,
    peer: String,
) -> Result<()> {
    let (mut read_half, mut write_half) = stream.into_split();
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(64);
    net.register_conn(conn_id, plugin_id, &peer, tx);

    net.deliver(
        plugin_id,
        Envelope::json(
            "io.tcp.opened",
            &json!({ "connection_id": conn_id, "peer": peer }),
        ),
    );

    let mut buf = vec![0u8; READ_CHUNK];
    let reason = loop {
        tokio::select! {
            outbound = rx.recv() => {
                match outbound {
                    Some(bytes) => {
                        if write_half.write_all(&bytes).await.is_err() {
                            break "host";
                        }
                    }
                    None => break "host",
                }
            }
            read = tokio::time::timeout(IDLE_TIMEOUT, read_half.read(&mut buf)) => {
                match read {
                    Ok(Ok(0)) => break "peer",
                    Ok(Ok(n)) => {
                        net.deliver(
                            plugin_id,
                            Envelope {
                                topic: "io.tcp.data".to_string(),
                                correlation_id: Some(conn_id.to_string()),
                                payload: buf[..n].to_vec(),
                            },
                        );
                    }
                    Ok(Err(_)) => break "peer",
                    Err(_) => break "idle",
                }
            }
        }
    };

    net.unregister_conn(conn_id);
    let _ = write_half.shutdown().await;
    net.deliver(
        plugin_id,
        Envelope::json(
            "io.tcp.closed",
            &json!({ "connection_id": conn_id, "peer": peer, "reason": reason }),
        ),
    );
    Ok(())
}
