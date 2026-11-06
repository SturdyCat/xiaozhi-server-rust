//! 无 I/O 的设备公共 helper。

/// 设备音量公共范围下界。
pub const VOLUME_MIN: f32 = 0.0;
/// EchoMuse/ESPHome 当前公共音量范围上界。
pub const VOLUME_MAX: f32 = 127.0;

/// 将管理面/ESPHome 的浮点音量值量化为设备档位。
///
/// 非有限值安全归零，其他值四舍五入并夹在公共设备范围内。adapter 只负责
/// 发送结果，不应各自复制这段边界逻辑。
pub fn quantize_volume(value: f32) -> u8 {
    if !value.is_finite() {
        return VOLUME_MIN as u8;
    }
    value.round().clamp(VOLUME_MIN, VOLUME_MAX) as u8
}

/// 解析设备协议使用的整数音量档位；拒绝浮点、负数、超界和缺失值。
pub fn parse_volume_level(body: &serde_json::Value) -> Option<u8> {
    body.get("level")
        .and_then(serde_json::Value::as_u64)
        .filter(|value| *value <= VOLUME_MAX as u64)
        .map(|value| value as u8)
}
