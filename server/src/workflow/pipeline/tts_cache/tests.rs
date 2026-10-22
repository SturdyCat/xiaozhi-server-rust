//! `server/src/app/tts_cache.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::*;
use crate::media::media::{AudioCodec, AudioFormat};

/// 测试用源采样率（磁盘 PCM 的真实率）。
const SRC_SR: u32 = 16_000;

/// 测试用下行档位（16k / 60ms Opus）。
const KEY_FMT: AudioFormat = AudioFormat::new(AudioCodec::Opus, 24_000, 60);

fn key(text: &str) -> u64 {
    TtsCache::make_key("sig", 0, 1.0, SRC_SR, KEY_FMT, text)
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
    let a = TtsCache::make_key("sig", 0, 1.0, SRC_SR, KEY_FMT, "文本");
    assert_ne!(
        a,
        TtsCache::make_key("sig2", 0, 1.0, SRC_SR, KEY_FMT, "文本"),
        "引擎签名"
    );
    assert_ne!(
        a,
        TtsCache::make_key("sig", 1, 1.0, SRC_SR, KEY_FMT, "文本"),
        "speaker"
    );
    assert_ne!(
        a,
        TtsCache::make_key("sig", 0, 1.1, SRC_SR, KEY_FMT, "文本"),
        "speed"
    );
    // 源采样率决定重采样比：同一句 16k 与 24k 还原出的帧字节不同，键必须分开
    assert_ne!(
        a,
        TtsCache::make_key("sig", 0, 1.0, 24_000, KEY_FMT, "文本"),
        "源采样率"
    );
    let mut f = KEY_FMT;
    f.sample_rate = 16_000;
    assert_ne!(
        a,
        TtsCache::make_key("sig", 0, 1.0, SRC_SR, f, "文本"),
        "下行采样率"
    );
    f = KEY_FMT;
    f.frame_ms = 40;
    assert_ne!(a, TtsCache::make_key("sig", 0, 1.0, SRC_SR, f, "文本"), "帧长");
    f = KEY_FMT;
    f.codec = AudioCodec::PcmS16Le;
    assert_ne!(
        a,
        TtsCache::make_key("sig", 0, 1.0, SRC_SR, f, "文本"),
        "编解码档位（bench 回声档与设备档可能几何相同）"
    );
    f = KEY_FMT;
    f.codec_version += 1;
    assert_ne!(a, TtsCache::make_key("sig", 0, 1.0, SRC_SR, f, "文本"), "契约版本");
    assert_ne!(
        a,
        TtsCache::make_key("sig", 0, 1.0, SRC_SR, KEY_FMT, "文本2"),
        "文本"
    );
    // 确定性：同输入同键（跨会话命中前提）
    assert_eq!(a, TtsCache::make_key("sig", 0, 1.0, SRC_SR, KEY_FMT, "文本"));
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
    HotAudioKey::from_session(&crate::config::Config::default(), text, SRC_SR)
}

/// 测试用下行格式（16k / 60ms Opus）：磁盘身份不含它，内存键含它。
const OUT: AudioFormat = AudioFormat::new(AudioCodec::Opus, 16_000, 60);

/// 测试用 PCM：`n` 毫秒 `SRC_SR` 静音（S16LE，偶数字节）。
fn pcm_ms(ms: usize) -> Vec<u8> {
    vec![0u8; ms * SRC_SR as usize / 1000 * 2]
}

#[tokio::test]
async fn persistent_cache_survives_restart_and_warms_memory() {
    let dir = std::env::temp_dir().join(format!("xz-tts-{}", uuid::Uuid::new_v4()));
    let k = persistent_key("普通聊天文本，不只是唤醒词");
    let mem = k.mem_key(OUT);
    let store = Arc::new(HotAudioStore::new(Some(dir.clone())));
    let c = TtsCache::with_store(2, store.clone());
    c.register_identity(k.clone(), mem);
    c.insert_async(mem, vec![vec![1, 2], vec![3]], Some(pcm_ms(150)))
        .await;
    drop(c);
    let c = TtsCache::with_store(2, store);
    c.register_identity(k.clone(), mem);
    assert!(c.get(mem).is_none());
    let hit = c.get_async(mem, OUT).await.expect("重启后读盘");
    // 读盘命中是 PCM → 按会话格式重新编码：帧数由音频时长决定（150ms@16k/60ms ≈ 3 帧），
    // 不再等于写入时的帧序。
    assert_eq!(hit.opus_frames.len(), 3);
    assert!(c.get(mem).is_some(), "暖进内存");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn lru_eviction_keeps_disk_entry_and_corruption_falls_back() {
    let dir = std::env::temp_dir().join(format!("xz-tts-{}", uuid::Uuid::new_v4()));
    let store = Arc::new(HotAudioStore::new(Some(dir.clone())));
    let c = TtsCache::with_store(1, store.clone());
    let a = persistent_key("A");
    let b = persistent_key("B");
    let (ma, mb) = (a.mem_key(OUT), b.mem_key(OUT));
    c.register_identity(a.clone(), ma);
    c.insert_async(ma, vec![vec![1]], Some(pcm_ms(70))).await;
    c.register_identity(b.clone(), mb);
    c.insert_async(mb, vec![vec![2]], Some(pcm_ms(70))).await;
    assert!(c.get(ma).is_none());
    assert!(c.get_async(ma, OUT).await.is_some(), "内存淘汰不删除磁盘");
    c.clear();
    // 裸 .pcm 无头部/校验和：**结构性**损坏（奇数字节 = 非整 S16LE 样本）读取即拒绝
    // 并删除；内容级损坏不可判定，完整性由 tmp+rename 原子写保证（见 hot_audio 文档）。
    std::fs::write(dir.join(format!("{:016x}.pcm", a.file_id())), b"broken!").unwrap();
    assert!(c.get_async(ma, OUT).await.is_none(), "结构损坏安全回退");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn disabled_cache_never_reads_or_writes_disk() {
    let dir = std::env::temp_dir().join(format!("xz-tts-{}", uuid::Uuid::new_v4()));
    let k = persistent_key("disabled");
    let c = TtsCache::with_store(0, Arc::new(HotAudioStore::new(Some(dir.clone()))));
    let mem = k.mem_key(OUT);
    c.register_identity(k.clone(), mem);
    c.insert_async(mem, vec![vec![1]], Some(pcm_ms(70))).await;
    assert!(c.get_async(mem, OUT).await.is_none());
    assert!(!dir.exists());
}

#[tokio::test]
async fn unwritable_store_preserves_memory() {
    let dir = std::env::temp_dir().join(format!("xz-tts-{}", uuid::Uuid::new_v4()));
    std::fs::write(&dir, b"not a directory").unwrap();
    let c = TtsCache::with_store(1, Arc::new(HotAudioStore::new(Some(dir.clone()))));
    let k = persistent_key("A");
    let mem = k.mem_key(OUT);
    c.register_identity(k.clone(), mem);
    c.insert_async(mem, vec![vec![1]], Some(pcm_ms(70))).await;
    assert!(c.get(mem).is_some());
    let _ = std::fs::remove_file(dir);
}

#[tokio::test]
async fn unregistered_or_zero_key_does_not_persist() {
    let dir = std::env::temp_dir().join(format!("xz-tts-{}", uuid::Uuid::new_v4()));
    let c = TtsCache::with_store(2, Arc::new(HotAudioStore::new(Some(dir.clone()))));
    c.insert_async(123, vec![vec![1]], Some(pcm_ms(70))).await;
    c.insert_async(0, vec![vec![1]], Some(pcm_ms(70))).await;
    assert!(c.get(123).is_some(), "仅内存调用保持兼容");
    assert!(c.get(0).is_none(), "零键明确禁用缓存");
    assert!(!dir.exists());
}

#[tokio::test]
async fn pcm_overflow_writes_memory_only() {
    let dir = std::env::temp_dir().join(format!("xz-tts-{}", uuid::Uuid::new_v4()));
    let k = persistent_key("超长句");
    let mem = k.mem_key(OUT);
    let c = TtsCache::with_store(2, Arc::new(HotAudioStore::new(Some(dir.clone()))));
    c.register_identity(k.clone(), mem);
    c.insert_async(mem, vec![vec![1]], Some(vec![0u8; MAX_ENTRY_BYTES + 1]))
        .await;
    assert!(c.get(mem).is_some(), "内存成品保留");
    assert!(
        !dir.join(format!("{:016x}.pcm", k.file_id())).exists(),
        "超限不写盘"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn identity_registry_and_entry_sizes_are_bounded() {
    let c = TtsCache::with_store(1, Arc::new(HotAudioStore::new(None)));
    for i in 0..1100 {
        let k = persistent_key(&format!("文本{i}"));
        c.register_identity(k.clone(), k.mem_key(OUT));
    }
    assert_eq!(c.inner.lock().unwrap().identities.len(), 1024);
    c.insert(1, vec![vec![1]; MAX_FRAMES + 1]);
    assert!(c.get(1).is_none());
    c.insert(1, vec![vec![1; MAX_ENTRY_BYTES + 1]]);
    assert!(c.get(1).is_none());
}
