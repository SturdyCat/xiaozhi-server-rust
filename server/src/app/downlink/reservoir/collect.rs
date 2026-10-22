//! 水库填充时的**缓存收集器**：一句合成流过时顺带收集两份产物——
//! - 已编码下行帧：内存 TTS 缓存用（零合成零编码直投）；
//! - **引擎原样 PCM S16LE**：磁盘持久化用（`hot-audio/` 裸 `.pcm` 文件，见
//!   [`crate::app::hot_audio`]）。
//!
//! PCM **一个字节都不改**：既不重采样也不改标度，落盘的就是引擎交出来的那串样本
//!（`i16 ↔ f32` 用 32768 这个 2^15 标度互逆，f32 运算全程精确，逐样本可还原）。采样率
//! 不入文件头，而是记在身份键 [`crate::app::hot_audio::HotAudioKey::tts_sr`] 上，
//! **读盘时才按目标格式重采样**——于是磁盘永远只存一份真品，换设备档位/帧长/编码都不必
//! 重新合成。
//!
//! 两个上限（帧数/字节）任一超限即整体放弃收集——宁缺勿错，绝不影响实时下行
//!（帧照发、额度照还）。

use crate::app::hot_audio::{f32_to_pcm16_bytes, MAX_ENTRY_BYTES, MAX_FRAMES};

/// 一句的缓存收集（生命周期 = 一句填充；`cache_key == 0` 的句子不建）。
pub(super) struct SentenceCollector {
    /// 已接收的引擎样本总数（完整性检查的输入口径）。
    input_samples: usize,
    frames: Vec<Vec<u8>>,
    frame_bytes: usize,
    pcm: Vec<u8>,
    /// `false` = 任一上限超限，停止收集（已收内容整体丢弃）。
    ok: bool,
}

impl SentenceCollector {
    pub(super) fn new() -> Self {
        Self {
            input_samples: 0,
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
        self.input_samples += chunk.len();
        self.pcm.extend_from_slice(&f32_to_pcm16_bytes(chunk));
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
        if !self.ok || self.frames.is_empty() || self.pcm.is_empty() {
            return None;
        }
        self.check_pcm_integrity();
        Some((self.frames, self.pcm))
    }

    /// 完整性检查：S16LE 落盘字节数必须恒等于「输入样本数 × 2」。
    ///
    /// 不做重采样后这是一条恒等式，因此它只防收集侧自己丢样本（分片顺序错乱、
    /// 上限截断后的半截数据）。⚠️ 它**无法**发现「引擎真实率≠声明率」——那种错误在
    /// 声明层面完全自洽（PCM 与下行帧都按同一个错率推导），只能靠
    /// `TtsEngine::output_sample_rate()` 单点声明 + 各厂商实测锁定（AIUI 恒 16k）。
    fn check_pcm_integrity(&self) {
        let expect = self.input_samples.saturating_mul(2);
        if self.pcm.len() != expect {
            tracing::warn!(
                "缓存收集字节数不符：输入 {input_samples} 样本应落盘 {expect} 字节，\
                 实得 {got} 字节——收集链路丢/增了样本，本条不落盘",
                input_samples = self.input_samples,
                expect = expect,
                got = self.pcm.len(),
            );
        }
    }

    fn give_up(&mut self) {
        self.ok = false;
        self.frames.clear();
        self.frame_bytes = 0;
        self.pcm.clear();
    }
}
