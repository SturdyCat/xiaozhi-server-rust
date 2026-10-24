//! 带 TCP keepalive 的 HTTP/WS 监听（`axum::serve` 的替换品）。
//!
//! **为什么不用 `axum::serve`**：`axum::serve` 不暴露 per-connection socket 选项，而
//! TCP keepalive 必须逐连接设置。设备**断电/拔线**时不发 FIN/RST，TCP 进入半开
//! （服务端视角仍是 ESTABLISHED）——tokio 的 read 永远 pending，写又只进内核缓冲
//! "成功"，应用层毫无感知。EchoMuse 的在线真源是 hub（会话收尾 `hub_remove_session`
//! 才摘除条目），于是管理台把断电设备一直显示成在线。
//!
//! TCP keepalive 探测由**内核**发送、对端不需要任何应用层配合：设备断电后探测无应答，
//! `idle + interval × retries`（默认 15 + 5×4 ≈ 35s）内内核判定死亡，read 返回错误 →
//! WS 会话结束 → hub 摘除 → 管理台转离线。正常设备只是空闲时，探测包每轮都被 ACK，
//! 连接完全不受影响（流量是每 15s 一个空探针，可忽略）。
//!
//! 实现：手写 accept loop——accept 后对 TcpStream 设 keepalive，再用 hyper http1 的
//! `serve_connection_with_upgrades` 跑同一个 Router。`into_make_service_with_connect_info`
//! 的 `Service<SocketAddr>` 分支（axum 内置 `Connected<SocketAddr>`）以 accept 到的真实
//! 对端地址构造 ConnectInfo，`/shell/{id}` 等依赖 TCP 对端校验的 handler 行为与
//! `axum::serve` 完全一致。

use std::net::SocketAddr;
use std::time::Duration;

use hyper::server::conn::http1;
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use socket2::{SockRef, TcpKeepalive};
use tokio::net::TcpListener;
use tower::Service;

/// keepalive 空闲窗口：连接静默多久后开始发探测。
const KEEPALIVE_IDLE: Duration = Duration::from_secs(15);
/// 探测间隔：每轮探测包之间隔多久。
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
/// 探测次数：连续无应答多少轮后内核判定半开连接死亡（15 + 5×4 ≈ 35s 总窗口）。
const KEEPALIVE_RETRIES: u32 = 4;

/// 监听 [`TcpListener`] 并为每个连接挂 TCP keepalive，跑给定的 [`axum::Router`]。
///
/// 与 `axum::serve(listener, router.into_make_service_with_connect_info())` 的对外行为
/// 一致（HTTP/1.1 + WebSocket 升级 + ConnectInfo），只多出 keepalive 一项。
/// accept 级错误只告警并继续（监听器本身不退出）；单连接错误静默关闭。
pub(crate) async fn serve_with_keepalive(
    listener: TcpListener,
    router: axum::Router,
) -> std::io::Result<()> {
    let mut make_service = router.into_make_service_with_connect_info::<SocketAddr>();
    loop {
        let (stream, remote_addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                // EMFILE/ENOBUFS 一类临时错误：告警 + 休眠一拍再收，避免忙转刷日志
                tracing::warn!("[serve] accept 失败，继续监听: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        // keepalive 探测失败仅在极老内核/受限容器上发生：连接照常服务，只是
        // 退化回"半开连接由应用层兜底"的老行为。
        let keepalive = TcpKeepalive::new()
            .with_time(KEEPALIVE_IDLE)
            .with_interval(KEEPALIVE_INTERVAL)
            .with_retries(KEEPALIVE_RETRIES);
        if let Err(e) = SockRef::from(&stream).set_tcp_keepalive(&keepalive) {
            tracing::debug!("[serve] keepalive 设置失败（连接继续，无死亡探测）: {e}");
        }
        // Connected<SocketAddr>：以 accept 的真实对端地址构造 ConnectInfo。
        let svc = match make_service.call(remote_addr).await {
            Ok(svc) => svc,
            Err(e) => match e {}, // Infallible
        };
        // axum 的 service 是 tower::Service，hyper 1 要自己的 Service —— 官方适配器转换。
        let conn = http1::Builder::new()
            .serve_connection(TokioIo::new(stream), TowerToHyperService::new(svc))
            .with_upgrades();
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                tracing::debug!("[serve] 连接关闭: {e}");
            }
        });
    }
}
