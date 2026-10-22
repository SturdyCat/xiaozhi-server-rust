use super::super::turn::start_turn;
use super::*;
fn mic(samples: usize) -> Vec<u8> {
    let mut raw = vec![frames::FRAME_MIC];
    raw.extend((0..samples).flat_map(|_| 1000i16.to_le_bytes()));
    raw
}
async fn begin(
    turn: &BoundedTurn,
    up: &UnboundedSender<IncomingFrame>,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<IncomingFrame>,
) {
    start_turn(
        turn,
        &mut futures_util::sink::drain::<axum::extract::ws::Message>(),
        up,
        "action 键",
    )
    .await
    .unwrap();
    assert!(matches!(rx.try_recv().unwrap(), IncomingFrame::Text(_)));
    assert!(matches!(rx.try_recv().unwrap(), IncomingFrame::Text(_)));
}
#[tokio::test]
async fn native_partial_pcm_precedes_end_without_codec_padding_or_replay() {
    let turn = BoundedTurn::default();
    let ble = Arc::new(Mutex::new(BleLink::default()));
    let (up, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut uplink = MicUplink::new().unwrap();
    begin(&turn, &up, &mut rx).await;
    uplink.on_binary(&mic(123), &turn, &up, &ble, false);
    let IncomingFrame::Audio(audio) = rx.try_recv().unwrap() else {
        panic!("native PCM expected")
    };
    assert_eq!(audio.data.len(), 246);
    assert_eq!(audio.format, AudioFormat::echo_mic());
    assert_eq!(
        frames::MIC_SAMPLE_RATE,
        audio.format.sample_rate,
        "上行格式必须与协议常量同源"
    );
    assert!(audio.is_current());
    uplink.on_binary(&[frames::FRAME_VAD_END], &turn, &up, &ble, false);
    assert!(matches!(
        rx.try_recv().unwrap(),
        IncomingFrame::AudioBoundary(UtteranceEnd::Speech, _)
    ));
    assert!(
        audio.is_current(),
        "completed speech remains valid until consumed"
    );
    uplink.on_binary(&[frames::FRAME_VAD_END], &turn, &up, &ble, false);
    uplink.on_binary(&mic(42), &turn, &up, &ble, false);
    assert!(rx.try_recv().is_err());
}
#[tokio::test]
async fn no_speech_invalidates_queued_audio_then_next_capture_is_fresh() {
    let turn = BoundedTurn::default();
    let ble = Arc::new(Mutex::new(BleLink::default()));
    let (up, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut uplink = MicUplink::new().unwrap();
    begin(&turn, &up, &mut rx).await;
    uplink.on_binary(&mic(100), &turn, &up, &ble, false);
    let IncomingFrame::Audio(old) = rx.try_recv().unwrap() else {
        panic!()
    };
    uplink.on_binary(&[frames::FRAME_NO_SPEECH], &turn, &up, &ble, false);
    assert!(!old.is_current());
    assert!(matches!(
        rx.try_recv().unwrap(),
        IncomingFrame::AudioBoundary(UtteranceEnd::NoSpeech, _)
    ));
    begin(&turn, &up, &mut rx).await;
    uplink.on_binary(&mic(640), &turn, &up, &ble, false);
    let IncomingFrame::Audio(new) = rx.try_recv().unwrap() else {
        panic!()
    };
    assert!(new.is_current());
    assert_ne!(old.epoch.unwrap().turn, new.epoch.unwrap().turn);
}
#[tokio::test]
async fn duration_overflow_cancels_capture_without_blocking_boundary_or_control() {
    let turn = BoundedTurn::default();
    let ble = Arc::new(Mutex::new(BleLink::default()));
    let (up, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut uplink = MicUplink::new().unwrap();
    begin(&turn, &up, &mut rx).await;
    uplink.on_binary(&mic(32000), &turn, &up, &ble, false); // exactly 2s reserved
    uplink.on_binary(&mic(1), &turn, &up, &ble, false); // overflow, no blocking wait
    let IncomingFrame::Audio(audio) = rx.try_recv().unwrap() else {
        panic!()
    };
    assert!(!audio.is_current(), "overflow cancels the WHOLE capture");
    assert!(matches!(
        rx.try_recv().unwrap(),
        IncomingFrame::EndOfUtterance(UtteranceEnd::NoSpeech)
    ));
    drop(audio);
    begin(&turn, &up, &mut rx).await;
    uplink.on_binary(&mic(1), &turn, &up, &ble, false);
    assert!(matches!(rx.try_recv().unwrap(), IncomingFrame::Audio(_)));
}
#[tokio::test]
async fn data_disconnect_invalidates_speech_end_and_stale_reader() {
    let turn = BoundedTurn::default();
    let ble = Arc::new(Mutex::new(BleLink::default()));
    let (up, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut uplink = MicUplink::new().unwrap();
    begin(&turn, &up, &mut rx).await;
    uplink.on_binary(&mic(100), &turn, &up, &ble, false);
    uplink.on_binary(&[frames::FRAME_VAD_END], &turn, &up, &ble, false);
    let old_audio = rx.try_recv().unwrap();
    let end = rx.try_recv().unwrap();
    uplink.media.disconnect();
    uplink.media.claim();
    let IncomingFrame::Audio(audio) = old_audio else {
        panic!()
    };
    assert!(!audio.is_current());
    let IncomingFrame::AudioBoundary(_, epoch) = end else {
        panic!()
    };
    assert!(!epoch.is_current());
    begin(&turn, &up, &mut rx).await;
    uplink.on_binary(&mic(100), &turn, &up, &ble, false);
    assert!(
        rx.try_recv().is_err(),
        "old data claim cannot enqueue into new epoch"
    );
}
#[test]
fn unsolicited_sentinels_and_unnegotiated_ble_are_ignored() {
    let turn = BoundedTurn::default();
    let ble = Arc::new(Mutex::new(BleLink::default()));
    let (up, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut uplink = MicUplink::new().unwrap();
    for raw in [
        &[frames::FRAME_VAD_END][..],
        &[frames::FRAME_NO_SPEECH],
        &mic(10),
    ] {
        uplink.on_binary(raw, &turn, &up, &ble, false);
    }
    let mut raw = vec![frames::FRAME_BLE_ADVERTS];
    raw.extend_from_slice(br#"{"adverts":[{"addr":"aa:bb","rssi":-40}]}"#);
    uplink.on_binary(&raw, &turn, &up, &ble, false);
    assert!(rx.try_recv().is_err());
    assert!(ble.lock().unwrap().adverts_snapshot().1.is_empty());
}
