//! Keep delivery-owned cancellation separate from genuine user/transport abort.
use crate::media::media::{AudioChunk, AudioWrite, TerminalProfile};
use crate::workflow::protocol::{ClientMessage, IncomingFrame, Transport};
use crate::workflow::session::SessionParams;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub(super) struct DeliveryTransport<'a, T: Transport> {
    pub inner: &'a mut T,
    pub user_abort: Arc<AtomicBool>,
    pub delivery_abort: Arc<AtomicBool>,
    pub started: bool,
}
impl<T: Transport> DeliveryTransport<'_, T> {
    fn cancel_user(&self) {
        self.user_abort.store(true, Ordering::Relaxed);
        self.delivery_abort.store(true, Ordering::Relaxed);
    }
    fn cancelled(&self) -> bool {
        if self.user_abort.load(Ordering::Relaxed) {
            self.delivery_abort.store(true, Ordering::Relaxed);
        }
        self.delivery_abort.load(Ordering::Relaxed)
    }
    fn observe(&self, frame: &IncomingFrame) {
        if matches!(frame, IncomingFrame::Closed)
            || matches!(frame,
            IncomingFrame::Text(text) if matches!(serde_json::from_str::<ClientMessage>(text),Ok(ClientMessage::Abort{..})))
        {
            self.cancel_user();
        }
    }
}
impl<T: Transport> Transport for DeliveryTransport<'_, T> {
    fn terminal_profile(&self, params: &SessionParams) -> TerminalProfile {
        self.inner.terminal_profile(params)
    }
    async fn send_text(&mut self, text: String) -> anyhow::Result<()> {
        let value: serde_json::Value = serde_json::from_str(&text)?;
        let tts = value["type"] == "tts";
        let stop = tts && value["state"] == "stop";
        anyhow::ensure!(
            stop || !self.cancelled(),
            "delivery cancelled before text submission"
        );
        self.inner.send_text(text).await?;
        if tts && value["state"] == "start" {
            self.started = true;
        }
        if stop {
            self.started = false;
        }
        Ok(())
    }
    async fn send_binary(&mut self, data: Vec<u8>) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.cancelled(),
            "delivery cancelled before binary submission"
        );
        self.inner.send_binary(data).await
    }
    async fn send_audio(
        &mut self,
        audio: AudioChunk,
        profile: TerminalProfile,
        timestamp: u32,
    ) -> anyhow::Result<AudioWrite> {
        if self.cancelled() {
            return Ok(AudioWrite::Cancelled);
        }
        let result = self.inner.send_audio(audio, profile, timestamp).await?;
        // A known lease/task cancellation must not be reclassified as user
        // cancellation if it races the native writer's cancellation result.
        if result == AudioWrite::Cancelled && !self.delivery_abort.load(Ordering::Relaxed) {
            self.cancel_user();
        }
        Ok(result)
    }
    async fn recv(&mut self) -> IncomingFrame {
        let frame = self.inner.recv().await;
        self.observe(&frame);
        frame
    }
    fn try_recv(&mut self) -> Option<IncomingFrame> {
        self.cancelled();
        let frame = self.inner.try_recv();
        if let Some(frame) = &frame {
            self.observe(frame);
        }
        frame
    }
    async fn keepalive(&mut self) {
        self.inner.keepalive().await;
    }
    async fn close(&mut self) {
        self.inner.close().await;
    }
}
