//! 通用设备运行时：能力描述、局域网管理发现与进程内设备注册表。
//!
//! 具体设备协议必须以独立插件 feature 装载；底座不提供 stub、兼容壳或业务配置。

pub mod capabilities;
pub mod discovery;
#[cfg(feature = "esp_home")]
pub mod esp_home;
pub mod registry;

#[cfg(feature = "echomuse-plugin")]
pub mod echomuse;
#[cfg(feature = "echomuse-plugin")]
pub mod firmware;
#[cfg(feature = "tuya-plugin")]
pub mod tuya;
#[cfg(feature = "echomuse-plugin")]
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
