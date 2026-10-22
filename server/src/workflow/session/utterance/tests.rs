use super::*;
use crate::workflow::downlink::poll_abort;
use crate::plugins::vad::{pool::VadPool, VadEngine};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

struct FakeVad {
    tail: Vec<f32>,
    flushes: Arc<AtomicUsize>,
    resets: Arc<AtomicUsize>,
}
impl VadEngine for FakeVad {
    fn accept(&mut self, samples: &[f32], _: &mut dyn FnMut(Vec<f32>)) {
        self.tail.extend(samples);
    }
    fn flush(&mut self, cb: &mut dyn FnMut(Vec<f32>)) {
        self.flushes.fetch_add(1, Ordering::SeqCst);
        if !self.tail.is_empty() {
            cb(std::mem::take(&mut self.tail));
        }
    }
    fn reset(&mut self) {
        self.tail.clear();
        self.resets.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn speech_flushes_tail_once_and_no_speech_discards_ready_and_partial_segments() {
    let pool = VadPool::new();
    let flushes = Arc::new(AtomicUsize::new(0));
    let resets = Arc::new(AtomicUsize::new(0));
    let mut vad = pool
        .acquire("test".into(), || {
            Ok(Box::new(FakeVad {
                tail: vec![2.0],
                flushes: flushes.clone(),
                resets: resets.clone(),
            }))
        })
        .await
        .unwrap();
    let mut ready = vec![vec![1.0]];
    assert_eq!(
        finish_utterance(&mut vad, &mut ready, UtteranceEnd::Speech),
        vec![vec![1.0], vec![2.0]]
    );
    assert_eq!(flushes.load(Ordering::SeqCst), 1);
    assert_eq!(resets.load(Ordering::SeqCst), 1);
    assert!(
        finish_utterance(&mut vad, &mut ready, UtteranceEnd::Speech).is_empty(),
        "no duplicate ASR"
    );
    vad.accept(&[3.0], &mut |_| {});
    ready.push(vec![4.0]);
    let before = flushes.load(Ordering::SeqCst);
    assert!(finish_utterance(&mut vad, &mut ready, UtteranceEnd::NoSpeech).is_empty());
    assert_eq!(
        flushes.load(Ordering::SeqCst),
        before,
        "NoSpeech must not emit ASR work"
    );
    assert!(ready.is_empty());
    assert!(
        finish_utterance(&mut vad, &mut ready, UtteranceEnd::Speech).is_empty(),
        "no ghost next turn"
    );
}

#[tokio::test]
async fn pool_return_resets_before_reuse() {
    let pool = VadPool::new();
    let resets = Arc::new(AtomicUsize::new(0));
    let mut first = pool
        .acquire("test".into(), || {
            Ok(Box::new(FakeVad {
                tail: Vec::new(),
                flushes: Arc::new(AtomicUsize::new(0)),
                resets: resets.clone(),
            }))
        })
        .await
        .unwrap();
    first.accept(&[1.0], &mut |_| {});
    drop(first);
    assert_eq!(resets.load(Ordering::SeqCst), 1);
    let mut next = pool
        .acquire("test".into(), || panic!("must reuse"))
        .await
        .unwrap();
    let mut segments = Vec::new();
    next.flush(&mut |seg| segments.push(seg));
    assert!(segments.is_empty());
}

struct Frames(VecDeque<IncomingFrame>);
impl Transport for Frames {
    async fn send_text(&mut self, _: String) -> anyhow::Result<()> {
        Ok(())
    }
    async fn send_binary(&mut self, _: Vec<u8>) -> anyhow::Result<()> {
        Ok(())
    }
    async fn recv(&mut self) -> IncomingFrame {
        self.0.pop_front().unwrap_or(IncomingFrame::Closed)
    }
    fn try_recv(&mut self) -> Option<IncomingFrame> {
        self.0.pop_front()
    }
    async fn keepalive(&mut self) {}
}

#[tokio::test]
async fn reply_poll_preserves_ordered_bounded_tail_and_end_then_next_wakeup() {
    let mut inner = Frames(VecDeque::from([
        IncomingFrame::Binary(vec![1, 2]),
        IncomingFrame::EndOfUtterance(UtteranceEnd::Speech),
        IncomingFrame::Text(r#"{"type":"listen","state":"detect"}"#.into()),
    ]));
    let mut transport = TurnTransport::new(&mut inner, true);
    let abort = Arc::new(AtomicBool::new(false));
    assert!(!poll_abort(&mut transport, "test", &abort));
    assert!(
        !poll_abort(&mut transport, "test", &abort),
        "do not re-poll deferred frames"
    );
    assert!(matches!(transport.recv().await, IncomingFrame::Binary(b) if b == [1,2]));
    assert!(matches!(
        transport.recv().await,
        IncomingFrame::EndOfUtterance(UtteranceEnd::Speech)
    ));
    assert!(matches!(transport.recv().await, IncomingFrame::Text(s) if s.contains("detect")));
}

#[tokio::test]
async fn abort_still_interrupts_but_deferred_no_speech_is_not_lost() {
    let mut inner = Frames(VecDeque::from([
        IncomingFrame::EndOfUtterance(UtteranceEnd::NoSpeech),
        IncomingFrame::Text(r#"{"type":"abort"}"#.into()),
    ]));
    let mut transport = TurnTransport::new(&mut inner, true);
    let abort = Arc::new(AtomicBool::new(false));
    assert!(poll_abort(&mut transport, "test", &abort));
    assert!(matches!(
        transport.recv().await,
        IncomingFrame::EndOfUtterance(UtteranceEnd::NoSpeech)
    ));
}

#[tokio::test]
async fn disconnect_drops_deferred_speech_before_it_can_reach_asr() {
    let mut inner = Frames(VecDeque::from([
        IncomingFrame::Binary(vec![1]),
        IncomingFrame::EndOfUtterance(UtteranceEnd::Speech),
        IncomingFrame::Closed,
    ]));
    let mut transport = TurnTransport::new(&mut inner, true);
    assert!(poll_abort(
        &mut transport,
        "test",
        &Arc::new(AtomicBool::new(false))
    ));
    assert!(transport.pending.is_empty());
    assert!(matches!(transport.recv().await, IncomingFrame::Closed));
    assert!(matches!(transport.recv().await, IncomingFrame::Closed));
}

#[tokio::test]
async fn stale_vad_guard_does_not_reenter_new_signature_pool() {
    let pool = VadPool::new();
    let build = || {
        Ok(Box::new(FakeVad {
            tail: Vec::new(),
            flushes: Arc::new(AtomicUsize::new(0)),
            resets: Arc::new(AtomicUsize::new(0)),
        }) as Box<dyn VadEngine>)
    };
    let old = pool.acquire("old".into(), build).await.unwrap();
    let current = pool.acquire("new".into(), build).await.unwrap();
    drop(old);
    // 新代实例还借出着；旧实例不许混入当前代的 idle 列表。
    let built = AtomicBool::new(false);
    let _next = pool
        .acquire("new".into(), || {
            built.store(true, Ordering::SeqCst);
            build()
        })
        .await
        .unwrap();
    assert!(built.load(Ordering::SeqCst));
    drop(current);
}

#[test]
fn esp_reply_poll_retains_legacy_audio_drop_behavior() {
    let mut inner = Frames(VecDeque::from([IncomingFrame::Binary(vec![1])]));
    let mut transport = TurnTransport::new(&mut inner, false);
    let abort = Arc::new(AtomicBool::new(false));
    assert!(!poll_abort(&mut transport, "test", &abort));
    assert!(transport.pending.is_empty());
    assert!(transport.inner.0.is_empty());
}
