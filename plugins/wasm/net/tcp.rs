//! 原始 TCP 监听：宿主只做连接管理与字节搬运，协议解析完全在插件侧。
//!
//! 事件：`io.tcp.opened` / `io.tcp.data`（原始字节）/ `io.tcp.closed`。

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
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let Ok((stream, peer)) = accepted else { continue };
                    // 连接 id 走全局分配器：与端点 id / 出站连接同一命名空间
                    // （曾用每端点局部计数器，跨端点首连接会在 conns 表相撞）。
                    let conn_id = net.alloc_id();
                    let peer_s = peer.to_string();
                    // 注册同步完成（S3m 勘误）：guest 收到 opened 后立刻 net_send
                    // 不应与 spawn 的 IO 任务赛跑。
                    let (tx, rx) = mpsc::channel(64);
                    net.register_conn(conn_id, &plugin, &peer_s, tx);
                    let net2 = Arc::clone(&net);
                    let plugin2 = plugin.clone();
                    tokio::spawn(async move {
                        let _ = serve_conn(net2, &plugin2, conn_id, stream, peer_s, rx).await;
                    });
                }
            }
        }
    });

    Ok((port, shutdown_tx))
}

/// 出站 TCP 连接（`tcp-connect`，S3l）：同步签名——宿主函数跑在 guest 泵的
/// `spawn_blocking` 线程里，回环/局域网 connect 阻塞可接受；解析 + 连接失败
/// 显式报错。成功后与入站连接同表同事件口径。
pub(super) fn connect(
    net: &Arc<NetManager>,
    plugin_id: &str,
    host: &str,
    port: u16,
) -> Result<u64> {
    let Some(handle) = net.runtime().cloned() else {
        anyhow::bail!("宿主未绑定 tokio 运行时，无法出站连接");
    };
    let addr = super::resolve_socket_addr(&format!("{host}:{port}"))?;
    let stream = std::net::TcpStream::connect(addr)
        .map_err(|e| anyhow::anyhow!("TCP 连接 {host}:{port} 失败：{e}"))?;
    stream.set_nonblocking(true)?;
    let peer = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| addr.to_string());
    let stream = tokio::net::TcpStream::from_std(stream)?;

    let conn_id = net.alloc_id();
    // 注册同步完成（S3m 勘误）：guest 拿到 connection-id 后立即 net-send，
    // 不与 spawn 的 IO 任务赛跑。
    let (tx, rx) = mpsc::channel(64);
    net.register_conn(conn_id, plugin_id, &peer, tx);
    let net = Arc::clone(net);
    let plugin = plugin_id.to_string();
    handle.spawn(async move {
        let _ = serve_conn(net, &plugin, conn_id, stream, peer, rx).await;
    });
    Ok(conn_id)
}

async fn serve_conn(
    net: Arc<NetManager>,
    plugin_id: &str,
    conn_id: u64,
    stream: tokio::net::TcpStream,
    peer: String,
    mut rx: mpsc::Receiver<Vec<u8>>,
) -> Result<()> {
    let (mut read_half, mut write_half) = stream.into_split();

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
    // S3p：closed 也带 correlation-id=connection-id（S3l 文档承诺的生命周期
    // 事件口径；此前只有 data 带，guest 无法把关闭映射回具体连接/设备）。
    net.deliver(
        plugin_id,
        Envelope {
            topic: "io.tcp.closed".to_string(),
            correlation_id: Some(conn_id.to_string()),
            payload: json!({ "connection_id": conn_id, "peer": peer, "reason": reason })
                .to_string()
                .into_bytes(),
        },
    );
    Ok(())
}
