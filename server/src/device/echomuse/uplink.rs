//! Direct, budgeted PCM ingress with data-claim and capture-generation ownership.
use super::ble::BleLink;
use super::frames::{self, UplinkFrame};
use super::media::{MediaState, MAX_CAPTURE_US};
use super::turn::BoundedTurn;
use crate::media::media::{AudioChunk, AudioEpoch, AudioFormat};
use crate::workflow::protocol::{IncomingFrame, UtteranceEnd};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

pub(super) struct MicUplink {
    media: Arc<MediaState>,
    epoch: AudioEpoch,
    generation: Option<u64>,
    captured_us: u64,
}
impl MicUplink {
    #[cfg(test)]
    pub(super) fn new() -> anyhow::Result<Self> {
        let media = Arc::new(MediaState::default());
        media.claim();
        Ok(Self::for_media(media))
    }
    pub(super) fn for_media(media: Arc<MediaState>) -> Self {
        let epoch = AudioEpoch::capture(&media.generation, 0);
        Self {
            media,
            epoch,
            generation: None,
            captured_us: 0,
        }
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
                    turn.finish(UtteranceEnd::NoSpeech, up, |_| {});
                }
            }
            UplinkFrame::VadEnd => turn.finish_media(UtteranceEnd::Speech, up, self.epoch.clone()),
            UplinkFrame::NoSpeech => {
                turn.finish_media(UtteranceEnd::NoSpeech, up, self.epoch.clone())
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
