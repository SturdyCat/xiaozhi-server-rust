//! 服务器全局实时日志流：`GET /api/logs`（**WebSocket**）。
//!
//! ## 目的
//! macApp 管理台连接后可实时看到服务器全部运行日志（tracing 输出的镜像流），
//! 无需 SSH 看容器 stdout；AI/脚本客户端也可以连上该 WS，一边调用 `/api/*`
//! 测试端点，一边实时读取日志观察效果。
//!
//! ## 实现
//! - [`LogHubLayer`] 是一个 `tracing_subscriber::Layer`：每条 tracing 事件被
//!   格式化成一行 `HH:MM:SS LEVEL [target] message`，同时写入
//!   **环形缓冲**（最近 [`BACKLOG`] 行，供新连接补历史）与 **broadcast 通道**
//!   （推给所有在线订阅者）。`main.rs` 里 `.with(log_stream::layer())` 挂载。
//! - WS 帧协议（全部 JSON 文本帧）：连接后先收一帧
//!   `{"type":"backlog","lines":[…]}`（最多 [`BACKLOG`] 行历史），之后每条新日志
//!   一帧 `{"type":"log","line":…}`；慢客户端丢行时补一条 lag 提示帧，不断流。
//! - 日志格式化只做「事件 → 行」，不落盘、不改变 stdout 的 fmt 输出。

use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use serde_json::json;
use tokio::sync::{broadcast, Mutex};
use tracing::Event;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer as _;

/// 环形缓冲行数：新 WS 连接可一次性补齐的最近日志。
const BACKLOG: usize = 500;
/// broadcast 通道容量：慢客户端允许积压的行数，超出则收到 lag 提示。
const CHANNEL: usize = 1024;

struct Hub {
    tx: broadcast::Sender<String>,
    ring: Mutex<VecDeque<String>>,
}

static HUB: OnceLock<Hub> = OnceLock::new();

fn hub() -> &'static Hub {
    HUB.get_or_init(|| Hub {
        tx: broadcast::channel(CHANNEL).0,
        ring: Mutex::new(VecDeque::with_capacity(BACKLOG)),
    })
}

/// 供 `main.rs` 挂进日志栈的全局采集层：`registry().with(filter).with(layer())…`。
/// 只有 INFO 及以上进入流（与默认 `EnvFilter("info")` 同档，保持轻量）。
pub fn layer<S>() -> impl tracing_subscriber::Layer<S> + Send + Sync + 'static
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    LogHubLayer.with_filter(LevelFilter::INFO)
}

struct LogHubLayer;

struct MessageVisitor {
    message: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else if !self.message.is_empty() {
            self.message.push_str(&format!(" {}={field:?} ", field.name()));
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for LogHubLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor {
            message: String::new(),
        };
        event.record(&mut visitor);
        let line = format!(
            "{} {:<5} [{}] {}",
            hhmmss(),
            event.metadata().level(),
            event.metadata().target(),
            visitor.message
        );
        let hub = hub();
        // 环形缓冲：只在写日志瞬间短持锁，绝不阻塞调用线程太久。
        if let Ok(mut ring) = hub.ring.try_lock() {
            if ring.len() >= BACKLOG {
                ring.pop_front();
            }
            ring.push_back(line.clone());
        }
        // 无订阅者时 Err 被忽略；慢客户端由接收端 lag 分支兜底。
        let _ = hub.tx.send(line);
    }
}

/// `UTC HH:MM:SS`（epoch 秒做纯算术换算，不引第三方时间库）。
fn hhmmss() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let tod = secs % 86_400;
    format!("{:02}:{:02}:{:02}", tod / 3600, (tod % 3600) / 60, tod % 60)
}
/// `GET /api/logs` 的 WebSocket 升级入口。
pub(super) async fn logs_ws(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(stream_logs)
}

async fn stream_logs(mut socket: WebSocket) {
    // 先订阅再取快照：订阅后、快照间的少量重叠行由「backlog 从 ring 取、live 从
    // broadcast 取，ring 在订阅前已包含」的顺序保证基本无洞；极端并发下最多重复一行。
    let mut rx = hub().tx.subscribe();
    let backlog: Vec<String> = hub()
        .ring
        .lock()
        .await
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    let frame = json!({ "type": "backlog", "lines": backlog }).to_string();
    if socket.send(Message::Text(frame.into())).await.is_err() {
        return;
    }
    loop {
        match rx.recv().await {
            Ok(line) => {
                let frame = json!({ "type": "log", "line": line }).to_string();
                if socket.send(Message::Text(frame.into())).await.is_err() {
                    return; // 客户端断开即可，服务端日志不受任何影响。
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                let frame =
                    json!({ "type": "log", "line": format!("…（日志过快，丢失 {n} 行）") })
                        .to_string();
                if socket.send(Message::Text(frame.into())).await.is_err() {
                    return;
                }
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

#[cfg(test)]
mod tests;
