//! [`listen`] 专用端口回归：端口门控与绑定/回退语义。
//!
//! 不起 [`crate::engine::Engines`]（与既有测试同口径，channel/平凡路由直驱）：
//! 用平凡路由验证真实 bind → serve（含 ConnectInfo make-service）→ loopback
//! 请求全链路，以及端口冲突时的 `None` 回退。

use axum::routing::get;
use axum::Router;

use super::listen::{bind_and_serve, dedicated_port};
use crate::app::echomuse::EchomuseConfig;

fn cfg(enabled: bool, plain_port: u16) -> EchomuseConfig {
    EchomuseConfig {
        enabled,
        plain_port,
        ..Default::default()
    }
}

#[test]
fn dedicated_port_gate() {
    assert_eq!(dedicated_port(&cfg(false, 8767)), None, "未启用不监听");
    assert_eq!(dedicated_port(&cfg(true, 0)), None, "0 = 关闭专用端口");
    assert_eq!(dedicated_port(&cfg(true, 8767)), Some(8767));
}

#[tokio::test]
async fn binds_and_serves_ephemeral() {
    let app = Router::new().route("/ping", get(|| async { "ok" }));
    let port = bind_and_serve(app, 0)
        .await
        .expect("port 0 应绑定临时端口并返回实际端口");
    assert!(port > 0);

    // 真实 loopback：连接 + 请求 + 响应（into_make_service_with_connect_info 全链路）。
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("专用端口应可连接");
    stream
        .write_all(b"GET /ping HTTP/1.1\r\nhost: t\r\nconnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    assert!(
        String::from_utf8_lossy(&buf).starts_with("HTTP/1.1 200"),
        "应经专用监听器返回 200"
    );
}

#[tokio::test]
async fn bind_conflict_returns_none() {
    // 与生产同址冲突（0.0.0.0 同端口）：macOS/BSD 对「127.0.0.1:p 已占 +
    // 0.0.0.0:p 再绑」会放行，只有完全相同的通配绑定才在各平台稳定 EADDRINUSE。
    let held = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
    let port = held.local_addr().unwrap().port();
    let app = Router::new().route("/ping", get(|| async { "ok" }));
    assert_eq!(
        bind_and_serve(app, port).await,
        None,
        "端口被占用应返回 None（调用方回退主端口）"
    );
    drop(held);
}
