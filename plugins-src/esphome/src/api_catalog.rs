//! S3r：ESPHome Native API **服务端面消息**（实体目录 / 状态 / 控制应答）protobuf 编解码。
//!
//! 规格权威 = `esphome-native-api` 3.0.0 默认版本 `proto::version_2026_6_2`
//! 的 prost 派生：字段**声明序（= tag 升序）**编码、**默认值跳过**（0 / 0.0 /
//! false / 空串不写 wire）、float = fixed32（IEEE-754 位模式 LE）、enum = varint。
//!
//! 覆盖面 = native `esp_home` 桥实际使用的消息全集（服务端角色）：
//! - 编码：`HelloResponse` / `PingResponse` / `ListEntitiesBinarySensorResponse` /
//!   `ListEntitiesNumberResponse` / `ListEntitiesDoneResponse` /
//!   `BinarySensorStateResponse` / `NumberStateResponse`；
//! - 解码（客户端 → 服务端）：`HelloRequest` / `NumberCommandRequest` /
//!   `SwitchCommandRequest`（空载荷请求 `ListEntitiesRequest` /
//!   `SubscribeStatesRequest` / `PingRequest` / `DisconnectRequest` 由帧层直接识别）。
//!
//! 宿主侧对拍：`plugins/wasm/tests/esphome_frame.rs` 用参考 crate 的 prost 编码
//! （`proto_to_vec`）与公开解析器（`parse_proto_message`）双向逐字节/逐字段比对。

use crate::api_frame::{
    pb_fixed32_field, pb_float_field, pb_len_field, pb_varint_field, varint_decode, WIRE_FIXED32,
    WIRE_LEN, WIRE_VARINT,
};

// ------------------------------------------------------------------
// 服务端 → 客户端：消息载荷编码
// ------------------------------------------------------------------

/// `HelloResponse`：api_version_major(1) / minor(2) varint，server_info(3) /
/// name(4) string。
pub(crate) fn hello_response_payload(
    api_version_major: u32,
    api_version_minor: u32,
    server_info: &str,
    name: &str,
) -> Vec<u8> {
    let mut out = Vec::new();
    pb_varint_field(1, u64::from(api_version_major), &mut out);
    pb_varint_field(2, u64::from(api_version_minor), &mut out);
    pb_len_field(3, server_info.as_bytes(), &mut out);
    pb_len_field(4, name.as_bytes(), &mut out);
    out
}

/// `DeviceInfoResponse` 身份面（S3u：设备会话的 Hello/DeviceInfo 应答）。
///
/// 只编码 native `esp_home` 桥实际设置并有客户端消费的字段（prost 默认值跳过，
/// 未列字段走默认）：name(2) / mac_address(3) / esphome_version(4) / model(6) /
/// legacy_bluetooth_proxy_version(11，deprecated 但旧客户端读) / manufacturer(12) /
/// friendly_name(13) / bluetooth_proxy_feature_flags(15) /
/// suggested_area(16) / api_encryption_supported(19)。
#[allow(clippy::too_many_arguments)]
pub(crate) fn device_info_response_payload(
    name: &str,
    mac_address: &str,
    esphome_version: &str,
    model: &str,
    manufacturer: &str,
    friendly_name: &str,
    suggested_area: &str,
    api_encryption_supported: bool,
    legacy_bluetooth_proxy_version: u32,
    bluetooth_proxy_feature_flags: u32,
) -> Vec<u8> {
    let mut out = Vec::new();
    pb_len_field(2, name.as_bytes(), &mut out);
    pb_len_field(3, mac_address.as_bytes(), &mut out);
    pb_len_field(4, esphome_version.as_bytes(), &mut out);
    pb_len_field(6, model.as_bytes(), &mut out);
    pb_varint_field(11, u64::from(legacy_bluetooth_proxy_version), &mut out);
    pb_len_field(12, manufacturer.as_bytes(), &mut out);
    pb_len_field(13, friendly_name.as_bytes(), &mut out);
    pb_varint_field(15, u64::from(bluetooth_proxy_feature_flags), &mut out);
    pb_len_field(16, suggested_area.as_bytes(), &mut out);
    if api_encryption_supported {
        pb_varint_field(19, 1, &mut out);
    }
    out
}

/// `ListEntitiesBinarySensorResponse`：object_id(1) string、key(2) **fixed32**、
/// name(3) string、device_class(5) string；其余字段（status 标记/icon/分类/
/// device_id）走默认值跳过（桥不发）。
pub(crate) fn list_entities_binary_sensor_payload(
    object_id: &str,
    key: u32,
    name: &str,
    device_class: &str,
) -> Vec<u8> {
    let mut out = Vec::new();
    pb_len_field(1, object_id.as_bytes(), &mut out);
    pb_fixed32_field(2, key, &mut out);
    pb_len_field(3, name.as_bytes(), &mut out);
    pb_len_field(5, device_class.as_bytes(), &mut out);
    out
}

/// `ListEntitiesNumberResponse`：object_id(1)、key(2) fixed32、name(3)、
/// min_value(6)/max_value(7)/step(8) float、unit_of_measurement(11) string、
/// mode(12) enum varint；icon(5) 走默认跳过。
pub(crate) fn list_entities_number_payload(
    object_id: &str,
    key: u32,
    name: &str,
    min_value: f32,
    max_value: f32,
    step: f32,
    unit_of_measurement: &str,
    mode: u32,
) -> Vec<u8> {
    let mut out = Vec::new();
    pb_len_field(1, object_id.as_bytes(), &mut out);
    pb_fixed32_field(2, key, &mut out);
    pb_len_field(3, name.as_bytes(), &mut out);
    pb_float_field(6, min_value, &mut out);
    pb_float_field(7, max_value, &mut out);
    pb_float_field(8, step, &mut out);
    pb_len_field(11, unit_of_measurement.as_bytes(), &mut out);
    pb_varint_field(12, u64::from(mode), &mut out);
    out
}

/// `BinarySensorStateResponse`：key(1) fixed32、state(2) bool、missing_state(3)
/// bool、device_id(4) varint。
pub(crate) fn binary_sensor_state_payload(
    key: u32,
    state: bool,
    missing_state: bool,
    device_id: u32,
) -> Vec<u8> {
    let mut out = Vec::new();
    pb_fixed32_field(1, key, &mut out);
    pb_varint_field(2, u64::from(state), &mut out);
    pb_varint_field(3, u64::from(missing_state), &mut out);
    pb_varint_field(4, u64::from(device_id), &mut out);
    out
}

/// `NumberStateResponse`：key(1) fixed32、state(2) float、missing_state(3) bool、
/// device_id(4) varint。
pub(crate) fn number_state_payload(
    key: u32,
    state: f32,
    missing_state: bool,
    device_id: u32,
) -> Vec<u8> {
    let mut out = Vec::new();
    pb_fixed32_field(1, key, &mut out);
    pb_float_field(2, state, &mut out);
    pb_varint_field(3, u64::from(missing_state), &mut out);
    pb_varint_field(4, u64::from(device_id), &mut out);
    out
}

// ------------------------------------------------------------------
// 客户端 → 服务端：消息载荷解码（未知字段按 wire type 跳过）
// ------------------------------------------------------------------

/// protobuf 单字段（走读用；未知 tag 由调用方忽略）。
enum Field<'a> {
    Varint(u32, u64),
    Len(u32, &'a [u8]),
    Fixed32(u32, u32),
}

/// 取下一个字段；载荷耗尽返回 `Ok(None)`。坏 wire（长度越界/未知 wire type）
/// 显式报错（fail-closed，不猜测恢复）。
fn next_field<'a>(rest: &mut &'a [u8]) -> Result<Option<Field<'a>>, String> {
    loop {
        if rest.is_empty() {
            return Ok(None);
        }
        let (key, n) = varint_decode(rest, 5)?;
        *rest = &rest[n..];
        let wire_type = (key & 0x7) as u32;
        let tag = (key >> 3) as u32;
        match wire_type {
            WIRE_VARINT => {
                let (v, n) = varint_decode(rest, 10)?;
                *rest = &rest[n..];
                return Ok(Some(Field::Varint(tag, v)));
            }
            WIRE_LEN => {
                let (len, n) = varint_decode(rest, 5)?;
                *rest = &rest[n..];
                let len = len as usize;
                if rest.len() < len {
                    return Err("长度字段的载荷不完整".to_string());
                }
                let bytes = &rest[..len];
                *rest = &rest[len..];
                return Ok(Some(Field::Len(tag, bytes)));
            }
            WIRE_FIXED32 => {
                if rest.len() < 4 {
                    return Err("fixed32 字段不完整".to_string());
                }
                let v = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
                *rest = &rest[4..];
                return Ok(Some(Field::Fixed32(tag, v)));
            }
            1 => {
                // fixed64：本消息面无该 wire 类型，跳过继续（向后兼容）。
                if rest.len() < 8 {
                    return Err("fixed64 字段不完整".to_string());
                }
                *rest = &rest[8..];
            }
            other => return Err(format!("未知 wire type：{other}")),
        }
    }
}

fn f32_of_bits(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// `HelloRequest`：client_info(1) string、api_version_major(2) / minor(3) varint。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HelloRequestInfo {
    pub client_info: String,
    pub api_version_major: u32,
    pub api_version_minor: u32,
}

pub(crate) fn hello_request_decode(payload: &[u8]) -> Result<HelloRequestInfo, String> {
    let mut info = HelloRequestInfo {
        client_info: String::new(),
        api_version_major: 0,
        api_version_minor: 0,
    };
    let mut rest = payload;
    while let Some(field) = next_field(&mut rest)? {
        match field {
            Field::Len(1, bytes) => info.client_info = String::from_utf8_lossy(bytes).into_owned(),
            Field::Varint(2, v) => info.api_version_major = v as u32,
            Field::Varint(3, v) => info.api_version_minor = v as u32,
            Field::Varint(_, _) | Field::Len(_, _) | Field::Fixed32(_, _) => {}
        }
    }
    Ok(info)
}

/// `NumberCommandRequest` 载荷（客户端 → 服务端；与服务端对称的编码面）。
pub(crate) fn number_command_payload(key: u32, state: f32, device_id: u32) -> Vec<u8> {
    let mut out = Vec::new();
    pb_fixed32_field(1, key, &mut out);
    pb_float_field(2, state, &mut out);
    pb_varint_field(3, u64::from(device_id), &mut out);
    out
}

/// `NumberCommandRequest`：key(1) fixed32、state(2) float、device_id(3) varint。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NumberCommand {
    pub key: u32,
    pub state: f32,
    pub device_id: u32,
}

pub(crate) fn number_command_decode(payload: &[u8]) -> Result<NumberCommand, String> {
    let mut command = NumberCommand {
        key: 0,
        state: 0.0,
        device_id: 0,
    };
    let mut rest = payload;
    while let Some(field) = next_field(&mut rest)? {
        match field {
            Field::Fixed32(1, bits) => command.key = bits,
            Field::Fixed32(2, bits) => command.state = f32_of_bits(bits),
            Field::Varint(3, v) => command.device_id = v as u32,
            Field::Varint(_, _) | Field::Len(_, _) | Field::Fixed32(_, _) => {}
        }
    }
    Ok(command)
}

/// `SwitchCommandRequest`：key(1) fixed32、state(2) bool、device_id(3) varint。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SwitchCommand {
    pub key: u32,
    pub state: bool,
    pub device_id: u32,
}

pub(crate) fn switch_command_decode(payload: &[u8]) -> Result<SwitchCommand, String> {
    let mut command = SwitchCommand {
        key: 0,
        state: false,
        device_id: 0,
    };
    let mut rest = payload;
    while let Some(field) = next_field(&mut rest)? {
        match field {
            Field::Fixed32(1, bits) => command.key = bits,
            Field::Varint(2, v) => command.state = v != 0,
            Field::Varint(3, v) => command.device_id = v as u32,
            Field::Varint(_, _) | Field::Len(_, _) | Field::Fixed32(_, _) => {}
        }
    }
    Ok(command)
}
