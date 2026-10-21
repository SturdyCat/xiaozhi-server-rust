//! Native media adapters feed the same VAD/ASR path as the legacy ESP decoder.
use super::{Session, SessionParams};
use crate::app::audio::media::{AudioChunk, AudioCodec, AudioFormat};
use crate::app::transport::Transport;

impl SessionParams {
    /// Saved-defaults diagnostic, NOT a claim about an ESP's negotiated hello.
    pub fn configured_defaults(cfg: &crate::config::Config, codec: AudioCodec) -> Self {
        let echo = codec == AudioCodec::PcmS16Le;
        // EchoMuse 上下行几何不同：mic 16 kHz、speaker 48 kHz（设备不重采样，
        // 见 `AudioFormat::echo_speaker` 的上游依据）。都取协议常量，不跟随
        // `[audio]` 里给 ESP 的 24 k/60 ms 配置。
        let echo_out = AudioFormat::echo_speaker();
        let version = if echo {
            crate::app::protocol::BinVersion::V1
        } else {
            crate::app::protocol::BinVersion::from_u8(cfg.audio.binary_protocol_version)
        };
        let mut params = Self {
            uplink_bin_ver: version,
            downlink_bin_ver: version,
            uplink_sr: 16_000,
            downlink_sr: if echo {
                echo_out.sample_rate
            } else {
                cfg.audio.downlink_sample_rate
            },
            output_codec: codec,
            downlink_frame_ms: if echo {
                echo_out.frame_ms
            } else {
                cfg.audio.downlink_frame_duration_ms
            },
            downlink_lead_ms: if echo { 0 } else { cfg.audio.downlink_lead_ms },
            downlink_buffer_ms: cfg.audio.downlink_buffer_ms,
            download_buffer_ms: cfg.audio.download_budget_ms(),
            downlink_buffer_start_ms: cfg.audio.downlink_buffer_start_ms,
            downlink_buffer_start_max_ms: cfg.audio.downlink_buffer_start_max_ms,
        };
        cfg.playback.apply(&mut params);
        params
    }
    pub fn output_format(&self) -> AudioFormat {
        AudioFormat::new(self.output_codec, self.downlink_sr, self.downlink_frame_ms)
    }
}
pub(super) fn native_pcm(chunk: &AudioChunk) -> Option<Vec<f32>> {
    if !chunk.is_current() || chunk.format != AudioFormat::echo_mic() || chunk.data.len() % 2 != 0
    {
        return None;
    }
    Some(
        chunk
            .data
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect(),
    )
}

impl<T: Transport> Session<'_, T> {
    pub(super) async fn handle_utterance_end(
        &mut self,
        reason: crate::app::transport::UtteranceEnd,
    ) {
        let segments =
            super::utterance::finish_utterance(&mut self.vad, &mut self.pending_segments, reason);
        if self.real_audio && !self.close_requested && !self.is_test && !segments.is_empty() {
            self.recognize_segments(segments).await;
        }
    }
    pub(super) async fn handle_audio(&mut self, chunk: AudioChunk) {
        if let Some(pcm) = native_pcm(&chunk) {
            self.handle_pcm(&pcm).await;
        }
    }

    pub(super) async fn handle_binary(&mut self, data: &[u8]) {
        let payload = crate::app::protocol::unwrap_uplink(self.params.uplink_bin_ver, data);
        if payload.is_empty() {
            return;
        }
        let Some(decoder) = self.uplink_decoder.as_mut() else {
            return;
        };
        let Ok(pcm) = decoder.decode_frame(payload) else {
            return;
        };
        self.handle_pcm(&pcm).await;
    }

    async fn handle_pcm(&mut self, pcm: &[f32]) {
        if self.test_recording {
            self.test_buf.extend_from_slice(pcm);
            return;
        }
        if !self.real_audio {
            return;
        }
        let mut segments = Vec::new();
        self.vad.accept(pcm, &mut |seg| segments.push(seg));
        if self.bounded_turn {
            self.pending_segments.extend(segments);
        } else {
            self.recognize_segments(segments).await;
        }
    }
}

/// Kept here for the session constructor: native PCM never constructs a codec.
pub(super) fn needs_opus(codec: AudioCodec) -> bool {
    codec == AudioCodec::Opus
}

#[cfg(test)]
mod tests;
