//! 两级 TTS 成品缓存：内存 LRU（已编码下行帧）+ [`HotAudioStore`] 持久化（**引擎原样
//! PCM，速率记在身份键上**，见 [`crate::media::hot_audio`]）。
//!
//! 两级**键口径刻意不同**：内存键含下行参数（帧按会话格式编码，格式变了帧就不同）；
//! 磁盘身份只含引擎/音色/语速/文本/源采样率（PCM 是合成产物，与投递格式无关）。因此
//! `get_async` 需要会话的下行格式：内存命中直接给帧；磁盘命中读 PCM → **按源真实率**
//! 重采样 + 编码（与流式合成同一分帧链路）→ 暖进内存后交付。
//!
//! 所有生产文本都可登记身份；`get_async` 先查内存，再在 `spawn_blocking` 读盘
//! （含编码）并暖内存。只有完整、成功且未被中断的句子可调用 `insert_async`，异步
//! 落盘结束后才返回；`pcm` 为 `None` 时只暖内存不写盘。同步 `get` / `insert` 保留为
//! 纯内存 API（兼容既有 wakeup 暖缓存与单测）。命中仍经水库、背压和回复级节拍器
//! 发送，不改变流式合成或 ESP pacing。测试台强制真合成不登记身份；AIUI 云闭环返回
//! 的音频发生合成后才到服务端，本地缓存无法撤销云端已经发生的合成，调用方只缓存
//! 可复现的独立 TTS 文本。

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use crate::media::hot_audio::{
    pcm_bytes_to_f32, HotAudioKey, HotAudioStore, MAX_ENTRY_BYTES, MAX_FRAMES,
};
use crate::media::media::AudioFormat;
use crate::workflow::downlink::framer::StreamingFramer;

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
    /// 只保存磁盘身份键（键内不含凭据明文的投影；原始签名不保留）。
    identities: HashMap<u64, HotAudioKey>,
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

    /// 缓存键包含**全部**编码输入（含编解码档位与契约版本——bench 回声档与设备档
    /// 可能几何参数相同而 codec 不同，帧字节不可互喂）；不得记录原始引擎签名（含凭据）。
    pub fn make_key(
        engine_signature: &str,
        speaker: i32,
        speed: f32,
        tts_sr: u32,
        format: AudioFormat,
        text: &str,
    ) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        engine_signature.hash(&mut h);
        speaker.hash(&mut h);
        speed.to_bits().hash(&mut h);
        // 源采样率决定重采样比，进而决定编码出的帧字节，必须进键
        tts_sr.hash(&mut h);
        format.codec.hash(&mut h);
        format.codec_version.hash(&mut h);
        format.sample_rate.hash(&mut h);
        format.channels.hash(&mut h);
        format.frame_ms.hash(&mut h);
        text.hash(&mut h);
        h.finish()
    }

    /// 登记当句身份后，既有 u64 键即可用于读盘/写盘。登记表独立有界（至少容纳
    /// 1024 个近期身份），不随音频 LRU 淘汰，给正在预取/编码的句子留出窗口。
    pub fn register_identity(&self, key: HotAudioKey, mem_key: u64) {
        if self.capacity == 0 || self.store.is_none() {
            return;
        }
        if mem_key == 0 {
            return;
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pos) = inner.identity_order.iter().position(|k| *k == mem_key) {
            inner.identity_order.remove(pos);
        } else if inner.identities.len() >= self.capacity.max(1024) {
            if let Some(oldest) = inner.identity_order.pop_front() {
                inner.identities.remove(&oldest);
            }
        }
        inner.identities.insert(mem_key, key);
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

    /// 内存 miss 才读盘，磁盘 IO 与 PCM→帧编码都在 blocking 池内；任何失败都按
    /// miss 处理（回退真合成），不影响合成回退路径。
    pub async fn get_async(&self, key: u64, format: AudioFormat) -> Option<CachedAudio> {
        // `get` 已对 `capacity == 0 || key == 0` 返回 `None`，内存 miss（含上述保留键）
        // 才会落到这里走磁盘；无需重复判定。
        if let Some(hit) = self.get(key) {
            return Some(hit);
        }
        let store = self.store.as_ref()?.clone();
        let hot_key = self.identity(key)?;
        let permit = self.io_gate.clone().acquire_owned().await.ok()?;
        if let Some(hit) = self.get(key) {
            return Some(hit);
        }
        let frames = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            // 磁盘 miss（None）与编码失败（Err）都按 miss 处理；读盘按**源真实率**还原
            store
                .get(&hot_key)
                .map(|pcm| encode_pcm(pcm, hot_key.tts_sr, format))
        })
        .await
        .ok()
        .flatten()
        .and_then(|r| r.ok())?;
        let hit = CachedAudio {
            opus_frames: Arc::new(frames),
        };
        self.insert_memory(key, hit.clone());
        Some(hit)
    }

    /// 纯内存插入：不执行 IO（测试与既有暖缓存辅助；生产磁盘命中经 `get_async` 暖入）。
    #[cfg(test)]
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
    ///
    /// `pcm` = 该句的引擎原样 S16LE 字节（水库在编码下行帧时顺带收集，未做任何
    /// 重采样；其采样率随身份键记录）；`None` 或超限时只暖内存、不写盘（磁盘身份
    /// 没有帧序号可重建，宁缺勿错）。
    pub async fn insert_async(&self, key: u64, frames: Vec<Vec<u8>>, pcm: Option<Vec<u8>>) {
        let audio = CachedAudio {
            opus_frames: Arc::new(frames),
        };
        if !self.insert_memory(key, audio.clone()) {
            return;
        }
        let (Some(store), Some(hot_key)) = (self.store.as_ref().cloned(), self.identity(key))
        else {
            return;
        };
        let Some(pcm) = pcm.filter(|p| !p.is_empty() && p.len() <= MAX_ENTRY_BYTES) else {
            return;
        };
        let Ok(permit) = self.io_gate.clone().acquire_owned().await else {
            return;
        };
        if let Err(e) = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.put(&hot_key, &pcm)
        })
        .await
        {
            tracing::warn!("TTS 缓存写盘任务失败（保留内存缓存）: {e}");
        }
    }

    fn identity(&self, key: u64) -> Option<HotAudioKey> {
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

/// 磁盘 PCM（引擎原样，`from_sr` 为其真实率）→ 当前会话格式的下行帧：与流式合成
/// **完全同一分帧链路**（重采样 + Opus 编码，`framer` 内跨片复用同一编解码器），
/// 保证缓存命中句与真合成句听感口径一致。
fn encode_pcm(pcm: Vec<u8>, from_sr: u32, format: AudioFormat) -> anyhow::Result<Vec<Vec<u8>>> {
    let samples = pcm_bytes_to_f32(&pcm);
    let mut framer = StreamingFramer::new_format(from_sr, format)?;
    let mut frames = framer.push(&samples)?;
    if let Some(tail) = framer.finish()? {
        frames.push(tail);
    }
    Ok(frames)
}

#[cfg(test)]
mod tests;
