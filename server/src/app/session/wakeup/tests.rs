//! `server/src/app/session/wakeup.rs` 的单元测试。
//!
//! 只测**音频来源决策**（[`resolve_greeting_audio`]）：`Session` 需要真实 VAD 模型才能构造
//!（`engine.rs` 的必需能力），所以决策被刻意写成不依赖 `Session` 的自由函数——与
//! `session/mcp.rs` 的等待泵同一手法。
//!
//! 未被自动化覆盖的只剩三行接线（`handle_text` 的 `detect` 分支 → `handle_wake_greeting`）
//! 与真机端到端（需要烧录固件 + 唤醒词），这在 CI 里做不到。

use std::path::PathBuf;

use super::*;
use crate::app::audio::media::{AudioCodec, AudioFormat};
use crate::config::Config;

fn tmp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("xz-wake-{}", uuid::Uuid::new_v4()))
}

/// 测试用源采样率：磁盘 PCM 的原样率。
const SRC_SR: u32 = 24_000;

fn key_of(text: &str) -> HotAudioKey {
    HotAudioKey::from_session(&Config::default(), text, SRC_SR)
}

/// 测试用下行格式（16k / 60ms Opus）：内存键含它，磁盘身份不含它。
const OUT: AudioFormat = AudioFormat::new(AudioCodec::Opus, 16_000, 60);

fn frames(n: usize) -> Vec<Vec<u8>> {
    (0..n).map(|i| vec![i as u8; 8]).collect()
}

/// 测试用 PCM：`ms` 毫秒 `SRC_SR` 静音（S16LE，偶数字节）。
fn pcm(ms: usize) -> Vec<u8> {
    vec![0u8; ms * SRC_SR as usize / 1000 * 2]
}

#[test]
fn 都不命中时要求合成() {
    let dir = tmp_dir();
    let cache = TtsCache::new(8);
    let store = HotAudioStore::new(Some(dir.clone()));
    assert_eq!(
        resolve_greeting_audio(&cache, &store, &key_of("我在"), OUT, true),
        GreetingAudio::NeedsSynth
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 内存命中直接就绪() {
    let dir = tmp_dir();
    let cache = TtsCache::new(8);
    let store = HotAudioStore::new(Some(dir.clone()));
    let k = key_of("我在");
    cache.insert(k.mem_key(OUT), frames(3));

    assert_eq!(
        resolve_greeting_audio(&cache, &store, &k, OUT, true),
        GreetingAudio::Ready("内存缓存")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 磁盘命中_暖缓存交给预取路径() {
    let dir = tmp_dir();
    let cache = TtsCache::new(8);
    let store = HotAudioStore::new(Some(dir.clone()));
    let k = key_of("我在");
    // 模拟"上一个进程留下的落盘音频"（24k PCM）
    assert!(store.put(&k, &pcm(80)));

    // 首次：磁盘命中（本函数只做决策；把 PCM 编码成帧、暖进内存是
    // `speak_canned` 预取路径 `TtsCache::get_async` 的职责，那里有会话格式）
    assert_eq!(
        resolve_greeting_audio(&cache, &store, &k, OUT, true),
        GreetingAudio::Ready("磁盘缓存")
    );
    assert!(
        cache.get(k.mem_key(OUT)).is_none(),
        "决策函数不再暖缓存（编码需要会话格式，职责在预取路径，见 tts_cache 测试）"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 关闭落盘时磁盘不参与() {
    let dir = tmp_dir();
    let cache = TtsCache::new(8);
    let store = HotAudioStore::new(Some(dir.clone()));
    let k = key_of("我在");
    assert!(store.put(&k, &pcm(80)));

    // persist=false：即使盘上有音频也不该读（用户显式关掉了复用）
    assert_eq!(
        resolve_greeting_audio(&cache, &store, &k, OUT, false),
        GreetingAudio::NeedsSynth
    );
    assert!(cache.get(k.mem_key(OUT)).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 换音色不串用旧音频() {
    let dir = tmp_dir();
    let cache = TtsCache::new(8);
    let store = HotAudioStore::new(Some(dir.clone()));
    let mut a = key_of("我在");
    assert!(store.put(&a, &pcm(80)));

    // 换了音色：键变 → 必须重新合成（需求里的"切换音色，重新获取"）
    a.speaker += 1;
    assert_eq!(
        resolve_greeting_audio(&cache, &store, &a, OUT, true),
        GreetingAudio::NeedsSynth
    );

    // 换语速同理
    let mut b = key_of("我在");
    b.speed = 1.4;
    assert_eq!(
        resolve_greeting_audio(&cache, &store, &b, OUT, true),
        GreetingAudio::NeedsSynth
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 未配置目录时纯内存复用() {
    // 没加载配置文件 → 无落盘目录：仍可内存复用，只是重启后要重新合成。
    let cache = TtsCache::new(8);
    let store = HotAudioStore::new(None);
    let k = key_of("我在");
    assert_eq!(
        resolve_greeting_audio(&cache, &store, &k, OUT, true),
        GreetingAudio::NeedsSynth
    );
    cache.insert(k.mem_key(OUT), frames(2));
    assert_eq!(
        resolve_greeting_audio(&cache, &store, &k, OUT, true),
        GreetingAudio::Ready("内存缓存")
    );
}

#[test]
fn 缓存容量为零时不会误报就绪() {
    // `[tts].cache_entries = 0`：insert 丢弃、get 恒 None。此时决策必须仍是 NeedsSynth
    //（不能"以为暖上了"却拿到空帧）——配置校验会拦下 persist+0 的组合，这里守住运行时口径。
    let cache = TtsCache::new(0);
    let store = HotAudioStore::new(None);
    let k = key_of("我在");
    cache.insert(k.mem_key(OUT), frames(2));
    assert!(cache.get(k.mem_key(OUT)).is_none());
    assert_eq!(
        resolve_greeting_audio(&cache, &store, &k, OUT, true),
        GreetingAudio::NeedsSynth
    );
}
