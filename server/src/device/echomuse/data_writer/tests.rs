use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
#[tokio::test]
async fn stalled_actual_writer_processes_uplink_and_acks_only_after_sink_completion() {
    let media = MediaState::default();
    media.claim();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut receipt = media
        .enqueue(
            &[0; crate::device::echomuse::frames::SPEAKER_CHUNK_BYTES],
            &tx,
        )
        .unwrap();
    let frame = rx.try_recv().unwrap();
    let (release, wait) = oneshot::channel::<()>();
    let mut sink = Box::pin(futures_util::sink::unfold(
        Some(wait),
        |mut wait, _: Message| async move {
            if let Some(wait) = wait.take() {
                let _ = wait.await;
            }
            Ok::<_, ()>(wait)
        },
    ));
    let mut stream = futures_util::stream::iter(vec![Ok::<_, ()>(Message::Binary(vec![4].into()))])
        .chain(futures_util::stream::pending());
    let seen = Arc::new(AtomicUsize::new(0));
    let seen_cb = seen.clone();
    let pending = write_frame(&mut sink, &mut stream, frame, &media, move |raw| {
        assert_eq!(raw, vec![4]);
        seen_cb.fetch_add(1, Ordering::Relaxed);
    });
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(5), &mut pending)
            .await
            .is_err()
    );
    assert_eq!(
        seen.load(Ordering::Relaxed),
        1,
        "input sentinel was handled during stalled output"
    );
    assert!(matches!(
        receipt.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    release.send(()).unwrap();
    assert!(pending.await);
    assert!(receipt.await.unwrap());
}
#[tokio::test]
async fn control_close_interrupts_stalled_writer_and_cancels_audio_receipt() {
    let media = MediaState::default();
    media.claim();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let receipt = media
        .enqueue(
            &[0; crate::device::echomuse::frames::SPEAKER_CHUNK_BYTES],
            &tx,
        )
        .unwrap();
    let mut sink = Box::pin(futures_util::sink::unfold(
        (),
        |(), _: Message| async move {
            futures_util::future::pending::<()>().await;
            Ok::<_, ()>(())
        },
    ));
    let mut stream = futures_util::stream::pending::<Result<Message, ()>>();
    let pending = write_frame(
        &mut sink,
        &mut stream,
        rx.try_recv().unwrap(),
        &media,
        |_| {},
    );
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(1), &mut pending)
            .await
            .is_err()
    );
    media.close();
    assert!(!pending.await);
    assert!(!receipt.await.unwrap());
}
#[tokio::test]
async fn ping_pong_do_not_abort_the_write() {
    // 回归：Ping/Pong 是协议层心跳，绝不能像断开一样中止发送。此前写帧循环的
    // `_ => break false` 会把设备回我们 Ping 的那次 Pong 当成发送失败返回 false，
    // 上层 data_loop 随即断开整条 /data 链（设备侧 close 1006 循环）。
    let media = MediaState::default();
    media.claim();
    let (tx, mut rx) = mpsc::unbounded_channel();
    media.enqueue(
        &[0; crate::device::echomuse::frames::SPEAKER_CHUNK_BYTES],
        &tx,
    );
    let frame = rx.try_recv().unwrap();
    // sink 卡在 oneshot 上：逼 select 先消费 stream 里的心跳帧。
    let (release, wait) = oneshot::channel::<()>();
    let mut sink = Box::pin(futures_util::sink::unfold(
        Some(wait),
        |mut wait, _: Message| async move {
            if let Some(wait) = wait.take() {
                let _ = wait.await;
            }
            Ok::<_, ()>(wait)
        },
    ));
    let mut stream = futures_util::stream::iter(vec![
        Ok::<_, ()>(Message::Ping(Vec::<u8>::new().into())),
        Ok::<_, ()>(Message::Pong(Vec::<u8>::new().into())),
        Ok::<_, ()>(Message::Binary(vec![7].into())),
    ])
    .chain(futures_util::stream::pending());
    let seen = Arc::new(AtomicUsize::new(0));
    let seen_cb = seen.clone();
    let pending = write_frame(&mut sink, &mut stream, frame, &media, move |raw| {
        assert_eq!(raw, vec![7]);
        seen_cb.fetch_add(1, Ordering::Relaxed);
    });
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(5), &mut pending)
            .await
            .is_err(),
        "Ping/Pong 不得让写帧提前返回"
    );
    assert_eq!(
        seen.load(Ordering::Relaxed),
        1,
        "Ping/Pong 之后的上行二进制仍必须被交付（心跳不是断链）"
    );
    release.send(()).unwrap();
    assert!(pending.await, "放行后正常写完");
}

#[tokio::test]
async fn stale_eos_ticket_is_discarded_without_touching_socket() {
    let media = MediaState::default();
    media.claim();
    let (tx, mut rx) = mpsc::unbounded_channel();
    media.enqueue_eos(&tx);
    let frame = rx.try_recv().unwrap();
    media.disconnect();
    let sends = Arc::new(AtomicUsize::new(0));
    let sends_cb = sends.clone();
    let mut sink = Box::pin(futures_util::sink::unfold((), move |(), _: Message| {
        sends_cb.fetch_add(1, Ordering::Relaxed);
        async { Ok::<_, ()>(()) }
    }));
    let mut stream = futures_util::stream::pending::<Result<Message, ()>>();
    assert!(write_frame(&mut sink, &mut stream, frame, &media, |_| {}).await);
    assert_eq!(sends.load(Ordering::Relaxed), 0);
}
