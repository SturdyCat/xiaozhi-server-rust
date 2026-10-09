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
    assert_ne!(a, TtsCache::make_key("sig2", 0, 1.0, 24000, 60, "文本"), "引擎签名");
    assert_ne!(a, TtsCache::make_key("sig", 1, 1.0, 24000, 60, "文本"), "speaker");
    assert_ne!(a, TtsCache::make_key("sig", 0, 1.1, 24000, 60, "文本"), "speed");
    assert_ne!(a, TtsCache::make_key("sig", 0, 1.0, 16000, 60, "文本"), "下行采样率");
    assert_ne!(a, TtsCache::make_key("sig", 0, 1.0, 24000, 40, "文本"), "帧长");
    assert_ne!(a, TtsCache::make_key("sig", 0, 1.0, 24000, 60, "文本2"), "文本");
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
