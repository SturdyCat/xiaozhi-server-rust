//! Typed terminal media contract. Wire framing remains a transport adapter concern.

use super::budget::AudioPermit;
use crate::workflow::protocol::BinVersion;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioCodec {
    Opus,
    PcmS16Le,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioFormat {
    pub codec: AudioCodec,
    /// Our encoder/PCM conversion contract revision, not an ESP binary header version.
    pub codec_version: u16,
    pub sample_rate: u32,
    pub channels: u16,
    pub frame_ms: u32,
}

impl AudioFormat {
    pub const fn new(codec: AudioCodec, sample_rate: u32, frame_ms: u32) -> Self {
        Self {
            codec,
            codec_version: 1,
            sample_rate,
            channels: 1,
            frame_ms,
        }
    }
    /// EchoMuse **上行**（麦克风）：协议常量 mono `S16_LE` 16 kHz / 40 ms。
    pub const fn echo_mic() -> Self {
        Self::new(AudioCodec::PcmS16Le, 16_000, 40)
    }
    /// EchoMuse **下行**（扬声器）：线缆格式为 mono `S16_LE` **48 kHz** / 40 ms。
    ///
    /// 依据上游 `controller/CLAUDE.md`：控制器把 TTS 用 ffmpeg 直接解码成
    /// "48kHz mono" 再发 `0x02` 帧；设备端 DAC 同样是 48 kHz 输出（ALSA 写入时
    /// 复制 L=R）。设备收到 PCM 后**不重采样**，直接按 48 kHz 播放。
    /// ⚠️ 早期按 16 kHz 下发时，设备把 16k 流当 48k 播 → 3 倍速、音调拔高
    /// （听感即"快进的高频尖叫声"）。
    pub const fn echo_speaker() -> Self {
        Self::new(AudioCodec::PcmS16Le, 48_000, 40)
    }
    pub fn identity(self) -> String {
        format!(
            "{:?}:v{}:{}:{}:{}",
            self.codec, self.codec_version, self.sample_rate, self.channels, self.frame_ms
        )
    }
    pub fn duration_us(self, bytes: usize) -> u64 {
        match self.codec {
            AudioCodec::PcmS16Le => {
                (bytes as u64).saturating_mul(1_000_000)
                    / (u64::from(self.sample_rate.max(1)) * u64::from(self.channels.max(1)) * 2)
            }
            AudioCodec::Opus => u64::from(self.frame_ms) * 1000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalCapabilities {
    pub bounded_utterance: bool,
    pub mcp: bool,
    pub playback_ack: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct TerminalProfile {
    pub input: AudioFormat,
    pub output: AudioFormat,
    /// ESP-only compatibility envelope. Echo's PCM path never consumes it.
    pub binary_version: BinVersion,
    pub capabilities: TerminalCapabilities,
}

impl TerminalProfile {
    pub fn esp(params: &crate::workflow::session::SessionParams) -> Self {
        Self {
            input: AudioFormat::new(AudioCodec::Opus, params.uplink_sr, 60),
            output: params.output_format(),
            binary_version: params.downlink_bin_ver,
            capabilities: TerminalCapabilities {
                bounded_utterance: false,
                mcp: false,
                playback_ack: false,
            },
        }
    }
    pub fn echo() -> Self {
        Self {
            input: AudioFormat::echo_mic(),
            output: AudioFormat::echo_speaker(),
            binary_version: BinVersion::V1,
            capabilities: TerminalCapabilities {
                bounded_utterance: true,
                mcp: false,
                playback_ack: false,
            },
        }
    }
}

/// A stamp follows queued audio through deferred transport/VAD reads.
/// Incrementing the shared generation invalidates old data claims atomically.
#[derive(Debug, Clone)]
pub struct AudioEpoch {
    generation: Arc<AtomicU64>,
    value: u64,
    pub turn: u64,
    turn_active: Option<Arc<AtomicBool>>,
}

impl AudioEpoch {
    pub fn capture(generation: &Arc<AtomicU64>, turn: u64) -> Self {
        Self {
            value: generation.load(Ordering::Acquire),
            generation: generation.clone(),
            turn,
            turn_active: None,
        }
    }
    pub fn with_turn(mut self, active: Arc<AtomicBool>) -> Self {
        self.turn_active = Some(active);
        self
    }
    pub fn is_current(&self) -> bool {
        self.value == self.generation.load(Ordering::Acquire)
            && self
                .turn_active
                .as_ref()
                .is_none_or(|active| self.turn != 0 && active.load(Ordering::Acquire))
    }
}

#[derive(Debug)]
pub struct AudioChunk {
    pub format: AudioFormat,
    pub data: Vec<u8>,
    pub epoch: Option<AudioEpoch>,
    // Kept while queued or deferred; Drop releases both bytes and duration.
    pub(crate) permit: Option<AudioPermit>,
}

impl AudioChunk {
    pub fn new(format: AudioFormat, data: Vec<u8>) -> Self {
        Self {
            format,
            data,
            epoch: None,
            permit: None,
        }
    }
    pub fn is_current(&self) -> bool {
        self.epoch.as_ref().is_none_or(AudioEpoch::is_current)
    }
}

/// This is transport/socket write completion, NEVER a device playback ACK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioWrite {
    SocketWritten,
    Cancelled,
}

#[cfg(test)]
mod tests;
