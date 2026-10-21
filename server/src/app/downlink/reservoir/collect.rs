//! 水库填充时的**缓存收集器**：一句合成流过时顺带收集两份产物——
//! - 已编码下行帧：内存 TTS 缓存用（零合成零编码直投）；
//! - 原始 24k PCM S16LE：磁盘持久化用（`hot-audio/`，见 [`crate::app::hot_audio`]；
//!   下行参数只属于投递，磁盘存引擎产物才能跨设备档位复用）。
//!
//! PCM 收集走**独立的重采样器**（引擎采样率 → 24k，相等时透传），与下行分帧器
//! 互不干扰；两个上限（帧数/字节）任一超限即整体放弃收集——宁缺勿错，绝不影响
//! 实时下行（帧照发、额度照还）。

use crate::app::audio::resample::StreamingResampler;
use crate::app::hot_audio::{f32_to_pcm16_bytes, MAX_ENTRY_BYTES, MAX_FRAMES, PCM_SAMPLE_RATE};

/// 一句的缓存收集（生命周期 = 一句填充；`cache_key == 0` 的句子不建）。
pub(super) struct SentenceCollector {
    resampler: StreamingResampler,
    frames: Vec<Vec<u8>>,
    frame_bytes: usize,
    pcm: Vec<u8>,
    /// `false` = 任一上限超限，停止收集（已收内容整体丢弃）。
    ok: bool,
}

impl SentenceCollector {
    /// `tts_sr` = 引擎输出采样率（即流分片的采样率）。
    pub(super) fn new(tts_sr: u32) -> Self {
        Self {
            resampler: StreamingResampler::new(tts_sr, PCM_SAMPLE_RATE),
            frames: Vec::new(),
            frame_bytes: 0,
            pcm: Vec::new(),
            ok: true,
        }
    }

    /// 收一片引擎 PCM（须在下行动帧**之前**、分片 Drop 之前调用）。
    pub(super) fn on_chunk(&mut self, chunk: &[f32]) {
        if !self.ok {
            return;
        }
        self.pcm
            .extend_from_slice(&f32_to_pcm16_bytes(&self.resampler.push(chunk)));
        if self.pcm.len() > MAX_ENTRY_BYTES {
            self.give_up();
        }
    }

    /// 收一帧下行帧副本。
    pub(super) fn on_frame(&mut self, frame: &[u8]) {
        if !self.ok {
            return;
        }
        if self.frames.len() >= MAX_FRAMES
            || self.frame_bytes.saturating_add(frame.len()) > MAX_ENTRY_BYTES
        {
            self.give_up();
            return;
        }
        self.frame_bytes += frame.len();
        self.frames.push(frame.to_vec());
    }

    /// 完整句收尾：帧与 PCM 都非空才交出（半截句不做可复用成品）。
    pub(super) fn take(self) -> Option<(Vec<Vec<u8>>, Vec<u8>)> {
        if self.ok && !self.frames.is_empty() && !self.pcm.is_empty() {
            Some((self.frames, self.pcm))
        } else {
            None
        }
    }

    fn give_up(&mut self) {
        self.ok = false;
        self.frames.clear();
        self.frame_bytes = 0;
        self.pcm.clear();
    }
}
