use super::*;
use crate::device::echomuse::ControlOut;

/// 已绑定出站通道的 LiveControl + 指令接收端（断言下发序列用）。
fn bound_live() -> (
    crate::device::echomuse::runtime::LiveControl,
    tokio::sync::mpsc::UnboundedReceiver<ControlOut>,
) {
    let live = crate::device::echomuse::runtime::LiveControl::new(None, false);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    live.mic.bind(tx.clone());
    live.fx.bind(tx);
    (live, rx)
}

/// 按序取出 control 指令的 type 字段。
fn ctl_types(rx: &mut tokio::sync::mpsc::UnboundedReceiver<ControlOut>) -> Vec<String> {
    ctl_json(rx)
        .iter()
        .map(|v| v["type"].as_str().unwrap_or("").to_string())
        .collect()
}

/// 按序取出 control 指令（断言字段形状用）。
fn ctl_json(rx: &mut tokio::sync::mpsc::UnboundedReceiver<ControlOut>) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    while let Some(ControlOut::Json(json)) = rx.try_recv().ok() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
            out.push(v);
        }
    }
    out
}

/// 手动按键开轮 = 官方 `_button_voice_turn`（`_run_voice_locked(is_wakeword=False)`）：
/// **只进聆听**——不播唤醒问候「我在」（那要 detect 触发）、不播唤醒提示音，
/// 但必须亮聆听环（按键轮的全部反馈就是这个环）。
#[tokio::test]
async fn button_turn_opens_listening_without_wake_greeting_or_cue() {
    let (live, mut rx) = bound_live();
    let (up, mut up_rx) = tokio::sync::mpsc::unbounded_channel();
    start_turn(&live, &up, "action 键", false);

    // 上行唯一一帧是 listen start：没有 detect，会话层就不会去播「我在」。
    assert!(
        matches!(up_rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains(r#""state":"start""#))
    );
    assert!(
        up_rx.try_recv().is_err(),
        "按键轮不得发 listen detect（会话层收到它就播唤醒问候）"
    );

    // 官方开轮序列：打断 flush → 音乐 duck → 停流 → 锁波束 → turn 流 → 聆听环。
    // 末位不是 play_cue：官方 play_cue("wake") 只走唤醒路径。
    assert_eq!(
        ctl_types(&mut rx),
        vec![
            "speaker_flush",
            "duck",
            "mic_stop",
            "beam_lock",
            "mic_start",
            "led_anim"
        ],
        "按键开轮到聆听环为止，不发唤醒提示音"
    );
}

/// 唤醒词命中开轮 = 官方唤醒路径：detect 携带来源（会话层据此问候）+ 唤醒提示音。
#[tokio::test]
async fn wakeword_turn_announces_source_and_plays_cue() {
    let (live, mut rx) = bound_live();
    let (up, mut up_rx) = tokio::sync::mpsc::unbounded_channel();
    start_turn(&live, &up, "本地唤醒词", true);
    assert!(
        matches!(
            up_rx.try_recv().unwrap(),
            IncomingFrame::Text(t)
                if t.contains(r#""state":"detect""#) && t.contains("本地唤醒词")
        ),
        "唤醒词轮必须发 detect（text 带来源标签）"
    );
    assert!(
        matches!(up_rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains(r#""state":"start""#))
    );
    assert_eq!(
        ctl_types(&mut rx),
        vec![
            "speaker_flush",
            "duck",
            "mic_stop",
            "beam_lock",
            "mic_start",
            "led_anim",
            "play_cue"
        ],
        "唤醒词轮末尾有唤醒提示音"
    );
}

/// 聆听环形状对齐官方 `listening_anim`（`em_scenes`）：solid + 实心绿 + listening 标记。
#[tokio::test]
async fn listening_ring_matches_official_listening_anim() {
    let (live, mut rx) = bound_live();
    let (up, _up_rx) = tokio::sync::mpsc::unbounded_channel();
    start_turn(&live, &up, "action 键", false);
    let msgs = ctl_json(&mut rx);
    let ring = msgs.last().expect("按键开轮必须下发聆听环");
    assert_eq!(ring["type"], "led_anim");
    assert_eq!(ring["anim"]["pattern"], "solid");
    assert_eq!(ring["anim"]["listening"], true);
    assert_eq!(ring["anim"]["colors"][0], serde_json::json!([0, 180, 0]));
    assert_eq!(
        ring["anim"]["ttlSec"], 30,
        "官方 listening_anim 的 30s 死信"
    );
}

/// 旧固件（caps 无 `led_anim`）：官方退回静态 `leds` 帧（`set_leds(listening, True)`）。
#[tokio::test]
async fn old_firmware_gets_static_leds_frame_instead_of_anim() {
    let live = crate::device::echomuse::runtime::LiveControl::new(None, false);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    live.mic.bind(tx.clone());
    live.fx.bind_with_caps(tx, false);
    let (up, _up_rx) = tokio::sync::mpsc::unbounded_channel();
    start_turn(&live, &up, "action 键", false);
    let msgs = ctl_json(&mut rx);
    let ring = msgs.last().expect("旧固件也必须收到聆听反馈");
    assert_eq!(ring["type"], "leds");
    assert_eq!(ring["listening"], true);
    assert_eq!(
        ring["leds"].as_array().map(Vec::len),
        Some(12),
        "12 颗全绿（em_scenes._solid）"
    );
}

#[tokio::test]
async fn speech_and_no_speech_each_rearm_consecutive_turns_without_reply_eos() {
    let (live, mut rx) = bound_live();
    let (up, mut up_rx) = tokio::sync::mpsc::unbounded_channel();
    for reason in [
        UtteranceEnd::Speech,
        UtteranceEnd::NoSpeech,
        UtteranceEnd::Speech,
    ] {
        start_turn(&live, &up, "action 键", false);
        // 按键轮只发 start（无 detect）：见 button_turn_opens_listening_…
        assert!(matches!(up_rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains("start")));
        assert_eq!(
            ctl_types(&mut rx),
            vec![
                "speaker_flush",
                "duck",
                "mic_stop",
                "beam_lock",
                "mic_start",
                "led_anim"
            ],
            "开轮必须是 flush→duck→停→锁→起→环"
        );
        start_turn(&live, &up, "action 键", false);
        assert!(up_rx.try_recv().is_err());
        assert!(ctl_types(&mut rx).is_empty(), "活跃轮内重复触发不下发");
        assert!(live.turn.finish(reason, &up, |_| {}));
        assert!(
            matches!(up_rx.try_recv().unwrap(), IncomingFrame::EndOfUtterance(r) if r == reason)
        );
        assert!(!live
            .turn
            .finish(reason, &up, |_| panic!("duplicate sentinel ran tail")));
        assert!(up_rx.try_recv().is_err());
    }
}

#[tokio::test]
async fn old_control_gate_cannot_end_replacement_control_turn() {
    let old = crate::device::echomuse::runtime::LiveControl::new(None, false);
    let (live, _rx) = bound_live();
    let (up, mut up_rx) = tokio::sync::mpsc::unbounded_channel();
    start_turn(&live, &up, "action 键", false);
    up_rx.try_recv().unwrap(); // listen start
    old.turn
        .finish(UtteranceEnd::NoSpeech, &up, |_| panic!("old idle gate"));
    start_turn(&live, &up, "action 键", false);
    assert!(up_rx.try_recv().is_err(), "replacement remains listening");
    live.turn.finish(UtteranceEnd::Speech, &up, |_| {});
    assert!(matches!(
        up_rx.try_recv().unwrap(),
        IncomingFrame::EndOfUtterance(UtteranceEnd::Speech)
    ));
}

#[tokio::test]
async fn unbound_mic_cancels_gate_and_can_retry_after_bind() {
    let live = crate::device::echomuse::runtime::LiveControl::new(None, false);
    let (up, mut up_rx) = tokio::sync::mpsc::unbounded_channel();
    // 未绑定出站通道 = control 不可达：立即按 NoSpeech 收口，不留悬空聆听门。
    // 按键轮没有 detect，所以第一帧就是收口（此后不会再有 listen start）。
    start_turn(&live, &up, "action 键", false);
    assert!(matches!(
        up_rx.try_recv().unwrap(),
        IncomingFrame::EndOfUtterance(UtteranceEnd::NoSpeech)
    ));
    assert!(up_rx.try_recv().is_err(), "收口后不得补发 listen start");
    let (live, mut rx) = bound_live();
    start_turn(&live, &up, "action 键", false);
    assert!(matches!(up_rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains("start")));
    assert!(
        !ctl_types(&mut rx).is_empty(),
        "绑定后开轮要真正下发采集指令"
    );
}

#[tokio::test]
async fn closed_channel_cancels_gate() {
    let (live, ctl) = bound_live();
    let (up, mut up_rx) = tokio::sync::mpsc::unbounded_channel();
    drop(ctl); // 会话收尾：control 出站已关闭
    start_turn(&live, &up, "action 键", false);
    assert!(matches!(
        up_rx.try_recv().unwrap(),
        IncomingFrame::EndOfUtterance(UtteranceEnd::NoSpeech)
    ));
    assert!(up_rx.try_recv().is_err(), "断链后绝不发 listen start");
}
