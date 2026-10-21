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
    encoder: Option<OpusFrameEncoder>,
    /// 每帧样本数 = 下行采样率 × 帧长（毫秒）。
    frame: usize,
    /// 不足整帧的尾部样本（跨分片积攒）。
    carry: Vec<f32>,
}

impl StreamingFramer {
    /// 按 `tts_sr`（引擎输出）→ `downlink_sr` 建立重采样与编码链。
    #[cfg(test)]
    pub(crate) fn new(tts_sr: u32, downlink_sr: u32, frame_ms: u32) -> anyhow::Result<Self> {
        Self::new_format(
            tts_sr,
            crate::app::audio::media::AudioFormat::new(
                crate::app::audio::media::AudioCodec::Opus,
                downlink_sr,
                frame_ms,
            ),
        )
    }

    pub(crate) fn new_format(
        tts_sr: u32,
        format: crate::app::audio::media::AudioFormat,
    ) -> anyhow::Result<Self> {
        use crate::app::audio::media::AudioCodec;
        anyhow::ensure!(
            tts_sr > 0
                && format.sample_rate > 0
                && format.frame_ms > 0
                && format.channels == 1
                && format.codec_version == 1,
            "unsupported audio format"
        );
        let frame = ((u64::from(format.sample_rate) * u64::from(format.frame_ms.max(1)) / 1000)
            as usize)
            .max(1);
        Ok(Self {
            resampler: StreamingResampler::new(tts_sr, format.sample_rate),
            encoder: match format.codec {
                AudioCodec::Opus => Some(OpusFrameEncoder::new(format.sample_rate)?),
                AudioCodec::PcmS16Le => None,
            },
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
            let opus = encode(&mut self.encoder, &self.carry[..self.frame])?;
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
        Ok(Some(encode(&mut self.encoder, &tail)?))
    }
}

fn encode(encoder: &mut Option<OpusFrameEncoder>, pcm: &[f32]) -> anyhow::Result<Vec<u8>> {
    match encoder {
        Some(encoder) => encoder.encode_frame(pcm),
        None => Ok(pcm
            .iter()
            .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
            .collect()),
    }
}

#[cfg(test)]
mod tests;
