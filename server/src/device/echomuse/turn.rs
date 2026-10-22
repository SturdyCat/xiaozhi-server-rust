//! One bounded capture gate shared by the current control and its data claims.
//! Never look it up by device id: replacement controls own independent gates.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::ws::Message;
use tokio::sync::mpsc::UnboundedSender;

use crate::workflow::protocol::{IncomingFrame, UtteranceEnd};

#[derive(Debug, Default)]
struct State {
    active: bool,
    generation: u64,
    token: Arc<AtomicBool>,
}

#[derive(Debug, Default)]
pub(crate) struct BoundedTurn(Mutex<State>);

impl BoundedTurn {
    /// `source` = 触发来源的人类可读标签（`"本地唤醒词"` / `"action 键"`），随 detect 的
    /// `text` 字段透传给会话层——ESP 协议里 detect 的 `text` 本就是唤醒词（见
    /// `session::wakeup` 模块注释），echomuse 没有唤醒词文本，用它携带来源标签，
    /// 会话日志因此能区分"叫醒的"还是"按键开的"（会话层不感知设备类型）。
    fn begin(&self, up: &UnboundedSender<IncomingFrame>, source: &str) -> Option<u64> {
        let mut state = self.0.lock().expect("echomuse turn lock");
        if state.active {
            return None;
        }
        state.generation = state.generation.wrapping_add(1);
        state.active = true;
        state.token = Arc::new(AtomicBool::new(true));
        let detect = serde_json::json!({
            "type": "listen",
            "state": "detect",
            "text": source,
        });
        let _ = up.send(IncomingFrame::Text(detect.to_string()));
        Some(state.generation)
    }

    /// Serialize PCM enqueue with the boundary and the next wake. The callback
    /// must be synchronous; no socket I/O or await occurs while holding the lock.
    pub(crate) fn with_active_audio(&self, f: impl FnOnce(u64, Arc<AtomicBool>)) {
        let state = self.0.lock().expect("echomuse turn lock");
        if state.active {
            f(state.generation, state.token.clone());
        }
    }

    pub(crate) fn finish(
        &self,
        reason: UtteranceEnd,
        up: &UnboundedSender<IncomingFrame>,
        tail: impl FnOnce(u64),
    ) {
        self.finish_frame(reason, up, tail, None);
    }

    pub(crate) fn finish_media(
        &self,
        reason: UtteranceEnd,
        up: &UnboundedSender<IncomingFrame>,
        epoch: crate::media::media::AudioEpoch,
    ) {
        self.finish_frame(reason, up, |_| {}, Some(epoch));
    }

    fn finish_frame(
        &self,
        reason: UtteranceEnd,
        up: &UnboundedSender<IncomingFrame>,
        tail: impl FnOnce(u64),
        epoch: Option<crate::media::media::AudioEpoch>,
    ) {
        let mut state = self.0.lock().expect("echomuse turn lock");
        if !state.active {
            return;
        }
        tail(state.generation);
        if reason == UtteranceEnd::NoSpeech {
            state.token.store(false, Ordering::Release);
        }
        let frame = match epoch {
            Some(epoch) => IncomingFrame::AudioBoundary(reason, epoch),
            None => IncomingFrame::EndOfUtterance(reason),
        };
        let _ = up.send(frame);
        state.active = false; // Rearm capture now, not when a TTS reply finishes.
    }
}

/// Keep the firmware sequence detect -> mic_start -> listen start. A data end
/// can race the awaited control write; never emit a stale start after that end.
///
/// `source` = 触发来源标签，随 detect 的 `text` 透传（见 [`BoundedTurn::begin`] 注释）。
pub(super) async fn start_turn<S>(
    turn: &BoundedTurn,
    sink: &mut S,
    up: &UnboundedSender<IncomingFrame>,
    source: &str,
) -> anyhow::Result<()>
where
    S: futures_util::Sink<Message> + Unpin,
{
    let Some(generation) = turn.begin(up, source) else {
        return Ok(());
    };
    if futures_util::SinkExt::send(
        sink,
        Message::Text(r#"{"type":"mic_start","lock_mic":true}"#.into()),
    )
    .await
    .is_err()
    {
        let mut state = turn.0.lock().expect("echomuse turn lock");
        if state.active && state.generation == generation {
            let _ = up.send(IncomingFrame::EndOfUtterance(UtteranceEnd::NoSpeech));
            state.token.store(false, Ordering::Release);
            state.active = false;
        }
        anyhow::bail!("mic_start 下发失败（control 面已断）");
    }
    let state = turn.0.lock().expect("echomuse turn lock");
    if state.active && state.generation == generation {
        let _ = up.send(IncomingFrame::Text(
            r#"{"type":"listen","state":"start"}"#.into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
