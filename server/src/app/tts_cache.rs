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
    /// 清空缓存（**必需**）：TTS 引擎热切换后，缓存里是**旧引擎**产出的下行音频
    /// （音色/采样率/语言可能都不同），继续命中会播出上一个引擎的声音。
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.map.clear();
        inner.order.clear();
    }

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
mod tests;
