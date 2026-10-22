//! 管理客户端**局域网自动发现**：服务端开机常驻广播 mDNS `_xiaozhi-admin._tcp.local`。
//!
//! ## 为什么常驻广播
//!
//! macApp 等非 web 客户端没有「同域托管」的便利——用户必须手填 `http://<LAN-IP>:<port>`，
//! 服务器换机/换 IP 后全部客户端跟着失联（管理台「切换服务器」能改地址，但前提是
//! 用户知道新地址）。EchoMuse 设备侧已有同款机制（`_emcontroller._tcp`，emOS 靠它找
//! controller），本模块把同一招反向用给管理客户端：广播 `ip + port`，macApp 用
//! Bonjour 浏览（`NWBrowser`）监听后自动连接，无需任何手填。
//!
//! - **常驻、无配置项**：与 `[server].port` 同源，监听绑在哪个端口就广播哪个端口；
//!   广播失败只告警（发现是辅助，手填地址永远可用）。
//! - **服务类型 `_xiaozhi-admin._tcp`**：与 EchoMuse 的 `_emcontroller._tcp` 区分开，
//!   客户端只认自己关心的类型；实例名 `xiaozhi-server`（同机多实例由 mDNS 冲突协商）。
//! - 广播地址用 [`crate::device::local_ipv4`] 探测默认出口（与启动日志的
//!   「局域网连接地址」同一口径）；探测失败退回 loopback 并告警（容器部署常见）。
//!
//! 客户端契约（macApp 侧实现见 `client/apps/macosApp/XiaoZhiModule+Discovery.m`）：
//! 浏览 `_xiaozhi-admin._tcp` → 解析出 `A/AAAA + port` → `http://<ip>:<port>` 即 API 基址。

use std::net::Ipv4Addr;

/// mDNS 服务类型（含尾部点，mdns-sd 与 Bonjour 浏览两侧均按此口径）。
pub const SERVICE_TYPE: &str = "_xiaozhi-admin._tcp.local.";

/// mDNS 实例名与主机名：实例名给用户看（客户端发现列表展示），主机名做 A 记录归属。
pub const INSTANCE_NAME: &str = "xiaozhi-server";
const HOST_NAME: &str = "xiaozhi-admin.local.";

/// 开机常驻广播管理发现服务（main 在 HTTP 监听成功后调用）。
///
/// 网络注册的阻塞细节放 `spawn_blocking`；失败只告警不退出——发现只是辅助，
/// 客户端手填地址的路径永远可用。
pub fn spawn_admin_mdns(port: u16) {
    let ip = advertise_ipv4();
    tokio::task::spawn_blocking(move || {
        use mdns_sd::ServiceDaemon;

        let daemon = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("[discovery] mDNS daemon 启动失败（客户端仍可手填地址）: {e}");
                return;
            }
        };
        match service_info(ip, port) {
            Ok(info) => match daemon.register(info) {
                Ok(_rx) => {
                    tracing::info!("[discovery] mDNS 已广播 {}（{ip}:{port}）", SERVICE_TYPE);
                }
                Err(e) => tracing::warn!("[discovery] mDNS 注册失败: {e}"),
            },
            Err(e) => tracing::warn!("[discovery] mDNS ServiceInfo 构造失败: {e}"),
        }
    });
}

/// 组装 ServiceInfo（拆出纯函数供测试：ip/port → 合法服务描述）。
fn service_info(ip: std::net::Ipv4Addr, port: u16) -> Result<mdns_sd::ServiceInfo, mdns_sd::Error> {
    // mdns-sd 0.11 只接受纯 IP 字符串（端口由下一参单独给出），传 "ip:port" 会报错。
    let props = [("app".to_string(), "xiaozhi-admin".to_string())]
        .into_iter()
        .collect();
    mdns_sd::ServiceInfo::new(
        SERVICE_TYPE,
        INSTANCE_NAME,
        HOST_NAME,
        ip.to_string(),
        port,
        Some(props),
    )
}

/// 广播地址：探测默认出口局域网 IPv4（与启动日志口径一致）；失败退回 loopback 并告警。
fn advertise_ipv4() -> Ipv4Addr {
    match crate::device::local_ipv4() {
        Some(ip) => ip,
        None => {
            tracing::warn!(
                "[discovery] 无法探测局域网 IPv4（容器部署常见），mDNS 退回 127.0.0.1；\
                 跨机发现不可用时请改用 host 网络部署或手填地址"
            );
            Ipv4Addr::LOCALHOST
        }
    }
}

#[cfg(test)]
mod tests;
