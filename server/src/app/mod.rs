//! 应用层：与具体引擎无关的运行时组件——会话编排、网关、协议、传输、下行、
//! 平台功能（OTA 托管 / 发音人目录）、令牌用量计量、设备与会话注册表。
//! 引擎实现见 [`crate::plugins`]。

pub mod audio;
pub mod bench_api;
pub mod downlink;
pub mod echomuse;
pub mod events;
pub mod firmware;
pub mod hot_audio;
pub mod inference;
pub mod inject;
pub mod logtime;
pub mod mcp;
pub mod metrics;
pub mod prefetch;
pub mod protocol;
pub mod registry;
pub mod session;
pub mod splitter;
pub mod sse;
pub mod transport;
pub mod tts_cache;
pub mod usage;
pub mod voices;
pub mod ws;

/// 探测本机默认出口的局域网 IPv4（UDP connect 不发包，只查内核路由选源）。
/// 无配置项：部署在哪台机器、是什么 IP 就用什么——macApp 连接地址展示与
/// EchoMuse mDNS 广播共用这一个出口。
pub fn local_ipv4() -> Option<std::net::Ipv4Addr> {
    let sock = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    sock.connect(("8.8.8.8", 80)).ok()?;
    match sock.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(v4) => Some(v4),
        std::net::IpAddr::V6(_) => None,
    }
}
