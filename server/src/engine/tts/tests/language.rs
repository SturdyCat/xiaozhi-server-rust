//! 语言池必须先登记 single-flight，而不是仅在构建后去重 Arc。
use super::*;
use std::sync::{mpsc, Barrier};
use std::time::Duration;

fn kokoro() -> Config {
    let mut config = Config::default();
    config.tts.engine = "kokoro".into();
    config.tts.kokoro.lang = "zh".into();
    config
}

#[test]
fn same_language_single_flight_while_other_language_builds_independently() {
    let cfg = kokoro();
    let slot = Arc::new(TtsSlot::new(&cfg, fake("zh", 0.0)));
    let calls = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(9));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release = Arc::new(Mutex::new(release_rx));
    let mut workers = Vec::new();
    for _ in 0..8 {
        let (slot, calls, barrier, started, release) = (
            slot.clone(),
            calls.clone(),
            barrier.clone(),
            started_tx.clone(),
            release.clone(),
        );
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            slot.language_lease_with("en", |actual, lang| {
                assert_eq!(actual.kokoro.lang, "zh");
                assert_eq!(lang, "en");
                calls.fetch_add(1, Ordering::SeqCst);
                started.send(()).unwrap();
                release.lock().unwrap().recv().unwrap();
                Ok(fake("en", 1.0))
            })
        }));
    }
    barrier.wait();
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    // en 的建模明确卡住，ja 仍须能完成；不允许用整池全局构建锁假装 single-flight。
    let other_slot = slot.clone();
    let (other_tx, other_rx) = mpsc::channel();
    let other = std::thread::spawn(move || {
        let lease = other_slot.language_lease_with("ja", |_, _| Ok(fake("ja", 2.0)));
        other_tx.send(lease).unwrap();
    });
    let ja_result = other_rx.recv_timeout(Duration::from_secs(2));
    // 即便断言失败，也放开所有 fake builder，避免测试留下阻塞线程。
    for _ in 0..8 {
        release_tx.send(()).unwrap();
    }
    let leases: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    other.join().unwrap();
    let ja = ja_result.expect("unrelated language must not wait for en");
    assert_eq!(output(&ja), vec![2.0]);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "one actual build per language/generation"
    );
    for lease in &leases {
        assert!(Arc::ptr_eq(&lease.engine(), &leases[0].engine()));
        assert_eq!(lease.config().kokoro.lang, "en");
        assert_eq!(lease.generation(), 1);
        assert_eq!(output(lease), vec![1.0]);
    }
    assert_ne!(mem(&key(&leases[0])), mem(&key(&slot.lease())));
    assert_eq!(slot.pool.lock().unwrap().len(), 2);

    let mut params_only = cfg.clone();
    params_only.tts.speaker = 7;
    params_only.tts.speed = 0.8;
    assert!(!slot
        .refresh_with(&params_only, |_| panic!("parameters do not rebuild"))
        .unwrap());
    let updated = slot.language_lease_with("en", |_, _| panic!("successful flight remains cached"));
    assert!(Arc::ptr_eq(&updated.engine(), &leases[0].engine()));
    assert_eq!(updated.config().speaker, 7);
    assert_eq!(updated.config().speed, 0.8);
    assert_eq!(updated.generation(), 1);

    let mut next_config = cfg;
    next_config.tts.kokoro.voices = "new-voices.bin".into();
    slot.refresh_with(&next_config, |_| Ok(fake("new-zh", 3.0)))
        .unwrap();
    let next = slot.language_lease_with("en", |_, _| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(fake("new-en", 4.0))
    });
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "new generation gets a new single flight"
    );
    assert_eq!(next.generation(), 2);
    assert_eq!(output(&next), vec![4.0]);
    assert_ne!(mem(&key(&next)), mem(&key(&leases[0])));
}

#[test]
fn stale_language_flight_cannot_reenter_new_generation_pool() {
    let cfg = kokoro();
    let slot = Arc::new(TtsSlot::new(&cfg, fake("old-zh", 0.0)));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let old_slot = slot.clone();
    let old = std::thread::spawn(move || {
        old_slot.language_lease_with("en", |_, _| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(fake("old-en", 1.0))
        })
    });
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let mut changed = cfg;
    changed.tts.kokoro.model = "new-model.onnx".into();
    slot.refresh_with(&changed, |_| Ok(fake("new-zh", 2.0)))
        .unwrap();
    let new = slot.language_lease_with("en", |_, _| Ok(fake("new-en", 3.0)));
    release_tx.send(()).unwrap();
    let old = old.join().unwrap();
    assert_eq!(old.generation(), 1);
    assert_eq!(old.engine().name(), "old-en");
    assert_eq!(new.generation(), 2);
    let cached =
        slot.language_lease_with("en", |_, _| panic!("new generation must retain its cell"));
    assert!(Arc::ptr_eq(&new.engine(), &cached.engine()));
    let pool = slot.pool.lock().unwrap();
    assert_eq!(pool.len(), 1);
    assert!(pool.contains_key(&(2, "en".to_string())));
}

#[test]
fn failed_language_build_retains_actual_default_identity_and_can_retry() {
    let slot = TtsSlot::new(&kokoro(), fake("zh", 1.0));
    let default = slot.lease();
    let fallback = slot.language_lease_with("en", |_, _| Err("fake missing model".into()));
    assert!(Arc::ptr_eq(&fallback.engine(), &default.engine()));
    assert_eq!(fallback.config().kokoro.lang, "zh");
    assert_eq!(fallback.fingerprint(), default.fingerprint());
    assert_eq!(key(&fallback), key(&default));
    assert!(slot.pool.lock().unwrap().is_empty());
    let retried = slot.language_lease_with("en", |_, _| Ok(fake("en", 2.0)));
    let cached = slot.language_lease_with("en", |_, _| panic!("must share successful flight"));
    assert!(Arc::ptr_eq(&retried.engine(), &cached.engine()));
    assert_ne!(key(&retried), key(&default));
}
