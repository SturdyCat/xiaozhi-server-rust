//! Socket 服务层：把「插件声明端点 → 宿主监听 → 事件投递 → 插件回写」串起来。
//!
//! 边界（用户 2026-11-08 定）：
//! - **宿主只做连接管理、HTTP 报文分帧、WS 帧分帧与事件路由**；协议语义在插件侧。
//! - TCP / UDP 给插件**原始字节**；HTTP / WS 给结构化消息。
//! - 句柄一律是不透明 `u64`，插件拿不到 socket / fd / 凭证。
//!
//! 生命周期：端点随插件 `activate` 绑定、`deactivate` 回收；连接随端点回收一起关闭。

mod http;
mod tcp;
mod udp;
mod ws;

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};
use tokio::sync::mpsc;

use super::contract::{EndpointSpecDto, ProtocolDto};
use super::event_bus::Envelope;

/// 向插件投递网络事件的回调（registry 实现）。
pub trait NioSink: Send + Sync {
    /// 投递事件到指定插件（按该插件的订阅路由）；未订阅则丢弃。
    ///
    /// HTTP 应答不走这里：guest 直接调 `http-respond` 宿主函数
    /// （见 [`NetManager::respond_http`]），避免 sink 与 NetManager 互相持有。
    fn deliver(&self, plugin_id: &str, env: Envelope);
}

/// 端点运行时句柄。
pub struct EndpointHandle {
    pub id: u64,
    pub plugin_id: String,
    pub spec: EndpointSpecDto,
    /// 实际绑定的端口（`port = 0` 时由宿主分配）。
    pub bound_port: u16,
    /// 关闭信号：drop 后监听循环退出。
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

/// 连接出站通道（写半边）。
pub struct ConnHandle {
    pub plugin_id: String,
    pub peer: String,
    pub tx: mpsc::Sender<Vec<u8>>,
}

/// HTTP 等待中的请求（响应经 oneshot 回写给连接任务）。
pub type PendingHttp = Arc<Mutex<BTreeMap<String, tokio::sync::oneshot::Sender<HttpReply>>>>;

/// HTTP 响应体（插件 → 宿主）。
#[derive(Debug, Clone)]
pub struct HttpReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// UDP 端点句柄：出站与组播用同步 `std` 套接字（宿主函数是同步签名）。
/// 收包走 `try_clone` 出来的 tokio 套接字，二者共享同一 fd。
struct UdpHandle {
    plugin_id: String,
    socket: Arc<std::net::UdpSocket>,
}

/// 网络管理器。
pub struct NetManager {
    endpoints: Mutex<BTreeMap<u64, EndpointHandle>>,
    conns: Mutex<BTreeMap<u64, ConnHandle>>,
    udp_sockets: Mutex<BTreeMap<u64, UdpHandle>>,
    pending_http: PendingHttp,
    next_id: AtomicU64,
    sink: Arc<dyn NioSink>,
    /// 宿主未绑定运行时（纯同步环境/测试）时为 None：端点一律拒绝监听。
    runtime: Option<tokio::runtime::Handle>,
}

impl NetManager {
    pub fn new(runtime: Option<tokio::runtime::Handle>, sink: Arc<dyn NioSink>) -> Arc<Self> {
        Arc::new(Self {
            endpoints: Mutex::new(BTreeMap::new()),
            conns: Mutex::new(BTreeMap::new()),
            udp_sockets: Mutex::new(BTreeMap::new()),
            pending_http: Arc::new(Mutex::new(BTreeMap::new())),
            next_id: AtomicU64::new(1),
            sink,
            runtime,
        })
    }

    /// tokio 句柄（协议实现 `spawn` / `enter` 用）；未绑定时 None。
    pub(super) fn runtime(&self) -> Option<&tokio::runtime::Handle> {
        self.runtime.as_ref()
    }

    fn alloc_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// 绑定端点并启动监听循环。返回实际端口。
    pub fn listen(self: &Arc<Self>, plugin_id: &str, spec: &EndpointSpecDto) -> Result<(u64, u16)> {
        // 端口冲突检测：同端口同协议已存在则拒绝
        {
            let eps = self
                .endpoints
                .lock()
                .map_err(|_| anyhow::anyhow!("端点锁中毒"))?;
            for e in eps.values() {
                if e.spec.port != 0 && e.spec.port == spec.port && e.spec.protocol == spec.protocol
                {
                    bail!(
                        "端口冲突：{}:{} 已被插件 {} 的端点 {} 占用",
                        spec.bind,
                        spec.port,
                        e.plugin_id,
                        e.spec.name
                    );
                }
            }
        }

        if self.runtime.is_none() {
            bail!("宿主未绑定 tokio 运行时，无法监听端点 {}", spec.name);
        }
        let id = self.alloc_id();
        let this = Arc::clone(self);
        let (port, shutdown) = match spec.protocol {
            ProtocolDto::Http => http::spawn(&this, id, plugin_id, spec)?,
            ProtocolDto::Ws => ws::spawn(&this, id, plugin_id, spec)?,
            ProtocolDto::Tcp => tcp::spawn(&this, id, plugin_id, spec)?,
            ProtocolDto::Udp => udp::spawn(&this, id, plugin_id, spec)?,
        };

        let handle = EndpointHandle {
            id,
            plugin_id: plugin_id.to_string(),
            spec: spec.clone(),
            bound_port: port,
            _shutdown: shutdown,
        };
        self.endpoints
            .lock()
            .map_err(|_| anyhow::anyhow!("端点锁中毒"))?
            .insert(id, handle);
        Ok((id, port))
    }

    /// 撤销端点（连接随之关闭）。
    pub fn unlisten(&self, plugin_id: &str, endpoint_id: u64) -> Result<()> {
        let mut eps = self
            .endpoints
            .lock()
            .map_err(|_| anyhow::anyhow!("端点锁中毒"))?;
        match eps.get(&endpoint_id) {
            Some(e) if e.plugin_id == plugin_id => {
                eps.remove(&endpoint_id);
                drop(eps);
                if let Ok(mut socks) = self.udp_sockets.lock() {
                    socks.remove(&endpoint_id);
                }
                Ok(())
            }
            Some(_) => bail!("端点 {endpoint_id} 不属于插件 {plugin_id}"),
            None => bail!("端点 {endpoint_id} 不存在"),
        }
    }

    /// 撤销某插件的全部端点与连接（deactivate / remove）。
    pub fn clear_plugin(&self, plugin_id: &str) {
        if let Ok(mut eps) = self.endpoints.lock() {
            eps.retain(|_, e| e.plugin_id != plugin_id);
        }
        if let Ok(mut conns) = self.conns.lock() {
            conns.retain(|_, c| c.plugin_id != plugin_id);
        }
        if let Ok(mut socks) = self.udp_sockets.lock() {
            socks.retain(|_, s| s.plugin_id != plugin_id);
        }
    }

    /// 注册连接（协议实现调用）。
    fn register_conn(&self, id: u64, plugin_id: &str, peer: &str, tx: mpsc::Sender<Vec<u8>>) {
        if let Ok(mut conns) = self.conns.lock() {
            conns.insert(
                id,
                ConnHandle {
                    plugin_id: plugin_id.to_string(),
                    peer: peer.to_string(),
                    tx,
                },
            );
        }
    }

    fn unregister_conn(&self, id: u64) {
        if let Ok(mut conns) = self.conns.lock() {
            conns.remove(&id);
        }
    }

    fn conn_owner(&self, id: u64) -> Option<(String, String)> {
        self.conns
            .lock()
            .ok()?
            .get(&id)
            .map(|c| (c.plugin_id.clone(), c.peer.clone()))
    }

    /// 出站写（TCP / WS）。返回入队字节数。
    pub fn send(&self, plugin_id: &str, conn_id: u64, payload: &[u8]) -> Result<u64> {
        let conns = self
            .conns
            .lock()
            .map_err(|_| anyhow::anyhow!("连接表锁中毒"))?;
        let conn = conns
            .get(&conn_id)
            .ok_or_else(|| anyhow::anyhow!("连接 {conn_id} 不存在"))?;
        if conn.plugin_id != plugin_id {
            bail!("连接 {conn_id} 不属于插件 {plugin_id}");
        }
        conn.tx
            .try_send(payload.to_vec())
            .map_err(|_| anyhow::anyhow!("连接 {conn_id} 出站队列已满或已关闭"))?;
        Ok(payload.len() as u64)
    }

    /// 主动关闭连接。
    pub fn close(&self, plugin_id: &str, conn_id: u64, _reason: &str) -> Result<()> {
        match self.conn_owner(conn_id) {
            Some((owner, _)) if owner == plugin_id => {
                self.unregister_conn(conn_id);
                Ok(())
            }
            Some((owner, _)) => bail!("连接 {conn_id} 属于插件 {owner}，非 {plugin_id}"),
            None => bail!("连接 {conn_id} 不存在"),
        }
    }

    /// 对端地址。
    pub fn peer(&self, conn_id: u64) -> Option<String> {
        self.conn_owner(conn_id).map(|(_, peer)| peer)
    }

    /// 出站 TCP 连接（`tcp-connect`，S3l）。与入站连接同表同事件口径；权限门
    /// 在 instance.rs 宿主函数层（`net` 权限）。
    pub fn tcp_connect(self: &Arc<Self>, plugin_id: &str, host: &str, port: u16) -> Result<u64> {
        tcp::connect(self, plugin_id, host, port)
    }

    // ---------------- UDP ----------------

    /// 注册 UDP 句柄（`udp::spawn` 调用；出站与组播复用同一 fd）。
    fn register_udp(&self, endpoint_id: u64, plugin_id: &str, socket: Arc<std::net::UdpSocket>) {
        if let Ok(mut socks) = self.udp_sockets.lock() {
            socks.insert(
                endpoint_id,
                UdpHandle {
                    plugin_id: plugin_id.to_string(),
                    socket,
                },
            );
        }
    }

    /// 取插件名下 UDP 句柄（含归属校验）。
    fn udp_handle(&self, plugin_id: &str, socket_id: u64) -> Result<Arc<std::net::UdpSocket>> {
        let socks = self
            .udp_sockets
            .lock()
            .map_err(|_| anyhow::anyhow!("UDP 句柄表锁中毒"))?;
        let h = socks
            .get(&socket_id)
            .ok_or_else(|| anyhow::anyhow!("UDP 句柄 {socket_id} 不存在"))?;
        if h.plugin_id != plugin_id {
            bail!(
                "UDP 句柄 {socket_id} 属于插件 {}，非 {plugin_id}",
                h.plugin_id
            );
        }
        Ok(Arc::clone(&h.socket))
    }

    /// UDP 出站（`udp-send`）。`peer` 形如 `ip:port`，主机名走一次同步解析。
    pub fn udp_send(
        &self,
        plugin_id: &str,
        socket_id: u64,
        peer: &str,
        payload: &[u8],
    ) -> Result<u64> {
        let socket = self.udp_handle(plugin_id, socket_id)?;
        let addr = resolve_socket_addr(peer)?;
        let n = socket
            .send_to(payload, addr)
            .map_err(|e| anyhow::anyhow!("UDP 发送到 {peer} 失败：{e}"))?;
        Ok(n as u64)
    }

    /// 加入组播组（`udp-join-multicast`，SSDP 等）。
    pub fn udp_join_multicast(&self, plugin_id: &str, socket_id: u64, group: &str) -> Result<()> {
        let socket = self.udp_handle(plugin_id, socket_id)?;
        if let Ok(v4) = group.parse::<Ipv4Addr>() {
            socket
                .join_multicast_v4(&v4, &Ipv4Addr::UNSPECIFIED)
                .map_err(|e| anyhow::anyhow!("加入组播 {group} 失败：{e}"))?;
            return Ok(());
        }
        if let Ok(v6) = group.parse::<Ipv6Addr>() {
            socket
                .join_multicast_v6(&v6, 0)
                .map_err(|e| anyhow::anyhow!("加入组播 {group} 失败：{e}"))?;
            return Ok(());
        }
        bail!("组播地址非法：{group}")
    }

    /// UDP 句柄投影（诊断用）。
    pub fn udp_socket_count(&self) -> usize {
        self.udp_sockets.lock().map(|s| s.len()).unwrap_or(0)
    }

    /// 投递事件（协议实现用）。
    fn deliver(&self, plugin_id: &str, env: Envelope) {
        self.sink.deliver(plugin_id, env);
    }

    /// 注册等待中的 HTTP 请求。
    fn wait_http(&self, request_id: &str) -> Result<tokio::sync::oneshot::Receiver<HttpReply>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending_http
            .lock()
            .map_err(|_| anyhow::anyhow!("HTTP 等待表锁中毒"))?
            .insert(request_id.to_string(), tx);
        Ok(rx)
    }

    /// 投递插件给的 HTTP 响应。
    fn resolve_http(&self, plugin_id: &str, request_id: &str, reply: HttpReply) -> Result<()> {
        let mut map = self
            .pending_http
            .lock()
            .map_err(|_| anyhow::anyhow!("HTTP 等待表锁中毒"))?;
        match map.remove(request_id) {
            Some(tx) => {
                let _ = tx.send(reply);
                Ok(())
            }
            None => bail!("HTTP 请求 {request_id} 不存在或已超时（插件 {plugin_id}）"),
        }
    }

    /// `http-respond` 宿主入口：把插件给的响应回写给等待中的连接。
    pub fn respond_http(
        &self,
        plugin_id: &str,
        request_id: &str,
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<()> {
        self.resolve_http(
            plugin_id,
            request_id,
            HttpReply {
                status,
                headers,
                body,
            },
        )
        .or_else(|e| {
            // 请求已超时（连接已回 504）：插件迟到不算错误，只记一笔。
            tracing::debug!(plugin = plugin_id, request_id, "http-respond 迟到：{e}");
            Ok(())
        })
    }

    /// 端点投影（`/api/plugins` 用）。
    pub fn endpoints_of(&self, plugin_id: &str) -> Vec<serde_json::Value> {
        self.endpoints
            .lock()
            .map(|eps| {
                eps.values()
                    .filter(|e| e.plugin_id == plugin_id)
                    .map(|e| {
                        serde_json::json!({
                            "name": e.spec.name,
                            "protocol": e.spec.protocol.as_str(),
                            "bind": e.spec.bind,
                            "port": e.bound_port,
                            "path": e.spec.path,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn connection_count(&self) -> usize {
        self.conns.lock().map(|c| c.len()).unwrap_or(0)
    }
}

/// 生成请求 id（HTTP / WS 用）。
fn next_request_id(counter: &AtomicU64) -> String {
    format!("r{}", counter.fetch_add(1, Ordering::Relaxed))
}

/// `ip:port` 或 `host:port` → 具体地址（仅用于 UDP 出站，同步解析一次）。
fn resolve_socket_addr(peer: &str) -> Result<SocketAddr> {
    if let Ok(addr) = peer.parse::<SocketAddr>() {
        return Ok(addr);
    }
    peer.to_socket_addrs()
        .map_err(|e| anyhow::anyhow!("对端地址解析失败 {peer}：{e}"))?
        .next()
        .ok_or_else(|| anyhow::anyhow!("对端地址无解析结果：{peer}"))
}
