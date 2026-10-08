//! TTS 结果内存缓存（LRU，按条目数）：把合成+重采样+Opus 编码后的**下行帧序列**
//! 按句缓存，命中时零合成延迟、直接经 [`crate::app::downlink::DownlinkPacer`]
//! 按 ESP 实时节奏下发——高频短句（"好的""我在""已打开"等）首字延迟从数百 ms
//! （甚至本地 Kokoro 的数秒）降到一次预充突发（~lead_ms）。
//!
//! ## 缓存层级的选择：缓存**编码后帧**而非 PCM
//! 缓存 PCM 还要每次重采样+编码（有 CPU 成本），且与下行参数耦合在解码端；
//! 直接缓存 Opus 帧负载（未加二进制协议头，发送时才 [`wrap_downlink`]）：
//! 命中路径零计算。因此缓存键必须包含影响编码结果的一切参数：
//! 引擎签名（backend/模型/音色凭据）、speaker、speed、下行采样率、帧长——
//! 任一变化键即不同，天然不会串用旧音频。
//!
//! 键经哈希混淆（std `DefaultHasher`）：引擎签名含讯飞密钥，**不可明文**
//! 进日志/内存键名。容量按条目数（`[tts].cache_entries`，0=关闭），超限逐最旧。

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

/// 一句缓存音频：已编码的下行 Opus 帧负载序列（**不含**二进制协议头）。
#[derive(Debug, Clone)]
pub struct CachedAudio {
    pub opus_frames: Arc<Vec<Vec<u8>>>,
}

/// 进程级 TTS 缓存（跨会话共享，放 [`crate::engine::Engines`]）。
pub struct TtsCache {
    /// 容量（条目数）；0 = 完全禁用（get 恒 None、insert 丢弃）。
    capacity: usize,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    map: HashMap<u64, CachedAudio>,
    /// 插入顺序（LRU 逐出用；get 命中移到队尾）。
    order: VecDeque<u64>,
}

impl TtsCache {
    pub fn new(capacity: u32) -> Self {
        Self {
            capacity: capacity as usize,
            inner: Mutex::new(Inner::default()),
        }
    }

    /// 缓存键（哈希混淆）：引擎签名 + speaker + speed + 下行采样率 + 帧长 + 文本。
    /// 引擎签名含凭据字段，哈希后不落明文（**不要**把返回值外的组成项打日志）。
    pub fn make_key(
        engine_signature: &str,
        speaker: i32,
        speed: f32,
        downlink_sr: u32,
        downlink_frame_ms: u32,
        text: &str,
    ) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        engine_signature.hash(&mut h);
        speaker.hash(&mut h);
        speed.to_bits().hash(&mut h);
        downlink_sr.hash(&mut h);
        downlink_frame_ms.hash(&mut h);
        text.hash(&mut h);
        h.finish()
    }

    /// 查缓存；命中时把该条目标记为最近使用（LRU touch）。
    pub fn get(&self, key: u64) -> Option<CachedAudio> {
        if self.capacity == 0 {
            return None;
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let hit = inner.map.get(&key).cloned()?;
        if let Some(pos) = inner.order.iter().position(|k| *k == key) {
            inner.order.remove(pos);
        }
        inner.order.push_back(key);
        Some(hit)
    }

    /// 插入（超容量逐最旧；容量 0 丢弃）。同键覆盖并保持原插入位次刷新。
    pub fn insert(&self, key: u64, frames: Vec<Vec<u8>>) {
        if self.capacity == 0 || frames.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.map.len() >= self.capacity && !inner.map.contains_key(&key) {
            if let Some(oldest) = inner.order.pop_front() {
                inner.map.remove(&oldest);
            }
        }
        if inner.map.insert(key, CachedAudio { opus_frames: Arc::new(frames) }).is_none() {
            inner.order.push_back(key);
        }
    }

    /// 当前条目数（观测/测试用）。
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).map.len()
    }

    /// 恒 false（`len` 只在观测时用，无空判场景；保留 API 对称性）。
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
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
}
