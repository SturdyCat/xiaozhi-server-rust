use super::*;

#[tokio::test]
async fn speech_and_no_speech_each_rearm_consecutive_turns_without_reply_eos() {
    let turn = BoundedTurn::default();
    let (up, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut sink = futures_util::sink::drain::<Message>();
    for reason in [
        UtteranceEnd::Speech,
        UtteranceEnd::NoSpeech,
        UtteranceEnd::Speech,
    ] {
        start_turn(&turn, &mut sink, &up, "action 键")
            .await
            .unwrap();
        // detect 的 text 携带触发来源标签（会话日志据此区分唤醒词 vs 按键）
        assert!(
            matches!(rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains(r#""state":"detect""#) && t.contains("action 键"))
        );
        assert!(matches!(rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains("start")));
        start_turn(&turn, &mut sink, &up, "action 键")
            .await
            .unwrap();
        assert!(rx.try_recv().is_err());
        turn.finish(reason, &up, |_| {});
        assert!(matches!(rx.try_recv().unwrap(), IncomingFrame::EndOfUtterance(r) if r == reason));
        turn.finish(reason, &up, |_| panic!("duplicate sentinel ran tail"));
        assert!(rx.try_recv().is_err());
    }
}

#[tokio::test]
async fn old_control_gate_cannot_end_replacement_control_turn() {
    let old = BoundedTurn::default();
    let new = BoundedTurn::default();
    let (up, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut sink = futures_util::sink::drain::<Message>();
    start_turn(&new, &mut sink, &up, "action 键").await.unwrap();
    rx.try_recv().unwrap();
    rx.try_recv().unwrap();
    old.finish(UtteranceEnd::NoSpeech, &up, |_| panic!("old idle gate"));
    start_turn(&new, &mut sink, &up, "action 键").await.unwrap();
    assert!(rx.try_recv().is_err(), "replacement remains listening");
    new.finish(UtteranceEnd::Speech, &up, |_| {});
    assert!(matches!(
        rx.try_recv().unwrap(),
        IncomingFrame::EndOfUtterance(UtteranceEnd::Speech)
    ));
}

#[tokio::test]
async fn failed_mic_start_cancels_gate_and_can_retry() {
    let turn = BoundedTurn::default();
    let (up, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut failing = Box::pin(futures_util::sink::unfold((), |(), _: Message| async {
        Err::<(), _>(std::io::Error::other("closed"))
    }));
    assert!(start_turn(&turn, &mut failing, &up, "action 键")
        .await
        .is_err());
    assert!(matches!(rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains("detect")));
    assert!(matches!(
        rx.try_recv().unwrap(),
        IncomingFrame::EndOfUtterance(UtteranceEnd::NoSpeech)
    ));
    let mut sink = futures_util::sink::drain::<Message>();
    start_turn(&turn, &mut sink, &up, "action 键")
        .await
        .unwrap();
    assert!(matches!(rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains("detect")));
}

#[tokio::test]
async fn sentinel_during_awaited_mic_start_does_not_emit_stale_listen_start() {
    let turn = std::sync::Arc::new(BoundedTurn::default());
    let (up, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let finish_turn = turn.clone();
    let finish_up = up.clone();
    let mut sink = Box::pin(futures_util::sink::unfold((), move |(), _: Message| {
        let turn = finish_turn.clone();
        let up = finish_up.clone();
        async move {
            turn.finish(UtteranceEnd::Speech, &up, |_| {});
            Ok::<(), std::convert::Infallible>(())
        }
    }));
    start_turn(&turn, &mut sink, &up, "action 键")
        .await
        .unwrap();
    assert!(matches!(rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains("detect")));
    assert!(matches!(
        rx.try_recv().unwrap(),
        IncomingFrame::EndOfUtterance(UtteranceEnd::Speech)
    ));
    assert!(rx.try_recv().is_err(), "no start after end");
    let mut ready_sink = futures_util::sink::drain::<Message>();
    start_turn(&turn, &mut ready_sink, &up, "action 键")
        .await
        .unwrap();
    assert!(matches!(rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains("detect")));
    assert!(matches!(rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains("start")));
}
