use super::*;
#[test]
fn bounded_tickets_cancel_without_replay_or_blocking_controls() {
    let media = MediaState::default();
    media.claim();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut receipts = Vec::new();
    for _ in 0..50 {
        receipts.push(
            media
                .enqueue(&[0; crate::device::echomuse::frames::SPEAKER_CHUNK_BYTES], &tx)
                .unwrap(),
        );
    }
    assert!(
        media
            .enqueue(&[0; crate::device::echomuse::frames::SPEAKER_CHUNK_BYTES], &tx)
            .is_none(),
        "2s duration bound"
    );
    media.enqueue_eos(&tx); // Independent control remains immediately enqueueable.
    assert_eq!(rx.len(), 51);
    let ticket = media.take_write().unwrap();
    media.disconnect();
    assert!(!ticket.is_current());
    drop(ticket);
    for mut done in receipts {
        assert!(done.try_recv().is_err());
    }
    while rx.try_recv().is_ok() {} // Same operation performed under hub lock before re-claim.
    media.claim();
    assert!(media
        .enqueue(&[0; crate::device::echomuse::frames::SPEAKER_CHUNK_BYTES], &tx)
        .is_some());
}
#[test]
fn no_claim_and_closed_control_never_accept_audio() {
    let media = MediaState::default();
    let (tx, _) = tokio::sync::mpsc::unbounded_channel();
    assert!(media
        .enqueue(&[0; crate::device::echomuse::frames::SPEAKER_CHUNK_BYTES], &tx)
        .is_none());
    media.claim();
    media.close();
    media.claim();
    assert!(!media.ready());
    assert!(media
        .enqueue(&[0; crate::device::echomuse::frames::SPEAKER_CHUNK_BYTES], &tx)
        .is_none());
}
