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

/// 官方 `handle_button_event` 开头就是 `if down: return`——按下什么都不做，
/// 判定全在松开时（只有松开事件带 `heldMs` 真值）。
#[test]
fn action_button_down_does_not_start_turn() {
    let (live, _ctl) = fixture();
    assert!(
        !fired_turn(&live, r#"{"type":"button","clickType":138,"down":true}"#),
        "action 键按下不应开轮（官方 if down: return）"
    );
}

#[test]
fn action_button_tap_starts_turn() {
    let (live, _ctl) = fixture();
    assert!(
        fired_turn(
            &live,
            r#"{"type":"button","clickType":138,"down":false,"heldMs":180}"#
        ),
        "action 键轻点（heldMs<750）松开会开轮"
    );
}

/// 官方 `em_button.decide`：`held_ms >= BUTTON_HOLD_MS(750)` → HOLD，
/// 转成 Home Assistant 的 long 事件，不开语音轮。
#[test]
fn action_button_hold_does_not_start_turn() {
    let (live, _ctl) = fixture();
    assert!(
        !fired_turn(
            &live,
            r#"{"type":"button","clickType":138,"down":false,"heldMs":900}"#
        ),
        "长按（heldMs>=750）是 HA long 事件，不应开轮"
    );
}

/// 官方 `em_button.decide`：`turn_active → CANCEL`。一轮进行中再点一下是**取消**，
/// 不是静默忽略——这是"卡住能自己出来"的关键路径（缺了它，一轮没收到设备
/// 0x04/0x05 就永久占门，之后每次按键全部无效）。
#[test]
fn second_tap_cancels_active_turn() {
    let (live, _ctl) = fixture();
    assert!(
        fired_turn(
            &live,
            r#"{"type":"button","clickType":138,"down":false,"heldMs":180}"#
        ),
        "第一次轻点应开轮"
    );
    assert!(live.turn.is_active(), "前置：轮次应处于活跃");

    let (up_tx, mut up_rx) = mpsc::unbounded_channel();
    let ble = std::sync::Arc::new(Mutex::new(super::super::ble::BleLink::default()));
    handle_control_text(
        r#"{"type":"button","clickType":138,"down":false,"heldMs":180}"#,
        "d1",
        &live,
        &up_tx,
        &ble,
        false,
    );
    assert!(!live.turn.is_active(), "第二次轻点应取消当前轮");
    let mut saw_end = false;
    while let Ok(frame) = up_rx.try_recv() {
        if matches!(frame, IncomingFrame::EndOfUtterance(_)) {
            saw_end = true;
        }
    }
    assert!(saw_end, "取消应按无语音收口（EndOfUtterance）");
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
        !fired_turn(
            &live,
            r#"{"type":"button","clickType":138,"down":false,"heldMs":180}"#
        ),
        "静音状态下 action 键轻点不应开轮（官方 BLOCKED）"
    );
}

#[test]
fn action_button_by_semantic_type_starts_turn() {
    let (live, _ctl) = fixture();
    assert!(
        fired_turn(
            &live,
            r#"{"type":"button","clickType":99,"down":false,"heldMs":180,"button":{"type":"action"}}"#
        ),
        "语义字段 button.type==action 也应开轮"
    );
}
