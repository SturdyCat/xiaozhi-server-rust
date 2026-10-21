//! 两级 TTS 成品缓存：内存 LRU + [`HotAudioStore`] 持久化已编码下行 Opus 帧。
//!
//! 所有生产文本都可登记身份；`get_async` 先查内存，再在 `spawn_blocking` 读盘并暖内存。
//! 只有完整、成功且未被中断的句子可调用 `insert_async`，异步落盘结束后才返回。
//! 同步 `get` / `insert` 保留为纯内存 API（兼容既有 wakeup 暖缓存与单测）。
//! 命中仍经水库、背压和回复级节拍器发送，不改变流式合成或 ESP pacing。
//! 测试台强制真合成不登记身份；AIUI 云闭环返回的音频发生合成后才到服务端，
//! 本地缓存无法撤销云端已经发生的合成，调用方只缓存可复现的独立 TTS 文本。

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use crate::app::hot_audio::{
    HotAudioIdentity, HotAudioKey, HotAudioStore, MAX_ENTRY_BYTES, MAX_FRAMES,
};

/// 一句缓存音频：已编码的下行 Opus 帧负载序列（不含二进制协议头）。
#[derive(Debug, Clone)]
pub struct CachedAudio {
    pub opus_frames: Arc<Vec<Vec<u8>>>,
}

/// 进程级共享缓存；容量 0 禁用两级缓存。磁盘错误一律按 miss / 写入失败降级。
pub struct TtsCache {
    capacity: usize,
    inner: Mutex<Inner>,
    store: Option<Arc<HotAudioStore>>,
    /// 许可在进入 blocking 池前获取，避免无界堆积 blocking IO 任务。
    io_gate: Arc<tokio::sync::Semaphore>,
}

#[derive(Default)]
struct Inner {
    map: HashMap<u64, CachedAudio>,
    order: VecDeque<u64>,
    /// 只保存固定长度的哈希投影，不保留任意长原文或含凭据的引擎签名。
    identities: HashMap<u64, HotAudioIdentity>,
    identity_order: VecDeque<u64>,
}

impl TtsCache {
    /// 纯内存缓存（测试及无持久化需求的调用点）。
    pub fn new(capacity: u32) -> Self {
        Self {
            capacity: capacity as usize,
            inner: Mutex::new(Inner::default()),
            store: None,
            io_gate: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }

    pub fn with_store(capacity: u32, store: Arc<HotAudioStore>) -> Self {
        Self {
            store: Some(store),
            ..Self::new(capacity)
        }
    }

    /// 缓存键包含全部编码输入；不得记录原始引擎签名（含凭据）。
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

    /// 登记当句身份后，既有 u64 键即可用于读盘/写盘。登记表独立有界（至少容纳
    /// 1024 个近期身份），不随音频 LRU 淘汰，给正在预取/编码的句子留出窗口。
    pub fn register_identity(&self, key: HotAudioKey) {
        if self.capacity == 0 || self.store.is_none() {
            return;
        }
        let mem_key = key.mem_key();
        if mem_key == 0 {
            return;
        }
        let identity = HotAudioIdentity::from_key(&key);
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pos) = inner.identity_order.iter().position(|k| *k == mem_key) {
            inner.identity_order.remove(pos);
        } else if inner.identities.len() >= self.capacity.max(1024) {
            if let Some(oldest) = inner.identity_order.pop_front() {
                inner.identities.remove(&oldest);
            }
        }
        inner.identities.insert(mem_key, identity);
        inner.identity_order.push_back(mem_key);
    }

    /// 纯内存查找并 touch LRU；0 是流水线“不缓存”的保留键。
    pub fn get(&self, key: u64) -> Option<CachedAudio> {
        if self.capacity == 0 || key == 0 {
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

    /// 内存 miss 才读盘，所有磁盘 IO 在 blocking 池内；读失败不影响合成回退。
    pub async fn get_async(&self, key: u64) -> Option<CachedAudio> {
        // `get` 已对 `capacity == 0 || key == 0` 返回 `None`，内存 miss（含上述保留键）
        // 才会落到这里走磁盘；无需重复判定。
        if let Some(hit) = self.get(key) {
            return Some(hit);
        }
        let store = self.store.as_ref()?.clone();
        let identity = self.identity(key)?;
        let permit = self.io_gate.clone().acquire_owned().await.ok()?;
        if let Some(hit) = self.get(key) {
            return Some(hit);
        }
        let frames = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.get_identity(&identity)
        })
        .await
        .ok()??;
        let hit = CachedAudio {
            opus_frames: Arc::new(frames),
        };
        self.insert_memory(key, hit.clone());
        Some(hit)
    }

    /// 纯内存插入：不执行 IO（用于已有持久化命中后的暖缓存）。
    pub fn insert(&self, key: u64, frames: Vec<Vec<u8>>) {
        self.insert_memory(
            key,
            CachedAudio {
                opus_frames: Arc::new(frames),
            },
        );
    }

    /// 完整成功句的唯一生产写入入口。保留 Arc 帧序列，落盘不再复制整句；等待
    /// blocking 写入结束，不留 detached 任务。磁盘不可写仍保留内存成品。
    pub async fn insert_async(&self, key: u64, frames: Vec<Vec<u8>>) {
        let audio = CachedAudio {
            opus_frames: Arc::new(frames),
        };
        if !self.insert_memory(key, audio.clone()) {
            return;
        }
        let (Some(store), Some(identity)) = (self.store.as_ref().cloned(), self.identity(key))
        else {
            return;
        };
        let Ok(permit) = self.io_gate.clone().acquire_owned().await else {
            return;
        };
        if let Err(e) = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.put_identity(&identity, &audio.opus_frames)
        })
        .await
        {
            tracing::warn!("TTS 缓存写盘任务失败（保留内存缓存）: {e}");
        }
    }

    fn identity(&self, key: u64) -> Option<HotAudioIdentity> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .identities
            .get(&key)
            .cloned()
    }

    fn insert_memory(&self, key: u64, audio: CachedAudio) -> bool {
        if self.capacity == 0
            || key == 0
            || audio.opus_frames.is_empty()
            || audio.opus_frames.len() > MAX_FRAMES
            || audio.opus_frames.iter().map(Vec::len).sum::<usize>() > MAX_ENTRY_BYTES
        {
            return false;
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pos) = inner.order.iter().position(|k| *k == key) {
            inner.order.remove(pos);
        } else if inner.map.len() >= self.capacity {
            if let Some(oldest) = inner.order.pop_front() {
                inner.map.remove(&oldest);
            }
        }
        inner.map.insert(key, audio);
        inner.order.push_back(key);
        true
    }

    /// 测试专用的内存重置；生产热切换保留按实际身份分隔的成品与磁盘。
    #[cfg(test)]
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.map.clear();
        inner.order.clear();
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map
            .len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests;
