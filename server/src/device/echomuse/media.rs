//! Connection-owned PCM queue reservations and real data-writer completion.
//! BLE/control retain separate channels; no audio budget wait holds a hub/turn lock.
use crate::media::budget::{AudioBudget, AudioPermit};
use crate::media::media::{AudioEpoch, AudioFormat};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc::UnboundedSender, oneshot, Notify};

pub(crate) const AUDIO_QUEUE_MS: u64 = 2000;
/// 输出队列字节额度：48 kHz mono S16LE = 96 kB/s，2 s 需 192 kB，取 256 kB 留余量
/// （时长额度 [`AUDIO_QUEUE_MS`] 仍会先到，字节额度只是防超大块绕开时长检查）。
pub(crate) const AUDIO_QUEUE_BYTES: usize = 256 * 1024;
pub(crate) const MAX_CAPTURE_US: u64 = 60_000_000;

#[derive(Debug)]
pub(crate) struct MediaState {
    pub generation: Arc<AtomicU64>,
    pub input: Arc<AudioBudget>,
    output: Arc<AudioBudget>,
    online: AtomicBool,
    closed: AtomicBool,
    writes: Mutex<VecDeque<WriteTicket>>,
    /// 音乐平面当前 epoch（None = 无播放）。与 voice 互相独立：语音轮次的
    /// disconnect_if_current 不得掐掉背景音乐（官方语义：语音 turn 只 duck）。
    music_epoch: Mutex<Option<AudioEpoch>>,
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
            music_epoch: Mutex::new(None),
            changed: Notify::new(),
        }
    }
}
impl MediaState {
    pub fn claim(&self) {
        let mut writes = self.writes.lock().expect("PCM writes lock");
        self.generation.fetch_add(1, Ordering::AcqRel);
        writes.clear();
        *self.music_epoch.lock().expect("music epoch lock") = None;
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
        let permit = self.output.reserve(
            AudioFormat::echo_speaker().duration_us(pcm.len()),
            pcm.len(),
        )?;
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

    // ── 音乐面（`0x04`/`0x05` 下行；设备 announce `audio_mix` 才使用）──

    /// 开一条音乐流 epoch（旧流立即作废：新播放顶掉旧播放）。断链/换设备时
    /// 由 `claim` 统一失效。返回的 epoch 由播放任务持有，逐帧校验。
    pub fn start_music(&self) -> Option<AudioEpoch> {
        if !self.ready() {
            return None;
        }
        let epoch = AudioEpoch::capture(&self.generation, 0);
        *self.music_epoch.lock().expect("music epoch lock") = Some(epoch.clone());
        Some(epoch)
    }

    /// 停止音乐流（管理面 stop）：epoch 作废 + `music_flush` 由调用方发 control 面。
    pub fn stop_music(&self) {
        *self.music_epoch.lock().expect("music epoch lock") = None;
        self.changed.notify_waiters();
    }

    /// 音乐帧入队（复用 output 预算：音乐与语音共享设备播放带宽与 2s 水库）。
    /// 返回 None = 未就绪/流已停/预算满（调用方节流重试）。
    pub fn enqueue_music(
        &self,
        pcm: &[u8],
        tx: &UnboundedSender<super::DataOut>,
    ) -> Option<oneshot::Receiver<bool>> {
        let epoch = self.music_epoch.lock().expect("music epoch lock").clone()?;
        let mut writes = self.writes.lock().expect("PCM writes lock");
        if !self.ready() || pcm.is_empty() || pcm.len() % 2 != 0 {
            return None;
        }
        let permit = self
            .output
            .reserve(AudioFormat::echo_speaker().duration_us(pcm.len()), pcm.len())?;
        let (ack, done) = oneshot::channel();
        writes.push_back(WriteTicket {
            epoch: epoch.clone(),
            ack: Some(ack),
            _permit: Some(permit),
        });
        if tx.send(super::frames::frame_music(pcm)).is_err() {
            writes.pop_back();
            return None;
        }
        Some(done)
    }

    /// 音乐流收尾（`0x05`）。只在流仍存活时入队。
    pub fn enqueue_music_eos(&self, tx: &UnboundedSender<super::DataOut>) {
        let Some(epoch) = self.music_epoch.lock().expect("music epoch lock").clone() else {
            return;
        };
        let mut writes = self.writes.lock().expect("PCM writes lock");
        if self.ready() && writes.len() < 64 {
            writes.push_back(WriteTicket {
                epoch,
                ack: None,
                _permit: None,
            });
            if tx.send(super::frames::frame_music_eos()).is_err() {
                writes.pop_back();
            }
        }
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
