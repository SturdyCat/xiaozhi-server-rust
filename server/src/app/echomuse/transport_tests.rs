//! Serializer/carrier contracts through native production framer and write receipts.
use super::super::frames;
use super::*;
use crate::app::protocol::ServerMessage;
fn harness() -> (EchomuseTransport, UnboundedReceiver<DataOut>) {
    let (_up, rx) = tokio::sync::mpsc::unbounded_channel();
    let (ctl, _ctl_rx) = tokio::sync::mpsc::unbounded_channel();
    let (data, data_rx) = tokio::sync::mpsc::unbounded_channel();
    (EchomuseTransport::new(rx, ctl, data).unwrap(), data_rx)
}
fn stop() -> String {
    ServerMessage::Tts {
        session_id: "s".into(),
        state: "stop".into(),
        text: None,
    }
    .to_json()
}
#[tokio::test]
async fn native_framer_pads_tail_then_socket_receipt_precedes_eos() {
    let (mut transport, mut data) = harness();
    let mut framer =
        crate::app::downlink::framer::StreamingFramer::new_format(16000, AudioFormat::echo())
            .unwrap();
    assert!(framer.push(&[0.5, -0.5]).unwrap().is_empty());
    let tail = framer.finish().unwrap().unwrap();
    assert_eq!(tail.len(), 1280);
    assert!(tail[4..].iter().all(|b| *b == 0));
    let media = transport.media.clone();
    {
        let pending = transport.send_audio(
            AudioChunk::new(AudioFormat::echo(), tail.clone()),
            TerminalProfile::echo(),
            0,
        );
        tokio::pin!(pending);
        assert!(
            tokio::time::timeout(Duration::from_millis(1), &mut pending)
                .await
                .is_err(),
            "enqueue is NOT a socket write ACK"
        );
        assert_eq!(data.try_recv().unwrap(), frames::frame_speaker(&tail));
        media.take_write().unwrap().complete(true);
        assert_eq!(pending.await.unwrap(), AudioWrite::SocketWritten);
    }
    transport.send_text(stop()).await.unwrap();
    assert_eq!(data.try_recv().unwrap(), frames::frame_speaker_eos());
    assert!(data.try_recv().is_err());
    assert!(
        transport.send_binary(vec![1]).await.is_err(),
        "legacy codec roundtrip is impossible"
    );
}
#[tokio::test]
async fn structural_stop_ignores_text_lookalikes_and_empty_streams_get_eos() {
    let (mut transport, mut data) = harness();
    for json in [
        r#"{"type":"tts","state":"start"}"#,
        r#"{"type":"tts_stop","state":"stop"}"#,
        r#"{"type":"stt","state":"stop"}"#,
        r#"{"payload":{"type":"tts","state":"stop"}}"#,
        r#"{"type":"tts","state":false}"#,
        "null",
        "bad json",
    ] {
        transport.send_text(json.into()).await.unwrap();
        assert!(data.try_recv().is_err());
    }
    transport
        .send_text("{ \"state\" : \"stop\", \"type\" : \"tts\" }".into())
        .await
        .unwrap();
    assert_eq!(data.try_recv().unwrap(), frames::frame_speaker_eos());
}
#[tokio::test]
async fn lost_data_cancels_receipt_and_never_queues_eos_for_reconnect() {
    let (mut transport, mut data) = harness();
    let media = transport.media.clone();
    {
        let pending = transport.send_audio(
            AudioChunk::new(AudioFormat::echo(), vec![0; 1280]),
            TerminalProfile::echo(),
            0,
        );
        tokio::pin!(pending);
        assert!(tokio::time::timeout(Duration::from_millis(1), &mut pending)
            .await
            .is_err());
        data.try_recv().unwrap();
        media.disconnect();
        media.claim(); // Reclaim BEFORE the old receipt's failed await resumes.
        assert_eq!(pending.await.unwrap(), AudioWrite::Cancelled);
        assert!(
            media.ready(),
            "old receipt cleanup must not disconnect replacement"
        );
    }
    transport.send_text(stop()).await.unwrap();
    assert!(data.try_recv().is_err());
    transport
        .send_text(r#"{"type":"tts","state":"start"}"#.into())
        .await
        .unwrap();
    transport.send_text(stop()).await.unwrap();
    assert_eq!(data.try_recv().unwrap(), frames::frame_speaker_eos());
}
