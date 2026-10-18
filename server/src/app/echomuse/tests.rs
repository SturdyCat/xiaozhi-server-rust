//! EchoMuse 模块测试：帧编解码、ack/config 形状、`EchomuseTransport` 双向翻译。
//!
//! 传输翻译用 channel 直驱（不起真实 WebSocket）：模拟 inbound 任务的输出直接推
//! 上行 channel，断言会话侧看到正确的 xiaozhi 语义帧；下行方向反向断言 `0x02/0x03`。

use tokio::sync::mpsc;

use super::frames::*;
use super::transport::EchomuseTransport;
use super::{ack_message, config_push, ControlOut};
use crate::app::transport::{IncomingFrame, Transport};

// ---- 帧编解码 ----

#[test]
fn mic_pcm_roundtrip() {
    let pcm = [0u8, 1, 2, 3, 4, 5, 6, 7];
    let mut frame = vec![FRAME_MIC];
    frame.extend_from_slice(&pcm);
    assert_eq!(parse_uplink(&frame), UplinkFrame::MicPcm(&pcm));
}

#[test]
fn session_audio_strips_header() {
    // 0x07 头：session u32 BE + seq u16 BE，其后才是 PCM。
    let pcm = [9u8, 8, 7];
    let mut frame = vec![FRAME_SESSION_AUDIO, 0, 0, 0, 42, 0, 7];
    frame.extend_from_slice(&pcm);
    assert_eq!(parse_uplink(&frame), UplinkFrame::MicPcm(&pcm));
}

#[test]
fn session_audio_truncated_is_unknown() {
    // 不足 6 字节头的 0x07 按未知忽略（不把头字节当 PCM）。
    assert_eq!(
        parse_uplink(&[FRAME_SESSION_AUDIO, 0, 0]),
        UplinkFrame::Unknown(FRAME_SESSION_AUDIO)
    );
}

#[test]
fn vad_sentinels() {
    assert_eq!(parse_uplink(&[FRAME_VAD_END]), UplinkFrame::VadEnd);
    assert_eq!(parse_uplink(&[FRAME_NO_SPEECH]), UplinkFrame::NoSpeech);
}

#[test]
fn unknown_and_empty_frames() {
    assert_eq!(parse_uplink(&[0x2A]), UplinkFrame::Unknown(0x2A));
    // 下行方向专属的音乐帧 0x04/0x05 出现在上行也是未知（方向命名空间）：
    // 0x04/0x05 在上行是哨兵，这里验证的是真正的未知值。
    assert_eq!(parse_uplink(&[0x99]), UplinkFrame::Unknown(0x99));
    assert_eq!(parse_uplink(&[]), UplinkFrame::Unknown(0));
}

#[test]
fn speaker_frames_shape() {
    assert_eq!(frame_speaker(&[1, 2, 3]), vec![FRAME_SPEAKER, 1, 2, 3]);
    assert_eq!(frame_speaker_eos(), vec![FRAME_SPEAKER_EOS]);
}

// ---- ack / config 形状（能力协商是兼容性契约） ----

#[test]
fn ack_declares_supported_subset_only() {
    let ack = ack_message("dev-1");
    let v: serde_json::Value = serde_json::from_str(&ack).unwrap();
    assert_eq!(v["type"], "ack");
    assert_eq!(v["device_id"], "dev-1");
    let feats = v["features"].as_array().unwrap();
    // 不声明 listen_session / audio_mix / ble_connect —— 服务器不支持的照协议静默降级。
    let names: Vec<&str> = feats.iter().filter_map(|f| f.as_str()).collect();
    assert!(names.contains(&"mic") && names.contains(&"speaker"));
    assert!(!names.contains(&"listen_session"));
    assert!(!names.contains(&"audio_mix"));
    assert!(!names.contains(&"ble_connect"));
}

#[test]
fn config_push_enables_on_device_wake() {
    let v: serde_json::Value = serde_json::from_str(&config_push()).unwrap();
    assert_eq!(v["type"], "config");
    assert_eq!(v["owwOnDevice"], true);
}

// ---- EchomuseTransport 翻译 ----

struct Harness {
    transport: EchomuseTransport,
    up_tx: mpsc::UnboundedSender<IncomingFrame>,
    ctl_rx: mpsc::UnboundedReceiver<ControlOut>,
    data_rx: mpsc::UnboundedReceiver<Vec<u8>>,
}

fn harness() -> Harness {
    let (up_tx, up_rx) = mpsc::unbounded_channel();
    let (ctl_tx, ctl_rx) = mpsc::unbounded_channel();
    let (data_tx, data_rx) = mpsc::unbounded_channel();
    Harness {
        transport: EchomuseTransport::new(up_rx, ctl_tx, data_tx).unwrap(),
        up_tx,
        ctl_rx,
        data_rx,
    }
}

#[tokio::test]
async fn recv_translates_uplink_events() {
    let mut h = harness();
    h.up_tx
        .send(IncomingFrame::Text(
            r#"{"type":"listen","state":"start"}"#.into(),
        ))
        .unwrap();
    h.up_tx.send(IncomingFrame::Binary(vec![1, 2, 3])).unwrap();
    drop(h.up_tx); // 上行 channel 关闭 = 断开

    assert!(matches!(
        h.transport.recv().await,
        IncomingFrame::Text(s) if s.contains("\"state\":\"start\"")
    ));
    assert!(matches!(h.transport.recv().await, IncomingFrame::Binary(b) if b == vec![1, 2, 3]));
    // 粘滞 Closed：channel 已关，之后恒报 Closed。
    assert!(matches!(h.transport.recv().await, IncomingFrame::Closed));
    assert!(matches!(
        h.transport.try_recv(),
        Some(IncomingFrame::Closed)
    ));
}

#[tokio::test]
async fn send_binary_produces_speaker_pcm_chunks() {
    let mut h = harness();
    // 一包 Opus 在 default（无 sherpa）构建下解码为空——仍应走通不发帧；
    // 真实解码路径由 `--features sherpa` 集成覆盖，这里验证通道与帧封装不断。
    h.transport.send_binary(b"\x01\x02".to_vec()).await.unwrap();
    // default 构建：解码空 → 无 0x02；data 面应有 0 条消息。
    assert!(h.data_rx.try_recv().is_err());
}

#[tokio::test]
async fn tts_stop_emits_eos() {
    let mut h = harness();
    h.transport
        .send_text(r#"{"type":"tts_stop","session_id":""}"#.into())
        .await
        .unwrap();
    // tts stop → 0x03 EOS（default 构建下无积压 PCM，直接一条 EOS）。
    let eos = h.data_rx.try_recv().expect("应发出 EOS");
    assert_eq!(eos, frame_speaker_eos());
    // 其余 JSON 不下发（stt/llm 等设备不消费）。
    h.transport
        .send_text(r#"{"type":"stt","text":"你好"}"#.into())
        .await
        .unwrap();
    assert!(h.data_rx.try_recv().is_err());
}

#[tokio::test]
async fn close_requests_shutdown_and_sticks() {
    let mut h = harness();
    h.transport.close().await;
    match h.ctl_rx.try_recv() {
        Ok(ControlOut::Shutdown) => {}
        other => panic!("close 应发 Shutdown，实际 {other:?}"),
    }
    assert!(matches!(h.transport.recv().await, IncomingFrame::Closed));
}

#[tokio::test]
async fn try_recv_empty_is_none_not_closed() {
    let mut h = harness();
    assert!(h.transport.try_recv().is_none());
}
