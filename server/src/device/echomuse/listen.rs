//! EchoMuse 控制器专用端口监听（官方接口对齐）。
//!
//! 上游 emOS 固件（`emos/init/init.c` 的 `tcp_peer()`）只把「ESTABLISHED 且远端
//! 端口为 8767/8770」的连接识别为 controller——这是设备串口 banner `controller`
//! 字段的唯一判据（2026-10-20 对 v2.18.1 源码核对：全固件仅此一处按端口判连接，
//! 其余逻辑不依赖该状态）。本服务端因此在 [`crate::config::Config` 的
//! `[echomuse].plain_port`]（默认 8767，官方明文 controller 端口）**额外**监听
//! 一份设备面路由，mDNS `plain_port` 也广播它——设备经发现自动拨官方端口，
//! banner 如实显示 connected。主端口始终继续服务 `/em/*`（旧 endpoint 文件与
//! 联调脚本不受影响）；专用端口绑定失败只告警，mDNS 回退广播主端口。

use std::sync::Arc;

use axum::Router;

use super::routes::em_routes;
use crate::runtime::Engines;

/// 专用端口门控：`enabled` 且 `plain_port > 0` 才额外监听（0 = 关闭，仅主端口）。
pub(crate) fn dedicated_port(cfg: &super::EchomuseConfig) -> Option<u16> {
    (cfg.enabled && cfg.plain_port > 0).then_some(cfg.plain_port)
}

/// 绑定并 spawn 一个已完成 `with_state` 的路由监听；返回实际绑定端口，
/// 绑定失败（占用等）返回 `None` 由调用方回退——专用端口是增强，不是前提。
///
/// 走 [`crate::serve::serve_with_keepalive`]（TCP keepalive 手动 accept loop）：
/// 设备断电不发 FIN，半开连接若不靠内核探测，hub 条目永不摘除、管理台永远在线。
pub(crate) async fn bind_and_serve(router: Router, port: u16) -> Option<u16> {
    let addr = format!("0.0.0.0:{port}");
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("[echomuse] 控制器专用端口 {addr} 绑定失败（mDNS 广播回退主端口）: {e}");
            return None;
        }
    };
    let bound = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    // ConnectInfo 与主端口同款：设备 shell 拨回校验 TCP 对端，不信任转发头。
    tokio::spawn(async move {
        if let Err(e) = crate::serve::serve_with_keepalive(listener, router).await {
            tracing::warn!("[echomuse] 控制器专用端口服务退出: {e}");
        }
    });
    Some(bound)
}

/// 额外监听设备面专用端口（主端口之外的第二监听器，路由同 [`em_routes`]）。
/// 返回实际绑定端口；`None` = 绑定失败，调用方应回退广播主端口。
pub(crate) async fn spawn_dedicated(engines: Arc<Engines>, port: u16) -> Option<u16> {
    let router = em_routes().with_state(engines);
    let bound = bind_and_serve(router, port).await?;
    tracing::info!(
        "[echomuse] 控制器专用端口已监听 0.0.0.0:{bound}（官方 plain 端口，mDNS plain_port 指向此处；主端口 /em/* 继续保留）"
    );
    Some(bound)
}
