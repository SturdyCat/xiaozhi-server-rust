use super::*;
use crate::workflow::protocol::IncomingFrame;
use serde_json::json;
use std::sync::Mutex;
use tokio::sync::mpsc;

/// 构造一条最小会话（mic/fx 已绑出站口，start_turn 不致发送失败）。
///
/// 必须**把接收端交还调用方**：`start_turn` 的 `mic.stop()/beam_lock()/turn_stream()`
/// 是 `send()`，接收端若随函数结束 drop，发送立即失败 → 开轮按 NoSpeech 收口、
/// 一条 `listen start` 都不会投。以前 detect 在收口前就发了，掩盖了这个断链。
fn fixture() -> (
    super::super::runtime::LiveControl,
    mpsc::UnboundedReceiver<super::super::ControlOut>,
) {
    let live = super::super::runtime::LiveControl::new(None, false);
    let (ctl_tx, ctl_rx) = mpsc::unbounded_channel();
    live.mic.bind(ctl_tx.clone());
    live.fx.bind(ctl_tx);
    (live, ctl_rx)
}

/// 把 button JSON 喂给 handle_control_text，返回上行 channel 里是否出现了开轮 `listen start` 帧。
///
/// 按键轮**不发** `state=detect`（那是唤醒词专用的"叫醒我"信号，会话层收到才播
/// 「我在」），所以这里认 `start`；末尾那条也是 `start_turn` 一定投的帧。
fn fired_turn(live: &super::super::runtime::LiveControl, text: &str) -> bool {
    let (up_tx, mut up_rx) = mpsc::unbounded_channel();
    let ble = std::sync::Arc::new(Mutex::new(super::super::ble::BleLink::default()));
    handle_control_text(text, "d1", live, &up_tx, &ble, false);
    let mut found = false;
    while let Ok(frame) = up_rx.try_recv() {
        if let IncomingFrame::Text(t) = frame {
            if t.contains(r#""state":"start""#) {
                found = true;
            }
        }
    }
    found
}

#[test]
fn action_button_down_starts_turn() {
    let (live, _ctl) = fixture();
    assert!(
        fired_turn(&live, r#"{"type":"button","clickType":138,"down":true}"#),
        "action 键（138）按下应开轮"
    );
}

#[test]
fn action_button_release_does_not_start_turn() {
    let (live, _ctl) = fixture();
    assert!(
        !fired_turn(&live, r#"{"type":"button","clickType":138,"down":false}"#),
        "action 键松开（down=false）不应开轮"
    );
}

#[test]
fn volume_buttons_do_not_start_turn() {
    let (live, _ctl) = fixture();
    assert!(!fired_turn(
        &live,
        r#"{"type":"button","clickType":115,"down":true}"#
    ));
    assert!(!fired_turn(
        &live,
        r#"{"type":"button","clickType":114,"down":true}"#
    ));
}

#[test]
fn mute_button_does_not_start_turn() {
    let (live, _ctl) = fixture();
    assert!(
        !fired_turn(&live, r#"{"type":"button","clickType":113,"down":true}"#),
        "禁麦键（113）不应开轮"
    );
}

#[test]
fn action_button_while_muted_does_not_start_turn() {
    let (live, _ctl) = fixture();
    live.update(&json!({"type":"mute_state","muted":true}));
    assert!(live.is_muted(), "前置：设备应处于静音");
    assert!(
        !fired_turn(&live, r#"{"type":"button","clickType":138,"down":true}"#),
        "静音状态下 action 键不应开轮"
    );
}

#[test]
fn action_button_by_semantic_type_starts_turn() {
    let (live, _ctl) = fixture();
    assert!(
        fired_turn(
            &live,
            r#"{"type":"button","clickType":99,"down":true,"button":{"type":"action"}}"#
        ),
        "语义字段 button.type==action 也应开轮"
    );
}
