//! ESPHome 命令 → xiaozhi 内部动作的**翻译缝**。
//!
//! 桥只说标准协议、不实现业务：可写实体（`number`）收到命令后，一律经本模块翻成
//! EchoMuse 的既有控制面消息（`ControlOut::Json` → `hub_ctl` → control_loop → 设备）。
//! 这样新增/修改设备动作只需改这里一行，不动协议层。
//!
//! ## 为什么只有音量可写
//!
//! EchoMuse 协议里**有下发路径的扬声器控制只有 `volume_set`**；静音只有设备上报的
//! `mute_state`，没有对应的 `mute_set`。因此静音在实体表里是只读 `binary_sensor`，
//! 不做"点了没反应"的假 `switch`。

use crate::device::echomuse::{hub_ctl, hub_live, hub_status, ControlOut};

/// 设备遥测快照（**每一项都可为 `None`**：设备尚未上报该字段）。
#[derive(Debug, Default, Clone, Copy)]
pub struct Telemetry {
    /// 扬声器档位（0..127 原始 tinymix 值）。
    pub volume: Option<f32>,
    /// 是否静音。
    pub muted: Option<bool>,
}

/// 当前被桥代理的设备 id。
///
/// Dot 2 是唯一设备，取 hub 里第一台即可；多台并存时只代理其中一台（其余需要多实例
/// 或多设备配置，当前不做——`docs/research/emx-integration-analysis.md` §7 不做清单）。
pub(super) fn active_device() -> Option<String> {
    let devices = hub_status();
    if devices.len() > 1 {
        tracing::debug!(
            "[esp_home] hub 有 {} 台设备，桥只代理第一台（{}）",
            devices.len(),
            devices[0].0
        );
    }
    devices.into_iter().next().map(|(id, _)| id)
}

/// Dot 2 是否在线（hub 有登记 = 与服务器保持着 control 连接）。
///
/// 注意与 `registry.sessions()` 的区别：那个是**语音会话**表，设备连着但没在对话时
/// 也为空。HA 用户看到的「在线」应当是设备连接态，故取 hub。
pub fn online() -> bool {
    !hub_status().is_empty()
}

/// 读设备遥测（音量 / 静音）。离线或尚未上报时对应项为 `None`。
pub fn telemetry() -> Telemetry {
    let Some(id) = active_device() else {
        return Telemetry::default();
    };
    let Some((_, live)) = hub_live(&id) else {
        return Telemetry::default();
    };
    let payload = live.payload(&id);
    Telemetry {
        volume: payload
            .get("volume_state")
            .and_then(|s| s.get("level"))
            .and_then(serde_json::Value::as_f64)
            .map(|v| v as f32),
        muted: payload
            .get("mute_state")
            .and_then(|s| s.get("muted"))
            .and_then(serde_json::Value::as_bool),
    }
}

/// 下发音量（0..127）。返回是否成功入队（设备在线且 control 通道可用）。
///
/// 调用方**不要**据此立即回推状态：设备会自己上报 `volume_state`，桥由探针读真实值
/// 再推给客户端。提前推会把 HA 的滑块弹回旧值。
pub fn set_volume(level: u8) -> bool {
    let level = level.min(127);
    let Some(id) = active_device() else {
        return false;
    };
    let Some(ctl) = hub_ctl(&id) else {
        return false;
    };
    let msg = serde_json::json!({ "type": "volume_set", "level": level }).to_string();
    match ctl.send(ControlOut::Json(msg)) {
        Ok(()) => {
            tracing::debug!("[esp_home] 已下发音量 {level} → {id}");
            true
        }
        Err(_) => {
            tracing::debug!("[esp_home] 音量下发失败（control 通道已关闭）device={id}");
            false
        }
    }
}

/// 把 ESPHome `number` 命令的浮点值转成设备的整数档位（四舍五入并夹到 0..127）。
///
/// 单独抽出便于测试：HA 可能发 `0.4` / `126.6` / 负数（滑块边界抖动）。
pub fn quantize_volume(value: f32) -> u8 {
    if !value.is_finite() {
        return 0;
    }
    value.round().clamp(0.0, 127.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_volume_rounds_and_clamps() {
        assert_eq!(quantize_volume(0.0), 0);
        assert_eq!(quantize_volume(0.4), 0);
        assert_eq!(quantize_volume(0.5), 1);
        assert_eq!(quantize_volume(63.6), 64);
        assert_eq!(quantize_volume(127.0), 127);
        // 边界外（HA 滑块抖动 / 异常值）一律夹紧，不 panic、不溢出。
        assert_eq!(quantize_volume(200.0), 127);
        assert_eq!(quantize_volume(-5.0), 0);
        assert_eq!(quantize_volume(f32::NAN), 0);
        assert_eq!(quantize_volume(f32::INFINITY), 0);
    }
}
