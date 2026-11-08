//! Native media adapters feed the same VAD/ASR path as the legacy ESP decoder.
use super::{Session, SessionParams, SessionTransportProfile};
use crate::media::media::{AudioChunk, AudioCodec, AudioFormat};
use crate::workflow::protocol::Transport;

impl SessionParams {
    /// Saved-defaults diagnostic, NOT a claim about a negotiated hello.
    ///
    /// The codec-specific profile is used only by diagnostics and tests. Real
    /// connection entries construct the same generic profile explicitly.
    pub fn configured_defaults(cfg: &crate::config::Config, codec: AudioCodec) -> Self {
        let native_pcm = codec == AudioCodec::PcmS16Le;
        let version = if native_pcm {
            crate::workflow::protocol::BinVersion::V1
        } else {
            crate::workflow::protocol::BinVersion::from_u8(cfg.audio.binary_protocol_version)
        };
        let (uplink_sr, downlink_sr, frame_ms) = if native_pcm {
            (16_000, 48_000, 40)
        } else {
            (
                16_000,
                cfg.audio.downlink_sample_rate,
                cfg.audio.downlink_frame_duration_ms,
            )
        };
        let mut params = Self {
            kind: if native_pcm {
                SessionTransportProfile::plugin("native-pcm", true)
            } else {
                SessionTransportProfile::core()
            },
            uplink_bin_ver: version,
            downlink_bin_ver: version,
            uplink_sr,
            downlink_sr,
            output_codec: codec,
            downlink_frame_ms: frame_ms,
            downlink_lead_ms: if native_pcm {
                0
            } else {
                cfg.audio.downlink_lead_ms
            },
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
    let input = AudioFormat::new(AudioCodec::PcmS16Le, 16_000, 40);
    if !chunk.is_current() || chunk.format != input || chunk.data.len() % 2 != 0 {
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

/// 设备 VAD 会话的上行累积：**拼成单一连续段**（对齐官方 controller——`0x01` PCM 逐块
/// 进同一队列、`0x04` 哨兵才收口整段送 STT）。逐块 push 会让 `recognize_segments`
/// 对每 160ms 碎片各跑一次 ASR，太短识别为空且静默跳过（真机表现：说了话、零日志、
/// 零响应）。
fn accumulate_device_pcm(pending: &mut Vec<Vec<f32>>, pcm: &[f32]) {
    match pending.last_mut() {
        Some(acc) => acc.extend_from_slice(pcm),
        None => pending.push(pcm.to_vec()),
    }
}

impl<T: Transport> Session<'_, T> {
    pub(super) async fn handle_utterance_end(
        &mut self,
        reason: crate::workflow::protocol::UtteranceEnd,
    ) {
        // 设备 VAD 会话：收口日志是现场唯一的"听到话了"信号——直通后服务端不再跑
        // VAD，原先由切段触发的日志一并消失，缺了它只能靠猜（"说完没反应"无从下手）。
        let device_ms = if self.device_vad && self.real_audio && !self.is_test {
            let samples: usize = self.pending_segments.iter().map(Vec::len).sum();
            Some(samples.saturating_mul(1000) / self.params.uplink_sr.max(1) as usize)
        } else {
            None
        };
        let segments = super::utterance::finish_utterance(
            self.vad.as_mut(),
            &mut self.pending_segments,
            reason,
        );
        if let Some(ms) = device_ms {
            match reason {
                crate::workflow::protocol::UtteranceEnd::Speech => tracing::info!(
                    "session {} 设备 VAD 收口：{ms}ms 语音（{} 段）→ ASR",
                    self.session_id,
                    segments.len()
                ),
                crate::workflow::protocol::UtteranceEnd::NoSpeech => {
                    tracing::info!(
                        "session {} 设备 VAD 收口：本轮未检测到语音（{ms}ms 已丢弃）",
                        self.session_id
                    )
                }
            }
        }
        self.delivery_idle = true;
        if self.real_audio && !self.close_requested && !self.is_test && !segments.is_empty() {
            self.recognize_segments(segments).await;
        }
    }
    /// Native PCM entry 上行：原生 mono PCM 块。格式守卫在
    /// [`native_pcm`]，类型判定来自入口声明的 transport profile。
    pub(super) async fn handle_audio(&mut self, chunk: AudioChunk) {
        debug_assert!(
            self.params.kind.device_vad,
            "native PCM audio requires an entry-declared device-VAD profile"
        );
        if let Some(pcm) = native_pcm(&chunk) {
            self.handle_pcm(&pcm).await;
        }
    }

    /// Opus entry 上行：二进制信封（unwrap → Opus 解码）。Native PCM entries
    /// do not construct `uplink_decoder`, so an unexpected binary frame is ignored.
    pub(super) async fn handle_binary(&mut self, data: &[u8]) {
        let payload = crate::workflow::protocol::unwrap_uplink(self.params.uplink_bin_ver, data);
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
        self.delivery_idle = false;
        if self.test_recording {
            self.test_buf.extend_from_slice(pcm);
            return;
        }
        if !self.real_audio {
            return;
        }
        if self.device_vad {
            // 设备端 speech gate 已判"这是人声"（Silero VAD 生效）：跳过服务端检测，
            // 累积为单一连续段，等设备 0x04（UtteranceEnd::Speech）整段送 ASR。
            // 双 VAD 口径不一致会吞字，且每帧重复推理白占一个 VAD 池名额。
            accumulate_device_pcm(&mut self.pending_segments, pcm);
            return;
        }
        let mut segments = Vec::new();
        self.vad
            .as_mut()
            .expect("非 device_vad 会话必须持有 VAD 实例")
            .accept(pcm, &mut |seg| segments.push(seg));
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
