//! Direct, budgeted PCM ingress with data-claim and capture-generation ownership.
use super::ble::BleLink;
use super::frames::{self, UplinkFrame};
use super::media::{MediaState, MAX_CAPTURE_US};
use super::mic::MicControl;
use super::turn::BoundedTurn;
use crate::media::media::{AudioChunk, AudioEpoch, AudioFormat};
use crate::workflow::protocol::{IncomingFrame, UtteranceEnd};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

pub(super) struct MicUplink {
    media: Arc<MediaState>,
    mic: MicControl,
    fx: super::fx::Fx,
    epoch: AudioEpoch,
    generation: Option<u64>,
    captured_us: u64,
    /// 丢帧诊断日志限频（静默黑洞曾让"说完没反应"无从排查）。
    last_drop_log: std::time::Instant,
}
impl MicUplink {
    #[cfg(test)]
    pub(super) fn new(mic: MicControl, fx: super::fx::Fx) -> anyhow::Result<Self> {
        let media = Arc::new(MediaState::default());
        media.claim();
        Ok(Self::for_media(media, mic, fx))
    }
    pub(super) fn for_media(media: Arc<MediaState>, mic: MicControl, fx: super::fx::Fx) -> Self {
        let epoch = AudioEpoch::capture(&media.generation, 0);
        Self {
            media,
            mic,
            fx,
            epoch,
            generation: None,
            captured_us: 0,
            last_drop_log: std::time::Instant::now() - std::time::Duration::from_secs(60),
        }
    }
    /// 一轮收口（0x04/0x05 或溢出）：turn 已收掉才发官方三连
    /// （`mic_stop → beam_unlock → mic_start` 回常开流）；对未活跃轮是 no-op。
    /// 同时切灯：有语音 → 思考 spinner（ASR/LLM 进行中）；无语音 → 慢脉冲自清。
    /// 音乐恢复（duck_off）：语音轮次只压低、从不停止音乐面。
    fn turn_finished(&mut self, ended: bool, speech: bool) {
        if !ended {
            return;
        }
        if speech {
            self.fx.thinking();
        } else {
            self.fx.nospeech();
        }
        self.fx.duck_off();
        self.mic.turn_ended();
    }
    pub(super) fn on_binary(
        &mut self,
        raw: &[u8],
        turn: &BoundedTurn,
        up: &UnboundedSender<IncomingFrame>,
        ble: &Arc<Mutex<BleLink>>,
        ble_on: bool,
    ) {
        if !self.epoch.is_current() || !self.media.ready() {
            // 曾是静默 return：代际错位/媒体未就绪时设备所有上行帧（含 0x04 收口）
            // 无声消失，"说完没反应"无从排查——限频打点照亮丢弃原因。
            let now = std::time::Instant::now();
            if now.duration_since(self.last_drop_log) > std::time::Duration::from_secs(5) {
                self.last_drop_log = now;
                tracing::warn!(
                    "device uplink 丢帧：epoch_current={} media_ready={}（代际错位或 data 未就绪，本条限频 5s）",
                    self.epoch.is_current(),
                    self.media.ready()
                );
            }
            return;
        }
        match frames::parse_uplink(raw) {
            UplinkFrame::MicPcm(pcm) => {
                let mut overflow = false;
                turn.with_active_audio(|generation, token| {
                    if self.generation != Some(generation) {
                        self.generation = Some(generation);
                        self.captured_us = 0;
                    }
                    // Drop incomplete S16_LE byte. No codec frame/padding is needed for VAD.
                    let bytes = pcm.len() / 2 * 2;
                    if bytes == 0 {
                        return;
                    }
                    let us = AudioFormat::echo_mic().duration_us(bytes);
                    let Some(permit) = self.media.input.reserve(us, bytes) else {
                        overflow = true;
                        return;
                    };
                    self.captured_us = self.captured_us.saturating_add(us);
                    if self.captured_us > MAX_CAPTURE_US {
                        overflow = true;
                        return;
                    }
                    let mut chunk = AudioChunk::new(AudioFormat::echo_mic(), pcm[..bytes].to_vec());
                    chunk.epoch = Some(
                        AudioEpoch::capture(&self.media.generation, generation).with_turn(token),
                    );
                    chunk.permit = Some(permit);
                    let _ = up.send(IncomingFrame::Audio(chunk));
                });
                if overflow {
                    let ended = turn.finish(UtteranceEnd::NoSpeech, up, |_| {});
                    self.turn_finished(ended, false);
                }
            }
            UplinkFrame::VadEnd => {
                let ended = turn.finish_media(UtteranceEnd::Speech, up, self.epoch.clone());
                self.turn_finished(ended, true);
            }
            UplinkFrame::NoSpeech => {
                let ended = turn.finish_media(UtteranceEnd::NoSpeech, up, self.epoch.clone());
                self.turn_finished(ended, false);
            }
            UplinkFrame::BleAdverts(payload) if ble_on => {
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(payload) {
                    if let Some(list) = v.get("adverts").and_then(|a| a.as_array()) {
                        ble.lock()
                            .expect("echomuse ble link 锁中毒")
                            .merge_adverts(list, super::registry::now_secs());
                    }
                }
            }
            UplinkFrame::BleGatt(payload) if ble_on => {
                ble.lock()
                    .expect("echomuse ble link 锁中毒")
                    .on_gatt(payload);
            }
            _ => {}
        }
    }
}
#[cfg(test)]
#[path = "uplink_tests.rs"]
mod tests;
