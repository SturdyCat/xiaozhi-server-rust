//! Dot2（biscuit 板 / emOS）专属适配：把官方固件 / 控制器的 Dot2 特定行为集中在此，
//! 与通用 echomuse 流水线解耦。
//!
//! 依据上游 `device/internal/client/control.go` 的 `SendButton` 与
//! `device/pkg/buttons` 的按键映射，以及架构文档（SETUP.md）整理。
//!
//! ## 按键线格式
//! 设备对 action 键、音量±、禁麦**都发 `type:"button"`**，仅靠 `clickType` 区分；
//! 新固件恒带语义字段 `button.type`（旧固件缺省）。控制器侧必须只认 action 键开语音轮——
//! 音量/禁麦由设备本地处理（ADC 静音 / 调音量，并分别上报 `mute_state` / `volume_state`），
//! 不应为其开轮（上游 `em_button.decide` 同语义：控制器只拒绝语音轮，不拦设备本地行为）。
//!
//! ## 静音主权
//! 禁麦是设备主权：设备做 ADC 静音 + 拒绝任何 `mic_start`，控制器侧再双保险拒绝开轮。
//! 设备侧才是硬保证（架构文档原话）。

use serde_json::Value;

/// action 键（按下说话 / 唤醒兜底入口）的 clickType。实测 = 138；语义字段
/// `button.type=="action"` 优先，本常量作数字兜底。
pub(crate) const ACTION_CLICK_TYPE: i64 = 138;
/// 音量 +（设备本地调音量，上报 `volume_state`）。实测 = 115。
pub(crate) const VOLUME_UP_CLICK_TYPE: i64 = 115;
/// 音量 −（设备本地调音量，上报 `volume_state`）。实测 = 114。
pub(crate) const VOLUME_DOWN_CLICK_TYPE: i64 = 114;
/// 禁麦（设备本地 ADC 静音，上报 `mute_state`）。实测 = 113。
pub(crate) const MUTE_CLICK_TYPE: i64 = 113;

/// 一条 `button` 事件是否代表 action 键（唯一应开语音轮的键）。
///
/// 双重保险：优先认语义字段 `button.type=="action"`（新固件恒发、不依赖数字取值），
/// 退而认数字 `clickType==ACTION_CLICK_TYPE`。音量±（114/115）、禁麦（113）的
/// `button.type` 不会是 `action`，数字也非 138，故不会误开轮。
pub(crate) fn is_action_button(v: &Value) -> bool {
    if v.get("button")
        .and_then(|b| b.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|t| t.eq_ignore_ascii_case("action"))
    {
        return true;
    }
    v.get("clickType").and_then(Value::as_i64) == Some(ACTION_CLICK_TYPE)
}

/// Dot2 物理按键类型（用于日志 / 未来扩展，如禁麦键的控制器侧行为）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dot2Button {
    /// 唯一应开语音轮的键。
    Action,
    VolumeUp,
    VolumeDown,
    /// 设备本地 ADC 静音；控制器侧双保险拒绝语音轮。
    Mute,
    /// 未知 / 固件私有键——控制器一律忽略。
    Other,
}

impl Dot2Button {
    /// 由 `clickType` 反查按键类型（语义字段缺省时的兜底日志用）。
    pub(crate) fn from_click_type(click_type: i64) -> Self {
        match click_type {
            ACTION_CLICK_TYPE => Dot2Button::Action,
            VOLUME_UP_CLICK_TYPE => Dot2Button::VolumeUp,
            VOLUME_DOWN_CLICK_TYPE => Dot2Button::VolumeDown,
            MUTE_CLICK_TYPE => Dot2Button::Mute,
            _ => Dot2Button::Other,
        }
    }
}

#[cfg(test)]
#[path = "dot2_tests.rs"]
mod tests;
