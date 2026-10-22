//! 设备管理：EchoMuse 设备控制面、固件 OTA、局域网管理客户端发现、
//! 进程内设备/会话注册表、发音人目录服务。

pub mod discovery;
pub mod echomuse;
pub mod firmware;
pub mod registry;
pub mod voices;

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
