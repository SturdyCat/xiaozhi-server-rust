//! `server/src/app/transport.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

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
