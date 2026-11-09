//! S3l：宿主**出站 TCP**（`tcp-connect`）回环验证。
//!
//! NetManager 直连回环：插件视角的完整生命周期——connect → `io.tcp.opened` →
//! `net-send` → 对端 echo → `io.tcp.data`（correlation-id = connection-id）→
//! `net-close` → `io.tcp.closed`；外加连接拒绝错误路径与 id 唯一性（全局
//! 命名空间，与端点 id 不相撞）。

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use crate::plugins::wasm::event_bus::Envelope;
use crate::plugins::wasm::net::{NetManager, NioSink};

/// 收集投递事件的 sink（测试断言用）。
#[derive(Clone, Default)]
struct CollectSink {
    events: Arc<Mutex<Vec<Envelope>>>,
}

impl NioSink for CollectSink {
    fn deliver(&self, _plugin_id: &str, env: Envelope) {
        if let Ok(mut evs) = self.events.lock() {
            evs.push(env);
        }
    }
}

impl CollectSink {
    fn wait_for(&self, topic: &str, timeout_ms: u64) -> Envelope {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            if let Ok(evs) = self.events.lock() {
                if let Some(env) = evs.iter().rev().find(|e| e.topic == topic) {
                    return env.clone();
                }
            }
            if std::time::Instant::now() >= deadline {
                panic!("等待事件 {topic} 超时");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

/// 回环 echo 服务：读多少回多少。
async fn spawn_echo() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                loop {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if sock.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
    });
    port
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn outbound_tcp_roundtrip_lifecycle_and_events() {
    let port = spawn_echo().await;
    let sink = CollectSink::default();
    let net = NetManager::new(
        Some(tokio::runtime::Handle::current()),
        Arc::new(sink.clone()),
    );

    // 连接：opened 事件带 connection_id 与 peer。
    let conn_id = net
        .tcp_connect("echo-plugin", "127.0.0.1", port)
        .expect("出站连接失败");
    let opened = sink.wait_for("io.tcp.opened", 2_000);
    assert_eq!(opened.topic, "io.tcp.opened");
    assert!(
        String::from_utf8_lossy(&opened.payload).contains(&format!("\"connection_id\":{conn_id}")),
        "opened 事件应带本连接 id：{}",
        String::from_utf8_lossy(&opened.payload)
    );

    // 出站写 → 对端 echo → data 事件（correlation-id = connection-id，原始字节）。
    net.send("echo-plugin", conn_id, b"ping-123")
        .expect("出站写失败");
    let data = sink.wait_for("io.tcp.data", 2_000);
    assert_eq!(
        data.correlation_id.as_deref(),
        Some(conn_id.to_string()).as_deref()
    );
    assert_eq!(data.payload, b"ping-123".to_vec(), "echo 回读应逐字节一致");

    // 归属校验：别的插件 id 不能动这条连接。
    assert!(net.send("other-plugin", conn_id, b"x").is_err());
    assert!(net.close("other-plugin", conn_id, "test").is_err());

    // 关闭：closed 事件。
    net.close("echo-plugin", conn_id, "test").expect("关闭失败");
    let closed = sink.wait_for("io.tcp.closed", 2_000);
    assert_eq!(
        closed.correlation_id.as_deref(),
        Some(conn_id.to_string()).as_deref(),
        "closed 事件 correlation-id 应等于 connection-id（S3p 修复）：{closed:?}"
    );
    assert!(
        String::from_utf8_lossy(&closed.payload).contains("\"reason\":\"host\""),
        "closed 事件应带 reason=host：{}",
        String::from_utf8_lossy(&closed.payload)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn outbound_connect_refused_is_explicit_error() {
    let sink = CollectSink::default();
    let net = NetManager::new(
        Some(tokio::runtime::Handle::current()),
        Arc::new(sink.clone()),
    );
    // 回环上没有监听者的端口 → 立刻 ECONNREFUSED，显式报错。
    let refused = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let dead_port = refused.local_addr().expect("addr").port();
    drop(refused);
    let err = net
        .tcp_connect("echo-plugin", "127.0.0.1", dead_port)
        .err()
        .expect("连接已死端口应失败");
    assert!(
        format!("{err:#}").contains("失败"),
        "错误应说明连接失败：{err:#}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn outbound_ids_come_from_global_namespace() {
    let port = spawn_echo().await;
    let sink = CollectSink::default();
    let net = NetManager::new(
        Some(tokio::runtime::Handle::current()),
        Arc::new(sink.clone()),
    );
    let a = net
        .tcp_connect("echo-plugin", "127.0.0.1", port)
        .expect("第一条出站连接");
    let b = net
        .tcp_connect("echo-plugin", "127.0.0.1", port)
        .expect("第二条出站连接");
    assert_ne!(a, b, "出站连接 id 必须唯一（全局分配器）");
    net.close("echo-plugin", a, "test").ok();
    net.close("echo-plugin", b, "test").ok();
}
