//! Connection-owned PCM queue reservations and real data-writer completion.
//! BLE/control retain separate channels; no audio budget wait holds a hub/turn lock.
use crate::app::audio::budget::{AudioBudget, AudioPermit};
use crate::app::audio::media::{AudioEpoch, AudioFormat};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc::UnboundedSender, oneshot, Notify};

pub(crate) const AUDIO_QUEUE_MS: u64 = 2000;
pub(crate) const AUDIO_QUEUE_BYTES: usize = 128 * 1024;
pub(crate) const MAX_CAPTURE_US: u64 = 60_000_000;

#[derive(Debug)]
pub(crate) struct MediaState {
    pub generation: Arc<AtomicU64>,
    pub input: Arc<AudioBudget>,
    output: Arc<AudioBudget>,
    online: AtomicBool,
    closed: AtomicBool,
    writes: Mutex<VecDeque<WriteTicket>>,
    pub changed: Notify,
}
impl Default for MediaState {
    fn default() -> Self {
        Self {
            generation: Arc::new(AtomicU64::new(1)),
            input: AudioBudget::new(AUDIO_QUEUE_MS, AUDIO_QUEUE_BYTES),
            output: AudioBudget::new(AUDIO_QUEUE_MS, AUDIO_QUEUE_BYTES),
            online: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            writes: Mutex::new(VecDeque::new()),
            changed: Notify::new(),
        }
    }
}
impl MediaState {
    pub fn claim(&self) {
        let mut writes = self.writes.lock().expect("PCM writes lock");
        self.generation.fetch_add(1, Ordering::AcqRel);
        writes.clear();
        self.online
            .store(!self.closed.load(Ordering::Acquire), Ordering::Release);
        self.changed.notify_waiters();
    }
    pub fn ready(&self) -> bool {
        self.online.load(Ordering::Acquire) && !self.closed.load(Ordering::Acquire)
    }
    pub fn closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
    fn disconnect_locked(&self, writes: &mut VecDeque<WriteTicket>) {
        self.online.store(false, Ordering::Release);
        self.generation.fetch_add(1, Ordering::AcqRel);
        writes.clear();
        self.changed.notify_waiters();
    }
    pub fn disconnect(&self) {
        let mut writes = self.writes.lock().expect("PCM writes lock");
        self.disconnect_locked(&mut writes);
    }
    pub fn disconnect_if_current(&self, epoch: &AudioEpoch) {
        let mut writes = self.writes.lock().expect("PCM writes lock");
        if epoch.is_current() {
            self.disconnect_locked(&mut writes);
        }
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.disconnect();
    }

    /// Registration and enqueue are one transaction. Raw data channel stays the
    /// BLE-compatible wire adapter, but every speaker frame owns a bounded ticket.
    pub fn enqueue_for(
        &self,
        pcm: &[u8],
        tx: &UnboundedSender<super::DataOut>,
        epoch: &AudioEpoch,
    ) -> Option<oneshot::Receiver<bool>> {
        let mut writes = self.writes.lock().expect("PCM writes lock");
        if !self.ready() || !epoch.is_current() || pcm.len() != super::frames::SPEAKER_CHUNK_BYTES {
            return None;
        }
        let permit = self
            .output
            .reserve(AudioFormat::echo().duration_us(pcm.len()), pcm.len())?;
        let (ack, done) = oneshot::channel();
        writes.push_back(WriteTicket {
            epoch: epoch.clone(),
            ack: Some(ack),
            _permit: Some(permit),
        });
        if tx.send(super::frames::frame_speaker(pcm)).is_err() {
            writes.pop_back();
            return None;
        }
        Some(done)
    }
    pub fn enqueue_eos_for(&self, tx: &UnboundedSender<super::DataOut>, epoch: &AudioEpoch) {
        let mut writes = self.writes.lock().expect("PCM writes lock");
        if self.ready() && epoch.is_current() && writes.len() < 64 {
            writes.push_back(WriteTicket {
                epoch: epoch.clone(),
                ack: None,
                _permit: None,
            });
            if tx.send(super::frames::frame_speaker_eos()).is_err() {
                writes.pop_back();
            }
        }
    }
    #[cfg(test)]
    pub fn enqueue(
        &self,
        pcm: &[u8],
        tx: &UnboundedSender<super::DataOut>,
    ) -> Option<oneshot::Receiver<bool>> {
        self.enqueue_for(pcm, tx, &AudioEpoch::capture(&self.generation, 0))
    }
    #[cfg(test)]
    pub fn enqueue_eos(&self, tx: &UnboundedSender<super::DataOut>) {
        self.enqueue_eos_for(tx, &AudioEpoch::capture(&self.generation, 0));
    }
    pub fn take_write(&self) -> Option<WriteTicket> {
        self.writes.lock().expect("PCM writes lock").pop_front()
    }
}

#[derive(Debug)]
pub(crate) struct WriteTicket {
    epoch: AudioEpoch,
    ack: Option<oneshot::Sender<bool>>,
    _permit: Option<AudioPermit>,
}
impl WriteTicket {
    pub fn is_current(&self) -> bool {
        self.epoch.is_current()
    }
    pub fn complete(mut self, written: bool) {
        if let Some(ack) = self.ack.take() {
            let _ = ack.send(written);
        }
    }
}

#[cfg(test)]
mod tests;
