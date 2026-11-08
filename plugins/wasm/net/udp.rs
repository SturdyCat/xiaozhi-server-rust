//! UDP 监听：宿主只搬字节，报文语义在插件侧。
//!
//! 事件：`io.udp.datagram`（`{socket_id, peer, payload(base64)}`）。
//! 出站走 `udp-send`；SSDP 等组播经 `udp-join-multicast`。

use std::sync::Arc;

use anyhow::Result;
use base64::Engine as _;
use serde_json::json;

use super::NetManager;
use crate::plugins::wasm::contract::EndpointSpecDto;
use crate::plugins::wasm::event_bus::Envelope;

const MAX_DATAGRAM: usize = 64 * 1024;

pub(super) fn spawn(
    net: &Arc<NetManager>,
    endpoint_id: u64,
    plugin_id: &str,
    spec: &EndpointSpecDto,
) -> Result<(u16, tokio::sync::oneshot::Sender<()>)> {
    let addr = format!("{}:{}", spec.bind, spec.port);
    let std_socket = std::net::UdpSocket::bind(&addr)
        .map_err(|e| anyhow::anyhow!("UDP 端点绑定失败 {addr}：{e}"))?;
    std_socket.set_nonblocking(true)?;
    let port = std_socket.local_addr()?.port();

    let Some(rt) = net.runtime().cloned() else {
        anyhow::bail!("宿主未绑定 tokio 运行时，无法监听 UDP 端点");
    };
    // 同一 fd 两个句柄：std 供同步 `udp-send` / `udp-join-multicast`，tokio 供收包。
    let recv_socket = {
        let _guard = rt.enter();
        Arc::new(tokio::net::UdpSocket::from_std(std_socket.try_clone()?)?)
    };
    net.register_udp(endpoint_id, plugin_id, Arc::new(std_socket));

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
    let net2 = Arc::clone(net);
    let plugin = plugin_id.to_string();
    let socket_id = endpoint_id;
    rt.spawn(async move {
        let mut buf = vec![0u8; MAX_DATAGRAM];
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                received = recv_socket.recv_from(&mut buf) => {
                    let Ok((n, peer)) = received else { continue };
                    let payload = json!({
                        "socket_id": socket_id,
                        "peer": peer.to_string(),
                        "payload": base64::engine::general_purpose::STANDARD.encode(&buf[..n]),
                    });
                    net2.deliver(
                        &plugin,
                        Envelope {
                            topic: "io.udp.datagram".to_string(),
                            correlation_id: Some(socket_id.to_string()),
                            payload: payload.to_string().into_bytes(),
                        },
                    );
                }
            }
        }
    });

    Ok((port, shutdown_tx))
}
