//! One bounded capture gate shared by the current control and its data claims.
//! Never look it up by device id: replacement controls own independent gates.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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
    /// `is_wakeword` = 这一轮是不是**设备端唤醒词命中**开的（对齐官方
    /// `em_controller._run_voice_locked(is_wakeword=...)`）。
    ///
    /// 只有唤醒词命中才发 `listen state=detect`——会话层把它当"用户刚叫醒我"，
    /// 据此播唤醒问候（`[wakeup]` 的「我在」）。**手动按键是"我要说话"，
    /// 不是"我在叫你"**：官方 `_button_voice_turn` 传 `is_wakeword=False`，
    /// 按键轮直接进聆听、不问候。此前一律发 detect 导致按 action 键先答一句
    /// 「我在」，把聆听态盖成播报态——用户既没看到聆听灯，也得多等一句话。
    fn begin(
        &self,
        up: &UnboundedSender<IncomingFrame>,
        source: &str,
        is_wakeword: bool,
    ) -> Option<u64> {
        let mut state = self.0.lock().expect("echomuse turn lock");
        if state.active {
            return None;
        }
        state.generation = state.generation.wrapping_add(1);
        state.active = true;
        state.token = Arc::new(AtomicBool::new(true));
        if is_wakeword {
            let detect = serde_json::json!({
                "type": "listen",
                "state": "detect",
                "text": source,
            });
            let _ = up.send(IncomingFrame::Text(detect.to_string()));
        }
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
    ) -> bool {
        self.finish_frame(reason, up, tail, None)
    }

    pub(crate) fn finish_media(
        &self,
        reason: UtteranceEnd,
        up: &UnboundedSender<IncomingFrame>,
        epoch: crate::media::media::AudioEpoch,
    ) -> bool {
        self.finish_frame(reason, up, |_| {}, Some(epoch))
    }

    /// 返回是否真的收掉了一轮（turn 未活跃时是 no-op，返回 false）——调用方凭它
    /// 决定要不要发采集收口三连，避免对已结束的轮重复下发。
    fn finish_frame(
        &self,
        reason: UtteranceEnd,
        up: &UnboundedSender<IncomingFrame>,
        tail: impl FnOnce(u64),
        epoch: Option<crate::media::media::AudioEpoch>,
    ) -> bool {
        let mut state = self.0.lock().expect("echomuse turn lock");
        if !state.active {
            return false;
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
        true
    }
}

/// 开一轮 bounded turn，对齐官方按键开轮时序
/// （`em_controller.py:_button_voice_turn`：`mic_stop → mic_start_turn`，加
/// `beam_lock`；固件 `data.go` 注释「the controller sends that pair after every
/// voice turn」——`StartMic` 对已激活流是忽略，不停就起不来）。
///
/// 顺带处理打断与反馈：`speaker_flush` 掐掉上一轮还在设备缓冲里的播报（"打断
/// 了还在说"），`led_anim` 聆听环 + `play_cue` 唤醒音立即给用户反馈。
///
/// `source` = 触发来源标签，随 detect 的 `text` 透传（见 [`BoundedTurn::begin`] 注释）。
/// `is_wakeword` = 唤醒词命中开的轮（`true`）还是手动按键开的轮（`false`）：
/// 前者播唤醒问候 + 唤醒提示音，后者**都不做**，直接给聆听环等用户说话。
/// 同步下发：control 出站是 FIFO channel，无 await 竞态窗口；通道已断时立即按
/// NoSpeech 收口，绝不能留下一个"自认为在聆听"的门等不到任何音频。
pub(super) fn start_turn(
    live: &super::runtime::LiveControl,
    up: &UnboundedSender<IncomingFrame>,
    source: &str,
    is_wakeword: bool,
) {
    let (turn, mic, fx) = (&live.turn, &live.mic, &live.fx);
    let Some(generation) = turn.begin(up, source, is_wakeword) else {
        return; // 已有一轮在进行：忽略重复触发（与会话层语义一致）
    };
    fx.speaker_flush(); // 打断：旧回答的缓冲音频立即作废
    fx.duck_on(); // 音乐压低（设备侧混音；轮末 duck_off）
    let mut ok = mic.stop();
    ok &= mic.beam_lock();
    ok &= mic.turn_stream();
    if !ok {
        let mut state = turn.0.lock().expect("echomuse turn lock");
        if state.active && state.generation == generation {
            let _ = up.send(IncomingFrame::EndOfUtterance(UtteranceEnd::NoSpeech));
            state.token.store(false, Ordering::Release);
            state.active = false;
        }
        return;
    }
    // 反馈：聆听环 + 唤醒音（wakeSound 开启时设备才发声）。fx 与 mic 同一条
    // FIFO channel，先灯后音的顺序就是发给设备的顺序。
    //
    // `wake_cue` 只属于**唤醒词命中**（官方 `play_cue("wake")` 也只在唤醒路径发，
    // 按键路径 `_button_voice_turn` 不发）。按键是用户主动说话，不需要"我听到了
    // 你叫我"的确认音；聆听环就是他要的反馈。
    fx.listening();
    if is_wakeword {
        fx.wake_cue();
    }
    // 会话层语义：采集门已开（设备侧指令已入队）。保持 detect → start 的顺序，
    // 会话层据此进入"可收音频"状态。
    let state = turn.0.lock().expect("echomuse turn lock");
    if state.active && state.generation == generation {
        let _ = up.send(IncomingFrame::Text(
            r#"{"type":"listen","state":"start"}"#.into(),
        ));
    }
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
