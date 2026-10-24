//! Dot2 按键识别：action 键（clickType=138 / button.type=="action"）应被识别，
//! 音量±（114/115）/ 禁麦（113）及其它键不应被识别为 action。

use serde_json::json;

use super::{
    is_action_button, Dot2Button, ACTION_CLICK_TYPE, MUTE_CLICK_TYPE, VOLUME_DOWN_CLICK_TYPE,
    VOLUME_UP_CLICK_TYPE,
};

#[test]
fn action_by_click_type_138() {
    assert!(is_action_button(
        &json!({"type":"button","clickType":138,"down":true})
    ));
}

#[test]
fn action_by_semantic_button_type() {
    // 数字不是 138 但语义字段是 action —— 仍应识别（不依赖数字取值）。
    assert!(is_action_button(
        &json!({"type":"button","clickType":99,"button":{"type":"action"}})
    ));
}

#[test]
fn volume_up_down_and_mute_are_not_action() {
    assert!(!is_action_button(
        &json!({"type":"button","clickType":VOLUME_UP_CLICK_TYPE})
    ));
    assert!(!is_action_button(
        &json!({"type":"button","clickType":VOLUME_DOWN_CLICK_TYPE})
    ));
    assert!(!is_action_button(
        &json!({"type":"button","clickType":MUTE_CLICK_TYPE})
    ));
    // 语义字段也不是 action
    assert!(!is_action_button(
        &json!({"type":"button","clickType":115,"button":{"type":"volume_up"}})
    ));
}

#[test]
fn unknown_click_type_is_not_action() {
    assert!(!is_action_button(&json!({"type":"button","clickType":42})));
    assert!(!is_action_button(&json!({"type":"button"}))); // 缺 clickType 也算非 action
}

#[test]
fn button_kind_mapping_matches_observed_values() {
    assert_eq!(
        Dot2Button::from_click_type(ACTION_CLICK_TYPE),
        Dot2Button::Action
    );
    assert_eq!(
        Dot2Button::from_click_type(VOLUME_UP_CLICK_TYPE),
        Dot2Button::VolumeUp
    );
    assert_eq!(
        Dot2Button::from_click_type(VOLUME_DOWN_CLICK_TYPE),
        Dot2Button::VolumeDown
    );
    assert_eq!(
        Dot2Button::from_click_type(MUTE_CLICK_TYPE),
        Dot2Button::Mute
    );
    assert_eq!(Dot2Button::from_click_type(999), Dot2Button::Other);
}
