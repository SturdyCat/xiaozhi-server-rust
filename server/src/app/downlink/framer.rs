//! 流式分帧器：PCM 分片 → 重采样 → Opus 逐帧编码。
//!
//! 从 `downlink.rs` 的流式下发函数里抽出（AGENTS.md §5.10 文件规模；同时被
//! 「水库填充任务」与「测试台直通路径」复用，避免两份实现漂移）。
//!
//! ## 为什么必须**跨分片复用**同一个重采样器与编码器
//! 每个分片单独新建重采样器 → 分片边界相位跳变（听感爆音）；单独新建 Opus 编码器 →
//! 每个分片都是一次"流重启"（状态清零），同样出杂音/断续。故本结构体在一次
//! 合成（一句）内持有两者，并把不足整帧的尾部样本 `carry` 跨分片积攒。
//!
//! 尾帧：残样本补零到整帧（Opus 只接受合法帧长，否则 `BadArgument`）。

use crate::app::audio::opus::OpusFrameEncoder;
use crate::app::audio::resample::StreamingResampler;
use crate::app::downlink::pad_tail;

/// 一句音频的分帧器（生命周期 = 一句合成）。
pub(crate) struct StreamingFramer {
    resampler: StreamingResampler,
    encoder: OpusFrameEncoder,
    /// 每帧样本数 = 下行采样率 × 帧长（毫秒）。
    frame: usize,
    /// 不足整帧的尾部样本（跨分片积攒）。
    carry: Vec<f32>,
}

impl StreamingFramer {
    /// 按 `tts_sr`（引擎输出）→ `downlink_sr` 建立重采样与编码链。
    pub(crate) fn new(tts_sr: u32, downlink_sr: u32, frame_ms: u32) -> anyhow::Result<Self> {
        let frame = ((downlink_sr as f32 * frame_ms.max(1) as f32 / 1000.0) as usize).max(1);
        Ok(Self {
            resampler: StreamingResampler::new(tts_sr, downlink_sr),
            encoder: OpusFrameEncoder::new(downlink_sr)?,
            frame,
            carry: Vec::new(),
        })
    }

    /// 每帧样本数（单测断言切分口径用；生产路径不需要，故仅测试编译）。
    #[cfg(test)]
    pub(crate) fn frame_samples(&self) -> usize {
        self.frame
    }

    /// 推入一个 PCM 分片，返回其中**凑满**的完整帧（按序）。
    ///
    /// 编码失败向上抛错（调用方按"整句作废"处理：不再继续该句、也不写结果缓存）——
    /// 与旧实现在 `send_sentence_audio_stream` 里的语义一致。
    pub(crate) fn push(&mut self, chunk: &[f32]) -> anyhow::Result<Vec<Vec<u8>>> {
        let down = self.resampler.push(chunk);
        self.carry.extend_from_slice(&down);
        let mut out: Vec<Vec<u8>> = Vec::new();
        while self.carry.len() >= self.frame {
            let opus = self.encoder.encode_frame(&self.carry[..self.frame])?;
            self.carry.drain(..self.frame);
            out.push(opus);
        }
        Ok(out)
    }

    /// 收尾：残样本补零成整帧并编码；无残样本返回 `Ok(None)`。
    pub(crate) fn finish(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        if self.carry.is_empty() {
            return Ok(None);
        }
        let tail = pad_tail(&self.carry, self.frame);
        self.carry.clear();
        Ok(Some(self.encoder.encode_frame(&tail)?))
    }
}

#[cfg(test)]
mod tests;
