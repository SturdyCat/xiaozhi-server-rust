//! `server/src/app/tts_cache.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::*;

fn key(text: &str) -> u64 {
    TtsCache::make_key("sig", 0, 1.0, 24000, 60, text)
}

#[test]
fn cache_roundtrip() {
    let c = TtsCache::new(4);
    c.insert(key("你好"), vec![vec![1, 2, 3]]);
    let hit = c.get(key("你好")).expect("应命中");
    assert_eq!(hit.opus_frames.as_slice(), &[vec![1u8, 2, 3]]);
    assert!(c.get(key("再见")).is_none(), "未插入的键不得命中");
}

#[test]
fn key_varies_with_all_encoding_inputs() {
    let a = TtsCache::make_key("sig", 0, 1.0, 24000, 60, "文本");
    assert_ne!(
        a,
        TtsCache::make_key("sig2", 0, 1.0, 24000, 60, "文本"),
        "引擎签名"
    );
    assert_ne!(
        a,
        TtsCache::make_key("sig", 1, 1.0, 24000, 60, "文本"),
        "speaker"
    );
    assert_ne!(
        a,
        TtsCache::make_key("sig", 0, 1.1, 24000, 60, "文本"),
        "speed"
    );
    assert_ne!(
        a,
        TtsCache::make_key("sig", 0, 1.0, 16000, 60, "文本"),
        "下行采样率"
    );
    assert_ne!(
        a,
        TtsCache::make_key("sig", 0, 1.0, 24000, 40, "文本"),
        "帧长"
    );
    assert_ne!(
        a,
        TtsCache::make_key("sig", 0, 1.0, 24000, 60, "文本2"),
        "文本"
    );
    // 确定性：同输入同键（跨会话命中前提）
    assert_eq!(a, TtsCache::make_key("sig", 0, 1.0, 24000, 60, "文本"));
}

#[test]
fn lru_evicts_oldest_and_touches_on_hit() {
    let c = TtsCache::new(2);
    c.insert(key("a"), vec![vec![1]]);
    c.insert(key("b"), vec![vec![2]]);
    let _ = c.get(key("a")); // touch a → b 变最旧
    c.insert(key("c"), vec![vec![3]]); // 逐出 b
    assert!(c.get(key("a")).is_some());
    assert!(c.get(key("b")).is_none(), "最旧且未被 touch 的应被逐出");
    assert!(c.get(key("c")).is_some());
}

#[test]
fn zero_capacity_disables() {
    let c = TtsCache::new(0);
    c.insert(key("a"), vec![vec![1]]);
    assert!(c.get(key("a")).is_none(), "容量 0 = 禁用");
    assert_eq!(c.len(), 0);
}

#[test]
fn empty_frames_not_cached() {
    let c = TtsCache::new(4);
    c.insert(key("a"), vec![]);
    assert!(c.get(key("a")).is_none(), "空帧序列不值得占条目");
}

#[test]
fn overwrite_refreshes_same_key() {
    let c = TtsCache::new(2);
    c.insert(key("a"), vec![vec![1]]);
    c.insert(key("b"), vec![vec![2]]);
    c.insert(key("a"), vec![vec![9]]); // 覆盖 a，不新增条目、不逐出自己
    assert_eq!(c.len(), 2);
    let hit = c.get(key("a")).unwrap();
    assert_eq!(hit.opus_frames.as_slice(), &[vec![9u8]]);
}

fn persistent_key(text: &str) -> HotAudioKey {
    HotAudioKey::from_session(&crate::config::Config::default(), 24_000, 60, text)
}

#[tokio::test]
async fn persistent_cache_survives_restart_and_warms_memory() {
    let dir = std::env::temp_dir().join(format!("xz-tts-{}", uuid::Uuid::new_v4()));
    let k = persistent_key("普通聊天文本，不只是唤醒词");
    let store = Arc::new(HotAudioStore::new(Some(dir.clone())));
    let c = TtsCache::with_store(2, store.clone());
    c.register_identity(k.clone());
    c.insert_async(k.mem_key(), vec![vec![1, 2], vec![3]]).await;
    drop(c);
    let c = TtsCache::with_store(2, store);
    c.register_identity(k.clone());
    assert!(c.get(k.mem_key()).is_none());
    let hit = c.get_async(k.mem_key()).await.expect("重启后读盘");
    assert_eq!(*hit.opus_frames, vec![vec![1, 2], vec![3]]);
    assert!(c.get(k.mem_key()).is_some(), "暖进内存");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn lru_eviction_keeps_disk_entry_and_corruption_falls_back() {
    let dir = std::env::temp_dir().join(format!("xz-tts-{}", uuid::Uuid::new_v4()));
    let store = Arc::new(HotAudioStore::new(Some(dir.clone())));
    let c = TtsCache::with_store(1, store.clone());
    let a = persistent_key("A");
    let b = persistent_key("B");
    c.register_identity(a.clone());
    c.insert_async(a.mem_key(), vec![vec![1]]).await;
    c.register_identity(b.clone());
    c.insert_async(b.mem_key(), vec![vec![2]]).await;
    assert!(c.get(a.mem_key()).is_none());
    assert!(
        c.get_async(a.mem_key()).await.is_some(),
        "内存淘汰不删除磁盘"
    );
    c.clear();
    std::fs::write(dir.join(format!("{:016x}.hot", a.file_id())), b"broken").unwrap();
    assert!(c.get_async(a.mem_key()).await.is_none(), "损坏安全回退");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn disabled_cache_never_reads_or_writes_disk() {
    let dir = std::env::temp_dir().join(format!("xz-tts-{}", uuid::Uuid::new_v4()));
    let k = persistent_key("disabled");
    let c = TtsCache::with_store(0, Arc::new(HotAudioStore::new(Some(dir.clone()))));
    c.register_identity(k.clone());
    c.insert_async(k.mem_key(), vec![vec![1]]).await;
    assert!(c.get_async(k.mem_key()).await.is_none());
    assert!(!dir.exists());
}

#[tokio::test]
async fn unwritable_store_preserves_memory() {
    let dir = std::env::temp_dir().join(format!("xz-tts-{}", uuid::Uuid::new_v4()));
    std::fs::write(&dir, b"not a directory").unwrap();
    let c = TtsCache::with_store(1, Arc::new(HotAudioStore::new(Some(dir.clone()))));
    let k = persistent_key("A");
    c.register_identity(k.clone());
    c.insert_async(k.mem_key(), vec![vec![1]]).await;
    assert!(c.get(k.mem_key()).is_some());
    let _ = std::fs::remove_file(dir);
}

#[tokio::test]
async fn unregistered_or_zero_key_does_not_persist() {
    let dir = std::env::temp_dir().join(format!("xz-tts-{}", uuid::Uuid::new_v4()));
    let c = TtsCache::with_store(2, Arc::new(HotAudioStore::new(Some(dir.clone()))));
    c.insert_async(123, vec![vec![1]]).await;
    c.insert_async(0, vec![vec![1]]).await;
    assert!(c.get(123).is_some(), "仅内存调用保持兼容");
    assert!(c.get(0).is_none(), "零键明确禁用缓存");
    assert!(!dir.exists());
}

#[test]
fn identity_registry_and_entry_sizes_are_bounded() {
    let c = TtsCache::with_store(1, Arc::new(HotAudioStore::new(None)));
    for i in 0..1100 {
        c.register_identity(persistent_key(&format!("文本{i}")));
    }
    assert_eq!(c.inner.lock().unwrap().identities.len(), 1024);
    c.insert(1, vec![vec![1]; MAX_FRAMES + 1]);
    assert!(c.get(1).is_none());
    c.insert(1, vec![vec![1; MAX_ENTRY_BYTES + 1]]);
    assert!(c.get(1).is_none());
}
