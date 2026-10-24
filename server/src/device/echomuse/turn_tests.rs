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
    let mut out = Vec::new();
    while let Some(ControlOut::Json(json)) = rx.try_recv().ok() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
            out.push(v["type"].as_str().unwrap_or("").to_string());
        }
    }
    out
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
        start_turn(&live, &up, "action 键");
        // detect 的 text 携带触发来源标签（会话日志据此区分唤醒词 vs 按键）
        assert!(
            matches!(up_rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains(r#""state":"detect""#) && t.contains("action 键"))
        );
        assert!(matches!(up_rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains("start")));
        // 官方开轮六连：打断 flush + 音乐 duck + 停流 + 锁波束 + turn 流 + 聆听环
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
            "开轮必须是 flush→duck→停→锁→起→环→提示音"
        );
        start_turn(&live, &up, "action 键");
        assert!(up_rx.try_recv().is_err());
        assert!(ctl_types(&mut rx).is_empty(), "活跃轮内重复触发不下发");
        assert!(live.turn.finish(reason, &up, |_| {}));
        assert!(matches!(up_rx.try_recv().unwrap(), IncomingFrame::EndOfUtterance(r) if r == reason));
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
    start_turn(&live, &up, "action 键");
    up_rx.try_recv().unwrap();
    up_rx.try_recv().unwrap();
    old.turn
        .finish(UtteranceEnd::NoSpeech, &up, |_| panic!("old idle gate"));
    start_turn(&live, &up, "action 键");
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
    start_turn(&live, &up, "action 键");
    assert!(matches!(up_rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains("detect")));
    assert!(matches!(
        up_rx.try_recv().unwrap(),
        IncomingFrame::EndOfUtterance(UtteranceEnd::NoSpeech)
    ));
    let (live, mut rx) = bound_live();
    start_turn(&live, &up, "action 键");
    assert!(matches!(up_rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains("detect")));
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
    start_turn(&live, &up, "action 键");
    assert!(matches!(up_rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains("detect")));
    assert!(matches!(
        up_rx.try_recv().unwrap(),
        IncomingFrame::EndOfUtterance(UtteranceEnd::NoSpeech)
    ));
    assert!(up_rx.try_recv().is_err(), "断链后绝不发 listen start");
}
