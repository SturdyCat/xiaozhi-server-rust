//! Native PCM Echo adapter. No Opus codec exists in either Echo direction.
use super::fx::Fx;
use super::{media::MediaState, ControlOut, DataOut};
use crate::media::media::*;
use crate::workflow::protocol::{IncomingFrame, Transport};
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub struct EchomuseTransport {
    up_rx: UnboundedReceiver<IncomingFrame>,
    ctl_tx: UnboundedSender<ControlOut>,
    data_tx: UnboundedSender<DataOut>,
    media: Arc<MediaState>,
    closed: bool,
    reply_epoch: Option<AudioEpoch>,
}
impl EchomuseTransport {
    #[cfg(test)]
    pub fn new(
        up_rx: UnboundedReceiver<IncomingFrame>,
        ctl_tx: UnboundedSender<ControlOut>,
        data_tx: UnboundedSender<DataOut>,
    ) -> Result<Self> {
        let media = Arc::new(MediaState::default());
        media.claim();
        Ok(Self::with_media(up_rx, ctl_tx, data_tx, media))
    }
    pub(crate) fn with_media(
        up_rx: UnboundedReceiver<IncomingFrame>,
        ctl_tx: UnboundedSender<ControlOut>,
        data_tx: UnboundedSender<DataOut>,
        media: Arc<MediaState>,
    ) -> Self {
        let reply_epoch = Some(AudioEpoch::capture(&media.generation, 0));
        Self {
            up_rx,
            ctl_tx,
            data_tx,
            media,
            closed: false,
            reply_epoch,
        }
    }
}
impl Transport for EchomuseTransport {
    fn terminal_profile(
        &self,
        params: &crate::workflow::session::SessionParams,
    ) -> TerminalProfile {
        let _ = params;
        TerminalProfile::echo()
    }
    async fn send_audio(
        &mut self,
        audio: AudioChunk,
        profile: TerminalProfile,
        _: u32,
    ) -> Result<AudioWrite> {
        anyhow::ensure!(
            audio.format == AudioFormat::echo_speaker() && profile.output == audio.format,
            "Echo requires PCM48k/40ms mono S16LE"
        );
        let Some(epoch) = self.reply_epoch.clone() else {
            return Ok(AudioWrite::Cancelled);
        };
        let Some(done) = self.media.enqueue_for(&audio.data, &self.data_tx, &epoch) else {
            return Ok(AudioWrite::Cancelled);
        };
        // Only data_loop's completed WebSocket send acknowledges this receipt.
        match tokio::time::timeout(Duration::from_secs(5), done).await {
            Ok(Ok(true)) if epoch.is_current() => Ok(AudioWrite::SocketWritten),
            _ => {
                self.media.disconnect_if_current(&epoch);
                Ok(AudioWrite::Cancelled)
            }
        }
    }
    async fn send_text(&mut self, json: String) -> Result<()> {
        let value = serde_json::from_str::<serde_json::Value>(&json).ok();
        // 灯效走同一 control 通道：播报开始 → meter 环；收尾 → 熄灯。
        let fx = Fx::bound(self.ctl_tx.clone());
        if value
            .as_ref()
            .is_some_and(|v| v["type"] == "tts" && v["state"] == "start")
        {
            fx.speaking();
            // Initial data identify may follow register. Wait with a deadline;
            // a reconnect later in this reply must never rebind its media epoch.
            let _ = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let changed = self.media.changed.notified();
                    tokio::pin!(changed);
                    changed.as_mut().enable();
                    if self.media.ready() || self.media.closed() {
                        break;
                    }
                    changed.await;
                }
            })
            .await;
            self.reply_epoch = Some(AudioEpoch::capture(&self.media.generation, 0));
        }
        let is_stop = value.is_some_and(|v| v["type"] == "tts" && v["state"] == "stop");
        // Framer already padded/sent the final PCM frame; EOS follows its write ACK.
        // No data claim means no queueing EOS for replay on a later connection.
        if is_stop {
            fx.off(); // 播报收尾：环立即熄（meter 的长死信是给异常路径的兜底）
            if let Some(epoch) = self.reply_epoch.take() {
                self.media.enqueue_eos_for(&self.data_tx, &epoch);
            }
        }
        Ok(())
    }
    async fn send_binary(&mut self, _: Vec<u8>) -> Result<()> {
        anyhow::bail!("Echo native PCM uses typed send_audio, not legacy Opus binary")
    }
    async fn recv(&mut self) -> IncomingFrame {
        if self.closed {
            return IncomingFrame::Closed;
        }
        match self.up_rx.recv().await {
            Some(frame) => {
                if matches!(frame, IncomingFrame::Closed) {
                    self.closed = true;
                }
                frame
            }
            None => {
                self.closed = true;
                IncomingFrame::Closed
            }
        }
    }
    fn try_recv(&mut self) -> Option<IncomingFrame> {
        if self.closed {
            return Some(IncomingFrame::Closed);
        }
        match self.up_rx.try_recv() {
            Ok(frame) => {
                if matches!(frame, IncomingFrame::Closed) {
                    self.closed = true;
                }
                Some(frame)
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => None,
            Err(_) => {
                self.closed = true;
                Some(IncomingFrame::Closed)
            }
        }
    }
    async fn keepalive(&mut self) {}
    async fn close(&mut self) {
        self.closed = true;
        self.media.close();
        let _ = self.ctl_tx.send(ControlOut::Shutdown);
    }
}
#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
