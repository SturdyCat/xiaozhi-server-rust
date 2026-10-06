//! 会话传输抽象：承载无关的收发原语。会话状态机与流水线（[`crate::session`] /
//! [`crate::downlink`]）只依赖 [`Transport`]，不感知 WebSocket / 未来 MQTT+UDP /
//! 本机测试通道等具体承载——接新设备形态时实现一份 trait 即可复用整条流水线。
//!
//! ## 设计取舍（借鉴而不照搬，选取与本项目规模相称的最小面）
//!
//! - **pipecat 的 transport 分离与 SystemFrame**：打断/断开是必须以最高优先级
//!   穿透流水线的控制事件——这里对应 [`IncomingFrame::Closed`] 与 abort 文本消息，
//!   由流水线在合成/下发的每个间隙经 [`Transport::try_recv`] 非阻塞轮询
//!   （原 `poll_abort` 语义不变，只是不再绑定 axum 的 `WebSocket` 类型）。
//! - **flowcat 的 carrier**：保活是**承载关注点**（WS 发 Ping，MQTT 走自身心跳），
//!   协议层不感知，故收敛进 [`Transport::keepalive`]。
//! - **不上 FrameProcessor 全家桶**：流水线拓扑固定为线性三级，泛型静态分发
//!   （零开销、免 async-trait 依赖）即满足；待出现第二承载且需运行期切换时再引入
//!   `dyn`。方法签名手写 RPITIT + `Send`（裸 `async fn` 的返回值无法向泛型调用方
//!   保证 `Send`，过不了 axum `on_upgrade`）。
//!
//! [`WsTransport`] 为当前唯一实现。`Closed` **粘滞**：承载一旦断开，后续
//! recv/try_recv 恒报 `Closed`——`try_recv` 的 `None` 只表示"暂无待处理消息"，
//! 两者不混淆，轮询方不会把半关闭状态误判为"安静"。

use std::future::Future;

use anyhow::Result;
use axum::extract::ws::{Message, WebSocket};
use futures_util::{FutureExt, StreamExt};

/// 上行帧：承载层原样交付，协议解析留在会话层（[`crate::protocol`]）。
#[derive(Debug)]
pub enum IncomingFrame {
    /// 文本消息（xiaozhi 协议 JSON）。
    Text(String),
    /// 二进制消息（上行 Opus 包）。
    Binary(Vec<u8>),
    /// 承载已断开/出错。粘滞：此后 recv/try_recv 恒返回 `Closed`。
    Closed,
}

/// 会话传输：下行发送 + 上行收取 + 保活，三组承载原语。
pub trait Transport: Send {
    /// 发送协议文本消息（`ServerMessage::to_json` 的产物）。
    fn send_text(&mut self, json: String) -> impl Future<Output = Result<()>> + Send;

    /// 发送二进制帧（下行 Opus 包，已按协商版本封装）。
    fn send_binary(&mut self, data: Vec<u8>) -> impl Future<Output = Result<()>> + Send;

    /// 阻塞收取下一条上行帧（会话主循环）；断开后恒返回 [`IncomingFrame::Closed`]。
    fn recv(&mut self) -> impl Future<Output = IncomingFrame> + Send;

    /// 非阻塞收取（流水线间隙轮询打断/断开用）；无待处理消息返回 `None`。
    fn try_recv(&mut self) -> Option<IncomingFrame>;

    /// 承载层保活。WS 实现发 Ping（tungstenite 自动回 Pong）；
    /// 自带心跳机制的承载（如 MQTT）留空即可。
    fn keepalive(&mut self) -> impl Future<Output = ()> + Send;
}

/// [`Transport`] 的 WebSocket 实现（xiaozhi 设备与 macApp 测试台共用）。
pub struct WsTransport {
    socket: WebSocket,
    closed: bool,
}

impl WsTransport {
    pub fn new(socket: WebSocket) -> Self {
        Self { socket, closed: false }
    }
}

impl Transport for WsTransport {
    async fn send_text(&mut self, json: String) -> Result<()> {
        self.socket
            .send(Message::Text(json.into()))
            .await
            .map_err(|e| anyhow::anyhow!("发送文本失败: {e}"))
    }

    async fn send_binary(&mut self, data: Vec<u8>) -> Result<()> {
        self.socket
            .send(Message::Binary(data.into()))
            .await
            .map_err(|e| anyhow::anyhow!("发送二进制失败: {e}"))
    }

    async fn recv(&mut self) -> IncomingFrame {
        if self.closed {
            return IncomingFrame::Closed;
        }
        loop {
            match self.socket.next().await {
                Some(Ok(Message::Text(t))) => return IncomingFrame::Text(t.to_string()),
                Some(Ok(Message::Binary(b))) => return IncomingFrame::Binary(b.to_vec()),
                // Ping/Pong 等控制帧：axum 内部自动应答，应用层忽略、继续等下一帧
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    tracing::debug!("承载读取错误（视为断开）: {e}");
                    self.closed = true;
                    return IncomingFrame::Closed;
                }
                None => {
                    self.closed = true;
                    return IncomingFrame::Closed;
                }
            }
        }
    }

    fn try_recv(&mut self) -> Option<IncomingFrame> {
        if self.closed {
            return Some(IncomingFrame::Closed);
        }
        loop {
            match self.socket.next().now_or_never() {
                None => return None, // 暂无待处理消息
                Some(None) => {
                    self.closed = true;
                    return Some(IncomingFrame::Closed);
                }
                Some(Some(Ok(Message::Text(t)))) => return Some(IncomingFrame::Text(t.to_string())),
                Some(Some(Ok(Message::Binary(b)))) => {
                    return Some(IncomingFrame::Binary(b.to_vec()))
                }
                Some(Some(Ok(_))) => {}
                Some(Some(Err(e))) => {
                    tracing::debug!("承载读取错误（视为断开）: {e}");
                    self.closed = true;
                    return Some(IncomingFrame::Closed);
                }
            }
        }
    }

    async fn keepalive(&mut self) {
        let _ = self.socket.send(Message::Ping("ping".into())).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 内存实现：预置一批上行帧，收完即"断开"。
    /// 证明流水线/打断轮询可在无网络条件下单测。
    struct MockTransport {
        frames: Vec<IncomingFrame>,
    }

    impl Transport for MockTransport {
        async fn send_text(&mut self, _json: String) -> Result<()> {
            Ok(())
        }
        async fn send_binary(&mut self, _data: Vec<u8>) -> Result<()> {
            Ok(())
        }
        async fn recv(&mut self) -> IncomingFrame {
            if self.frames.is_empty() {
                IncomingFrame::Closed
            } else {
                self.frames.remove(0)
            }
        }
        fn try_recv(&mut self) -> Option<IncomingFrame> {
            (!self.frames.is_empty()).then(|| self.frames.remove(0))
        }
        async fn keepalive(&mut self) {}
    }

    #[tokio::test]
    async fn recv_delivers_frames_then_sticky_closed() {
        let mut t = MockTransport {
            frames: vec![IncomingFrame::Text("a".into()), IncomingFrame::Binary(vec![1])],
        };
        assert!(matches!(t.recv().await, IncomingFrame::Text(_)));
        assert!(matches!(t.recv().await, IncomingFrame::Binary(_)));
        assert!(matches!(t.recv().await, IncomingFrame::Closed));
        // 粘滞：断开后恒为 Closed
        assert!(matches!(t.recv().await, IncomingFrame::Closed));
    }

    #[tokio::test]
    async fn try_recv_returns_none_when_idle() {
        let mut t = MockTransport { frames: vec![] };
        assert!(t.try_recv().is_none(), "无待处理消息应为 None 而非 Closed");
    }
}
