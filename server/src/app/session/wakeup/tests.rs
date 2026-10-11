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
use crate::config::Config;

fn tmp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("xz-wake-{}", uuid::Uuid::new_v4()))
}

fn key_of(text: &str) -> HotAudioKey {
    HotAudioKey::from_session(&Config::default(), 24_000, 60, text)
}

fn frames(n: usize) -> Vec<Vec<u8>> {
    (0..n).map(|i| vec![i as u8; 8]).collect()
}

#[test]
fn 都不命中时要求合成() {
    let dir = tmp_dir();
    let cache = TtsCache::new(8);
    let store = HotAudioStore::new(Some(dir.clone()));
    assert_eq!(
        resolve_greeting_audio(&cache, &store, &key_of("我在"), true),
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
    cache.insert(k.mem_key(), frames(3));

    assert_eq!(
        resolve_greeting_audio(&cache, &store, &k, true),
        GreetingAudio::Ready("内存缓存")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 磁盘命中并暖进内存_第二次走内存() {
    let dir = tmp_dir();
    let cache = TtsCache::new(8);
    let store = HotAudioStore::new(Some(dir.clone()));
    let k = key_of("我在");
    // 模拟"上一个进程留下的落盘音频"
    assert!(store.put(&k, &frames(4)));

    // 首次：磁盘命中
    assert_eq!(
        resolve_greeting_audio(&cache, &store, &k, true),
        GreetingAudio::Ready("磁盘缓存")
    );
    // 关键断言：命中磁盘时必须**暖进内存**——否则每次唤醒都要读盘，
    // 且 `speak_canned` 会走真合成（那就完全没省下 TTS）。
    assert!(cache.get(k.mem_key()).is_some(), "磁盘命中必须暖进内存缓存");

    // 再问一次：现在应该走内存（证明暖缓存生效）
    assert_eq!(
        resolve_greeting_audio(&cache, &store, &k, true),
        GreetingAudio::Ready("内存缓存")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 关闭落盘时磁盘不参与() {
    let dir = tmp_dir();
    let cache = TtsCache::new(8);
    let store = HotAudioStore::new(Some(dir.clone()));
    let k = key_of("我在");
    assert!(store.put(&k, &frames(4)));

    // persist=false：即使盘上有音频也不该读（用户显式关掉了复用）
    assert_eq!(
        resolve_greeting_audio(&cache, &store, &k, false),
        GreetingAudio::NeedsSynth
    );
    assert!(cache.get(k.mem_key()).is_none(), "不该暖缓存");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 换音色不串用旧音频() {
    let dir = tmp_dir();
    let cache = TtsCache::new(8);
    let store = HotAudioStore::new(Some(dir.clone()));
    let mut a = key_of("我在");
    assert!(store.put(&a, &frames(4)));

    // 换了音色：键变 → 必须重新合成（需求里的"切换音色，重新获取"）
    a.speaker += 1;
    assert_eq!(
        resolve_greeting_audio(&cache, &store, &a, true),
        GreetingAudio::NeedsSynth
    );

    // 换语速同理
    let mut b = key_of("我在");
    b.speed = 1.4;
    assert_eq!(
        resolve_greeting_audio(&cache, &store, &b, true),
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
        resolve_greeting_audio(&cache, &store, &k, true),
        GreetingAudio::NeedsSynth
    );
    cache.insert(k.mem_key(), frames(2));
    assert_eq!(
        resolve_greeting_audio(&cache, &store, &k, true),
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
    cache.insert(k.mem_key(), frames(2));
    assert!(cache.get(k.mem_key()).is_none());
    assert_eq!(
        resolve_greeting_audio(&cache, &store, &k, true),
        GreetingAudio::NeedsSynth
    );
}
