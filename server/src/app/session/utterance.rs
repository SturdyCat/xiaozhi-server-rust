//! 有界话语边界与播报期间的有序上行缓冲（只对支持此契约的承载启用）。
use std::collections::VecDeque;

use crate::app::protocol::ClientMessage;
use crate::app::transport::{IncomingFrame, Transport, UtteranceEnd};
use crate::plugins::vad::pool::VadGuard;

/// Speech 才把已切段及尾段交给 ASR；NoSpeech 丢弃两者。两种结束都重置 recurrent state。
pub(super) fn finish_utterance(
    vad: &mut VadGuard,
    pending: &mut Vec<Vec<f32>>,
    reason: UtteranceEnd,
) -> Vec<Vec<f32>> {
    let mut segments = std::mem::take(pending);
    match reason {
        UtteranceEnd::Speech => vad.flush(&mut |seg| segments.push(seg)),
        UtteranceEnd::NoSpeech => segments.clear(),
    }
    vad.reset();
    segments
}

/// 下行 poll_abort 只观察打断/关闭；下一话语的音频和结束事件保持顺序回主循环。
/// ESP 未启用时完全沿用原有 try_recv 丢上行语义。上限触发关闭而非半句误识别。
pub(super) struct TurnTransport<'a, T> {
    inner: &'a mut T,
    bounded: bool,
    pending: VecDeque<IncomingFrame>,
    bytes: usize,
    overflowed: bool,
}

impl<'a, T> TurnTransport<'a, T> {
    pub(super) fn new(inner: &'a mut T, bounded: bool) -> Self {
        Self {
            inner,
            bounded,
            pending: VecDeque::new(),
            bytes: 0,
            overflowed: false,
        }
    }
}

fn frame_bytes(frame: &IncomingFrame) -> usize {
    match frame {
        IncomingFrame::Binary(b) => b.len(),
        IncomingFrame::Audio(audio) => audio.data.len(),
        IncomingFrame::Text(s) => s.len(),
        _ => 0,
    }
}

impl<T: Transport> Transport for TurnTransport<'_, T> {
    fn terminal_profile(
        &self,
        params: &crate::app::session::SessionParams,
    ) -> crate::app::audio::media::TerminalProfile {
        self.inner.terminal_profile(params)
    }
    async fn send_audio(
        &mut self,
        audio: crate::app::audio::media::AudioChunk,
        profile: crate::app::audio::media::TerminalProfile,
        timestamp: u32,
    ) -> anyhow::Result<crate::app::audio::media::AudioWrite> {
        self.inner.send_audio(audio, profile, timestamp).await
    }

    async fn send_text(&mut self, json: String) -> anyhow::Result<()> {
        self.inner.send_text(json).await
    }
    async fn send_binary(&mut self, data: Vec<u8>) -> anyhow::Result<()> {
        self.inner.send_binary(data).await
    }
    async fn recv(&mut self) -> IncomingFrame {
        if self.overflowed {
            return IncomingFrame::Closed;
        }
        if let Some(frame) = self.pending.pop_front() {
            self.bytes -= frame_bytes(&frame);
            return frame;
        }
        self.inner.recv().await
    }
    fn try_recv(&mut self) -> Option<IncomingFrame> {
        if self.overflowed {
            return Some(IncomingFrame::Closed);
        }
        if !self.bounded {
            return self.inner.try_recv();
        }
        while let Some(frame) = self.inner.try_recv() {
            let interrupt = match &frame {
                IncomingFrame::Closed => true,
                IncomingFrame::Text(s) => matches!(
                    serde_json::from_str::<ClientMessage>(s),
                    Ok(ClientMessage::Abort { .. })
                ),
                _ => false,
            };
            if interrupt {
                if matches!(frame, IncomingFrame::Closed) {
                    // 断开粘滞优先于之前暂存的话语；不得离线后再派发 ASR。
                    self.pending.clear();
                    self.bytes = 0;
                    self.overflowed = true;
                }
                return Some(frame);
            }
            self.bytes += frame_bytes(&frame);
            if self.pending.len() >= 4096 || self.bytes > 8 * 1024 * 1024 {
                self.pending.clear();
                self.overflowed = true;
                return Some(IncomingFrame::Closed);
            }
            self.pending.push_back(frame);
        }
        None
    }
    async fn keepalive(&mut self) {
        self.inner.keepalive().await;
    }
    async fn close(&mut self) {
        self.inner.close().await;
    }
}

#[cfg(test)]
mod tests;
