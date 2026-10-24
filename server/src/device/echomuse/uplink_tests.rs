use super::super::turn::start_turn;
use super::*;
fn mic(samples: usize) -> Vec<u8> {
    let mut raw = vec![frames::FRAME_MIC];
    raw.extend((0..samples).flat_map(|_| 1000i16.to_le_bytes()));
    raw
}
/// 已绑定采集/灯效口的 LiveControl；接收端必须存活到测试结束
/// （drop 即关闭通道，send 会失败）。
fn bound() -> (
    crate::device::echomuse::runtime::LiveControl,
    tokio::sync::mpsc::UnboundedReceiver<crate::device::echomuse::ControlOut>,
) {
    let live = crate::device::echomuse::runtime::LiveControl::new(None, false);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    live.mic.bind(tx.clone());
    live.fx.bind(tx);
    (live, rx)
}
fn begin(
    live: &crate::device::echomuse::runtime::LiveControl,
    up: &UnboundedSender<IncomingFrame>,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<IncomingFrame>,
) {
    start_turn(live, up, "action 键");
    assert!(matches!(rx.try_recv().unwrap(), IncomingFrame::Text(_)));
    assert!(matches!(rx.try_recv().unwrap(), IncomingFrame::Text(_)));
}
#[tokio::test]
async fn native_partial_pcm_precedes_end_without_codec_padding_or_replay() {
    let ble = Arc::new(Mutex::new(BleLink::default()));
    let (up, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (live, _ctl_rx) = bound();
    let mut uplink = MicUplink::new(live.mic.clone(), live.fx.clone()).unwrap();
    begin(&live, &up, &mut rx);
    uplink.on_binary(&mic(123), &live.turn, &up, &ble, false);
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
    uplink.on_binary(&[frames::FRAME_VAD_END], &live.turn, &up, &ble, false);
    assert!(matches!(
        rx.try_recv().unwrap(),
        IncomingFrame::AudioBoundary(UtteranceEnd::Speech, _)
    ));
    assert!(
        audio.is_current(),
        "completed speech remains valid until consumed"
    );
    uplink.on_binary(&[frames::FRAME_VAD_END], &live.turn, &up, &ble, false);
    uplink.on_binary(&mic(42), &live.turn, &up, &ble, false);
    assert!(rx.try_recv().is_err());
}
#[tokio::test]
async fn no_speech_invalidates_queued_audio_then_next_capture_is_fresh() {
    let ble = Arc::new(Mutex::new(BleLink::default()));
    let (up, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (live, _ctl_rx) = bound();
    let mut uplink = MicUplink::new(live.mic.clone(), live.fx.clone()).unwrap();
    begin(&live, &up, &mut rx);
    uplink.on_binary(&mic(100), &live.turn, &up, &ble, false);
    let IncomingFrame::Audio(old) = rx.try_recv().unwrap() else {
        panic!()
    };
    uplink.on_binary(&[frames::FRAME_NO_SPEECH], &live.turn, &up, &ble, false);
    assert!(!old.is_current());
    assert!(matches!(
        rx.try_recv().unwrap(),
        IncomingFrame::AudioBoundary(UtteranceEnd::NoSpeech, _)
    ));
    begin(&live, &up, &mut rx);
    uplink.on_binary(&mic(640), &live.turn, &up, &ble, false);
    let IncomingFrame::Audio(new) = rx.try_recv().unwrap() else {
        panic!()
    };
    assert!(new.is_current());
    assert_ne!(old.epoch.unwrap().turn, new.epoch.unwrap().turn);
}
#[tokio::test]
async fn duration_overflow_cancels_capture_without_blocking_boundary_or_control() {
    let ble = Arc::new(Mutex::new(BleLink::default()));
    let (up, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (live, _ctl_rx) = bound();
    let mut uplink = MicUplink::new(live.mic.clone(), live.fx.clone()).unwrap();
    begin(&live, &up, &mut rx);
    uplink.on_binary(&mic(32000), &live.turn, &up, &ble, false); // exactly 2s reserved
    uplink.on_binary(&mic(1), &live.turn, &up, &ble, false); // overflow, no blocking wait
    let IncomingFrame::Audio(audio) = rx.try_recv().unwrap() else {
        panic!()
    };
    assert!(!audio.is_current(), "overflow cancels the WHOLE capture");
    assert!(matches!(
        rx.try_recv().unwrap(),
        IncomingFrame::EndOfUtterance(UtteranceEnd::NoSpeech)
    ));
    drop(audio);
    begin(&live, &up, &mut rx);
    uplink.on_binary(&mic(1), &live.turn, &up, &ble, false);
    assert!(matches!(rx.try_recv().unwrap(), IncomingFrame::Audio(_)));
}
#[tokio::test]
async fn data_disconnect_invalidates_speech_end_and_stale_reader() {
    let ble = Arc::new(Mutex::new(BleLink::default()));
    let (up, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (live, _ctl_rx) = bound();
    let mut uplink = MicUplink::new(live.mic.clone(), live.fx.clone()).unwrap();
    begin(&live, &up, &mut rx);
    uplink.on_binary(&mic(100), &live.turn, &up, &ble, false);
    uplink.on_binary(&[frames::FRAME_VAD_END], &live.turn, &up, &ble, false);
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
    begin(&live, &up, &mut rx);
    uplink.on_binary(&mic(100), &live.turn, &up, &ble, false);
    assert!(
        rx.try_recv().is_err(),
        "old data claim cannot enqueue into new epoch"
    );
}
#[test]
fn unsolicited_sentinels_and_unnegotiated_ble_are_ignored() {
    let ble = Arc::new(Mutex::new(BleLink::default()));
    let (up, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (live, _ctl_rx) = bound();
    let mut uplink = MicUplink::new(live.mic.clone(), live.fx.clone()).unwrap();
    for raw in [
        &[frames::FRAME_VAD_END][..],
        &[frames::FRAME_NO_SPEECH],
        &mic(10),
    ] {
        uplink.on_binary(raw, &live.turn, &up, &ble, false);
    }
    let mut raw = vec![frames::FRAME_BLE_ADVERTS];
    raw.extend_from_slice(br#"{"adverts":[{"addr":"aa:bb","rssi":-40}]}"#);
    uplink.on_binary(&raw, &live.turn, &up, &ble, false);
    assert!(rx.try_recv().is_err());
    assert!(ble.lock().unwrap().adverts_snapshot().1.is_empty());
}
