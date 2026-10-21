use super::*;
use crate::app::prefetch::{push_pcm_slice, MAX_DOWNLOAD_BYTES};
#[test]
fn oversized_provider_chunk_is_sliced_and_never_expands_duration_or_byte_budget() {
    let budget = DownloadBudget::new(100);
    let producer_budget = budget.clone();
    let (tx, mut rx) = mpsc::channel(16);
    let producer = std::thread::spawn(move || {
        push_pcm_slice(
            &producer_budget,
            &tx,
            &vec![0.25; 8000],
            16000,
            &AtomicBool::new(false),
        )
    });
    let mut samples = 0;
    let mut chunks = 0;
    while let Some(chunk) = rx.blocking_recv() {
        assert!(budget.queued_ms() <= 100);
        assert!(chunk.pcm.len() <= 1600);
        samples += chunk.pcm.len();
        chunks += 1;
        drop(chunk);
    }
    assert!(producer.join().unwrap());
    assert_eq!(samples, 8000);
    assert_eq!(chunks, 5);
    assert_eq!(budget.queued_ms(), 0);
}
#[test]
fn byte_limit_is_independent_and_abort_releases_a_waiting_producer() {
    let budget = DownloadBudget::new(10000);
    let abort = Arc::new(AtomicBool::new(false));
    assert!(budget.reserve_bytes(1, MAX_DOWNLOAD_BYTES, &abort, &|| false));
    let b = budget.clone();
    let a = abort.clone();
    let waiter = std::thread::spawn(move || b.reserve_bytes(1, 4, &a, &|| false));
    std::thread::sleep(Duration::from_millis(10));
    assert!(!waiter.is_finished());
    abort.store(true, Ordering::Relaxed);
    assert!(!waiter.join().unwrap());
    budget.release_bytes(1, MAX_DOWNLOAD_BYTES);
    assert_eq!(budget.queued_ms(), 0);
}

struct WrongRateTts;
impl TtsEngine for WrongRateTts {
    fn synthesize_stream(
        &self,
        _: &str,
        _: f32,
        _: i32,
        mut callback: TtsChunkCallback,
    ) -> Result<()> {
        assert!(!callback(24000, &[0.2; 100]));
        Ok(())
    }
    fn output_sample_rate(&self) -> u32 {
        16000
    }
    fn name(&self) -> &'static str {
        "wrong-rate-fake"
    }
}
#[tokio::test]
async fn provider_returning_ok_after_rejected_callback_is_not_complete_success() {
    let cache = Arc::new(TtsCache::new(2));
    let mut prepared = spawn_sentence_prefetch(
        abort_flag(),
        cache.clone(),
        budget(),
        gate(1),
        None,
        |_| SentenceSource {
            tts: Arc::new(WrongRateTts),
            cache_key: 77,
            hot_audio_key: None,
            speed: 1.0,
            speaker: 0,
        },
        sentences(&["x"]),
    );
    let item = prepared.recv().await.unwrap();
    let PreparedAudio::Stream {
        mut chunks, done, ..
    } = item.audio
    else {
        panic!()
    };
    assert!(chunks.recv().await.is_none());
    assert!(!done.await.unwrap());
    assert!(cache.get(77).is_none());
}
