//! MCP 的音频策略必须把 typed 结束事件与尾包作为一个有序单位处理。
use super::*;
use crate::workflow::protocol::UtteranceEnd;

#[tokio::test]
async fn discovery_keeps_tail_and_end_but_stop_listening_drops_both() {
    for policy in [BinaryPolicy::Keep, BinaryPolicy::Drop] {
        let mut transport = MockTransport::new(vec![
            IncomingFrame::Binary(vec![7, 8]),
            IncomingFrame::EndOfUtterance(UtteranceEnd::Speech),
            reply_frame(1),
        ]);
        let (result, deferred) =
            pump_reply(&mut transport, 1, Duration::from_secs(1), policy).await;
        assert!(result.is_ok());
        if policy == BinaryPolicy::Keep {
            assert_eq!(deferred.len(), 2);
            assert!(matches!(&deferred[0], IncomingFrame::Binary(b) if b == &[7, 8]));
            assert!(matches!(
                deferred[1],
                IncomingFrame::EndOfUtterance(UtteranceEnd::Speech)
            ));
        } else {
            assert!(deferred.is_empty(), "退下等待不能再回灌结束事件和残留音频");
        }
    }
}
