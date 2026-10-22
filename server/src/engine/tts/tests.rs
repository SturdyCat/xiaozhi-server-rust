//! 无模型/网络的实际租约与缓存回归；fake 标记实例输出，不能只验证配置字符串。
use super::*;

mod language;
use crate::app::hot_audio::{HotAudioKey, HotAudioStore};
use crate::app::tts_cache::TtsCache;
use crate::plugins::tts::TtsChunkCallback;
use std::sync::atomic::{AtomicUsize, Ordering};

struct FakeTts(&'static str, f32);
impl TtsEngine for FakeTts {
    fn synthesize_stream(
        &self,
        _: &str,
        _: f32,
        _: i32,
        mut cb: TtsChunkCallback,
    ) -> anyhow::Result<()> {
        cb(24000, &[self.1]);
        Ok(())
    }
    fn name(&self) -> &'static str {
        self.0
    }
}
fn fake(name: &'static str, value: f32) -> Arc<dyn TtsEngine> {
    Arc::new(FakeTts(name, value))
}
fn cfg(voice: &str) -> Config {
    let mut cfg = Config::default();
    cfg.tts.engine = "xfyun".into();
    cfg.tts.xfyun.voice = voice.into();
    cfg
}
fn key(lease: &TtsLease) -> HotAudioKey {
    HotAudioKey::from_lease(lease, "same sentence")
}

/// 测试用下行档位（只影响内存键；磁盘身份不含它）。
fn fmt() -> crate::app::audio::media::AudioFormat {
    use crate::app::audio::media::{AudioCodec, AudioFormat};
    AudioFormat::new(AudioCodec::Opus, 24_000, 60)
}

fn mem(k: &HotAudioKey) -> u64 {
    k.mem_key(fmt())
}

/// 测试用 PCM：70ms 24k 静音（S16LE）。
fn pcm() -> Vec<u8> {
    vec![0u8; 70 * 48]
}
fn output(lease: &TtsLease) -> Vec<f32> {
    lease.engine().synthesize("test", 1.0, 0).unwrap().0
}

#[test]
fn failed_build_retains_instance_config_fingerprint_generation_and_cache_identity() {
    let initial = cfg("old");
    let slot = TtsSlot::new(&initial, fake("old", 1.0));
    let old = slot.lease();
    let cache = TtsCache::new(8);
    cache.insert(mem(&key(&old)), vec![vec![1, 2, 3]]);
    assert!(slot
        .refresh_with(&cfg("bad-new"), |_| Err("fake failure".into()))
        .is_err());
    let retained = slot.lease();
    assert!(Arc::ptr_eq(&old.engine(), &retained.engine()));
    assert_eq!(retained.config().xfyun.voice, "old");
    assert_eq!(retained.fingerprint(), old.fingerprint());
    assert_eq!(retained.generation(), old.generation());
    assert_eq!(output(&retained), vec![1.0]);
    assert!(cache.get(mem(&key(&retained))).is_some());
    assert_ne!(
        key(&retained),
        HotAudioKey::from_session(&cfg("bad-new"), "same sentence", 24_000)
    );
}

#[test]
fn successful_refresh_keeps_old_session_pinned_and_separates_cache_entries() {
    let slot = TtsSlot::new(&cfg("old"), fake("old", 1.0));
    let old_session = slot.lease().with_synthesis_params(3, 1.2);
    assert!(slot
        .refresh_with(&cfg("new"), |_| Ok(fake("new", 2.0)))
        .unwrap());
    let new_session = slot.lease();
    assert_eq!(output(&old_session), vec![1.0]);
    assert_eq!(output(&new_session), vec![2.0]);
    assert_eq!(old_session.config().speaker, 3);
    assert_eq!(old_session.config().speed, 1.2);
    assert_eq!(new_session.generation(), old_session.generation() + 1);
    assert_ne!(mem(&key(&old_session)), mem(&key(&new_session)));
    let cache = TtsCache::new(8);
    cache.insert(mem(&key(&old_session)), vec![vec![1]]);
    assert!(cache.get(mem(&key(&new_session))).is_none());
    assert!(cache.get(mem(&key(&old_session))).is_some());
}

#[test]
fn unchanged_build_identity_updates_parameters_without_rebuild_or_generation_change() {
    let original = cfg("old");
    let slot = TtsSlot::new(&original, fake("old", 1.0));
    let mut changed = original.clone();
    changed.tts.speed = 1.3;
    changed.tts.speaker = 7;
    let old = slot.lease();
    assert!(!slot
        .refresh_with(&changed, |_| panic!("must not build"))
        .unwrap());
    let new = slot.lease();
    assert!(Arc::ptr_eq(&old.engine(), &new.engine()));
    assert_eq!(new.generation(), old.generation());
    assert_eq!(new.fingerprint(), old.fingerprint());
    assert_ne!(mem(&key(&old)), mem(&key(&new)));
}

#[test]
fn serialized_concurrent_refresh_cannot_publish_old_build_after_new_build() {
    let slot = Arc::new(TtsSlot::new(&cfg("initial"), fake("initial", 0.0)));
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let completed = Arc::new(AtomicUsize::new(0));
    let first_slot = slot.clone();
    let first = std::thread::spawn(move || {
        first_slot
            .refresh_with(&cfg("older"), |_| {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(fake("older", 1.0))
            })
            .unwrap()
    });
    started_rx.recv().unwrap();
    let second_slot = slot.clone();
    let second_completed = completed.clone();
    let second = std::thread::spawn(move || {
        second_slot
            .refresh_with(&cfg("newest"), |_| {
                second_completed.fetch_add(1, Ordering::SeqCst);
                Ok(fake("newest", 2.0))
            })
            .unwrap()
    });
    assert_eq!(
        completed.load(Ordering::SeqCst),
        0,
        "new build waits for older refresh"
    );
    release_tx.send(()).unwrap();
    assert!(first.join().unwrap());
    assert!(second.join().unwrap());
    assert_eq!(slot.lease().config().xfyun.voice, "newest");
    assert_eq!(slot.lease().engine().name(), "newest");
    assert_eq!(output(&slot.lease()), vec![2.0]);
    assert_eq!(slot.lease().generation(), 3);
}

#[tokio::test]
async fn disk_cache_uses_retained_actual_identity_after_failed_refresh() {
    let dir = std::env::temp_dir().join(format!(
        "lease-cache-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let slot = TtsSlot::new(&cfg("old"), fake("old", 1.0));
    let actual = key(&slot.lease());
    let store = Arc::new(HotAudioStore::new(Some(dir.clone())));
    let cache = TtsCache::with_store(4, store.clone());
    cache.register_identity(actual.clone(), mem(&actual));
    cache
        .insert_async(mem(&actual), vec![vec![9, 8, 7]], Some(pcm()))
        .await;
    cache.clear();
    assert!(slot
        .refresh_with(&cfg("bad"), |_| Err("failed".into()))
        .is_err());
    let retained = key(&slot.lease());
    cache.register_identity(retained.clone(), mem(&retained));
    let hit = cache.get_async(mem(&retained), fmt()).await.unwrap();
    assert!(
        !hit.opus_frames.is_empty(),
        "磁盘命中按会话格式从 PCM 重新编码"
    );
    assert!(store
        .get(&HotAudioKey::from_session(&cfg("bad"), "same sentence", 24_000))
        .is_none());
    // 只清理由本测试创建的明确临时目录。
    assert!(dir
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("lease-cache-"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn kokoro_signature_covers_every_constructor_input_without_delimiter_collisions() {
    let original = Config::default();
    let sig = capability_signature(&original, Capability::Tts);
    for field in [
        "model",
        "voices",
        "tokens",
        "data_dir",
        "dict_dir",
        "lexicon",
        "lang",
        "num_threads",
    ] {
        let mut value = serde_json::to_value(&original).unwrap();
        let current = &mut value["tts"]["kokoro"][field];
        *current = if field == "num_threads" {
            serde_json::json!(17)
        } else {
            serde_json::json!("changed")
        };
        let changed: Config = serde_json::from_value(value).unwrap();
        assert_ne!(
            sig,
            capability_signature(&changed, Capability::Tts),
            "missing {field}"
        );
    }
    let mut a = original.clone();
    let mut b = original.clone();
    a.tts.kokoro.model = "a|b".into();
    a.tts.kokoro.voices = "c".into();
    b.tts.kokoro.model = "a".into();
    b.tts.kokoro.voices = "b|c".into();
    assert_ne!(
        capability_signature(&a, Capability::Tts),
        capability_signature(&b, Capability::Tts)
    );
}
