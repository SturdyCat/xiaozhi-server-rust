//! S3k：ESPHome Native API **明文帧编解码 + 核心消息 protobuf 编解码**（guest 单源）。
//!
//! 规格权威 = 外部 crate `esphome-native-api`（当前 native ESPHome 栈所有者）：
//! - `frame.rs::FrameCodec` 明文分支：`0x00 || varint(payload_len) || type(1B)
//!   || protobuf payload`——**wire 标记 = 载荷长度（不含类型字节）**；标记最长
//!   4 字节（>4 显式报错）。注意参考实现的 asymmetry：编码器写 `item.len()-1`，
//!   解码器读 `varint + 1` 还原（本文件两侧各按其口径对齐，对拍实证）。
//!   加密帧以 `0x01` 开头（u16 BE 长度 + noise），不在本文件范围，遇到显式报错；
//! - `parser.rs`：消息类型 id ↔ 消息名（1=HelloRequest … 123=…，本文件收录桥
//!   策略面相关的核心子集）；
//! - `proto/*.rs` prost 派生：字段**按声明序编码**、**默认值跳过**（0 / false /
//!   空串不写 wire）。
//!
//! 宿主侧对拍：`plugins/wasm/tests/esphome_frame.rs` 用 **esphome-native-api
//! 真连接**（`EspHomeApi::start` over 内存 duplex）双向验证——guest 帧被真实
//! 握手消费、真连接发出的帧被 guest 解码，PingResponse 帧逐字节对拍。

/// 与参考实现同口径的声明长度上限（8MB）。
const MAX_FRAME_LEN: usize = 8 * 1024 * 1024;
/// guest 事件输入解码上限（16KB；结果 hex ×2 不超 kv 64KB 上限）。
pub(crate) const INPUT_CAP: usize = 16 * 1024;
/// 帧长度标记 varint 最长字节数（frame.rs 的 `> 4` 守卫同口径）。
const FRAME_MARKER_MAX: usize = 4;

// ------------------------------------------------------------------
// varint（LEB128）
// ------------------------------------------------------------------

pub(crate) fn varint_encode(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

/// 解码 varint，返回 (值, 消耗字节数)；`max` 为最长字节数守卫。
pub(crate) fn varint_decode(data: &[u8], max: usize) -> Result<(u64, usize), String> {
    let mut value: u64 = 0;
    let mut shift = 0u32;
    for (i, &byte) in data.iter().enumerate() {
        if i >= max {
            return Err(format!("varint 标记过长（> {max} 字节）"));
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, i + 1));
        }
        shift += 7;
        if shift >= 64 {
            return Err("varint 位宽溢出".to_string());
        }
    }
    Err("varint 数据不完整".to_string())
}

// ------------------------------------------------------------------
// 帧层
// ------------------------------------------------------------------

/// 编码明文帧：`0x00 || varint(payload_len) || type || payload`。
/// **wire 标记 = 载荷长度（不含类型字节）**——参考实现编码器写 `item.len()-1`
/// 的 asymmetry 怪癖（对拍实证）：`+1` 是解码器侧还原，不在 wire 上。
pub(crate) fn frame_encode(message_type: u8, payload: &[u8]) -> Result<Vec<u8>, String> {
    let frame_len = payload.len() + 1; // + 类型字节（上限口径与参考实现一致）
    if frame_len > MAX_FRAME_LEN {
        return Err(format!("帧超限（{frame_len} > {MAX_FRAME_LEN}）"));
    }
    let mut out = Vec::with_capacity(frame_len + 6);
    out.push(0x00);
    varint_encode(payload.len() as u64, &mut out);
    out.push(message_type);
    out.extend_from_slice(payload);
    Ok(out)
}

/// 帧解码结果：类型字节 + protobuf 载荷。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedFrame {
    pub message_type: u8,
    pub payload: Vec<u8>,
}

/// 解码明文帧：起始字节必须 `0x00`（`0x01` = 加密帧，显式报错），varint 标记
/// ≤ 4 字节，帧体长度 = 标记值 **+ 1**（解码器侧还原类型字节，参考实现
/// `decode_length_delimiter(..) + 1` 同口径），数据不足声明长度报错。
pub(crate) fn frame_decode(data: &[u8]) -> Result<ParsedFrame, String> {
    let marker = data.first().ok_or("空帧")?;
    match *marker {
        0x00 => {}
        0x01 => return Err("加密帧（noise）不在插件明文面范围".to_string()),
        other => return Err(format!("非法起始字节：{other:#04x}")),
    }
    let (marker_value, marker_len) = varint_decode(&data[1..], FRAME_MARKER_MAX)?;
    // 解码器侧 +1：标记 = 载荷长度，帧体 = 类型字节 + 载荷。
    let frame_len = marker_value as usize + 1;
    let total = 1 + marker_len + frame_len;
    if data.len() < total {
        return Err(format!("数据短于帧声明（{} < {total}）", data.len()));
    }
    let message_type = data[1 + marker_len];
    let payload = data[2 + marker_len..1 + marker_len + frame_len].to_vec();
    Ok(ParsedFrame {
        message_type,
        payload,
    })
}

/// 从流式缓冲切出**下一个完整明文帧**（保留半包/粘包余量，返回整帧字节）。
///
/// 只做边界判定（`0x00` 标记 + varint 长度 + 类型字节），内容语义交给
/// [`frame_decode`]；起始标记非 0（加密帧）显式报错——会话层据此区分明文/加密。
pub(crate) fn next_frame_from_buffer(buf: &mut Vec<u8>) -> Result<Option<Vec<u8>>, String> {
    if buf.is_empty() {
        return Ok(None);
    }
    if buf[0] != 0x00 {
        return Err(format!(
            "明文会话收到非明文帧标记 {:#04x}（协议不匹配或对端走加密）",
            buf[0]
        ));
    }
    let (marker_value, marker_len) = match varint_decode(&buf[1..], FRAME_MARKER_MAX) {
        Ok(parsed) => parsed,
        Err(_) if buf.len() - 1 < FRAME_MARKER_MAX => return Ok(None),
        Err(error) => return Err(error),
    };
    // 解码器侧 +1：wire 标记 = 载荷长度，帧体 = 类型字节 + 载荷。
    if marker_value as usize + 1 > INPUT_CAP {
        return Err("会话明文帧声明超限".to_string());
    }
    let total = 1 + marker_len + marker_value as usize + 1;
    if buf.len() < total {
        return Ok(None);
    }
    let frame = buf[..total].to_vec();
    buf.drain(..total);
    Ok(Some(frame))
}

// ------------------------------------------------------------------
// 消息类型映射（parser.rs 核心子集）
// ------------------------------------------------------------------

/// 消息名 → 类型 id（parser.rs `proto_message_mappings!` 同值）。
pub(crate) fn message_type_id(name: &str) -> Result<u8, String> {
    match name {
        "hello_request" => Ok(1),
        "hello_response" => Ok(2),
        "authentication_request" => Ok(3),
        "authentication_response" => Ok(4),
        "disconnect_request" => Ok(5),
        "disconnect_response" => Ok(6),
        "ping_request" => Ok(7),
        "ping_response" => Ok(8),
        "device_info_request" => Ok(9),
        "device_info_response" => Ok(10),
        "list_entities_request" => Ok(11),
        "list_entities_binary_sensor_response" => Ok(12),
        "list_entities_switch_response" => Ok(17),
        "list_entities_done_response" => Ok(19),
        "subscribe_states_request" => Ok(20),
        "binary_sensor_state_response" => Ok(21),
        "switch_state_response" => Ok(26),
        "switch_command_request" => Ok(33),
        "list_entities_number_response" => Ok(49),
        "number_state_response" => Ok(50),
        "number_command_request" => Ok(51),
        other => Err(format!("未收录的消息类型：{other}")),
    }
}

/// 类型 id → 消息名（frame-parse 结果可读性）。
pub(crate) fn message_type_name(id: u8) -> Option<&'static str> {
    match id {
        1 => Some("hello_request"),
        2 => Some("hello_response"),
        3 => Some("authentication_request"),
        4 => Some("authentication_response"),
        5 => Some("disconnect_request"),
        6 => Some("disconnect_response"),
        7 => Some("ping_request"),
        8 => Some("ping_response"),
        9 => Some("device_info_request"),
        10 => Some("device_info_response"),
        11 => Some("list_entities_request"),
        12 => Some("list_entities_binary_sensor_response"),
        17 => Some("list_entities_switch_response"),
        19 => Some("list_entities_done_response"),
        20 => Some("subscribe_states_request"),
        21 => Some("binary_sensor_state_response"),
        26 => Some("switch_state_response"),
        33 => Some("switch_command_request"),
        49 => Some("list_entities_number_response"),
        50 => Some("number_state_response"),
        51 => Some("number_command_request"),
        _ => None,
    }
}

// ------------------------------------------------------------------
// protobuf 字段编码（prost 语义：声明序 + 默认值跳过）
// ------------------------------------------------------------------

pub(crate) const WIRE_VARINT: u32 = 0;
pub(crate) const WIRE_LEN: u32 = 2;
pub(crate) const WIRE_FIXED32: u32 = 5;

pub(crate) fn pb_key(tag: u32, wire_type: u32, out: &mut Vec<u8>) {
    varint_encode((u64::from(tag) << 3) | u64::from(wire_type), out);
}

pub(crate) fn pb_varint_field(tag: u32, value: u64, out: &mut Vec<u8>) {
    if value == 0 {
        return; // prost：零值不写 wire
    }
    pb_key(tag, WIRE_VARINT, out);
    varint_encode(value, out);
}

pub(crate) fn pb_len_field(tag: u32, data: &[u8], out: &mut Vec<u8>) {
    if data.is_empty() {
        return; // prost：空串不写 wire
    }
    pb_key(tag, WIRE_LEN, out);
    varint_encode(data.len() as u64, out);
    out.extend_from_slice(data);
}

pub(crate) fn pb_fixed32_field(tag: u32, value: u32, out: &mut Vec<u8>) {
    if value == 0 {
        return;
    }
    pb_key(tag, WIRE_FIXED32, out);
    out.extend_from_slice(&value.to_le_bytes());
}

/// float 字段：wire = fixed32（IEEE-754 位模式 LE）；prost 对 0.0 同样跳过。
pub(crate) fn pb_float_field(tag: u32, value: f32, out: &mut Vec<u8>) {
    if value == 0.0 {
        return;
    }
    pb_fixed32_field(tag, value.to_bits(), out);
}

/// `HelloRequest` 载荷（声明序：client_info(1) string、api_version_major(2)、
/// api_version_minor(3)，均 uint32 varint）。
pub(crate) fn hello_request_payload(client_info: &str, major: u32, minor: u32) -> Vec<u8> {
    let mut out = Vec::new();
    pb_len_field(1, client_info.as_bytes(), &mut out);
    pb_varint_field(2, u64::from(major), &mut out);
    pb_varint_field(3, u64::from(minor), &mut out);
    out
}

/// `SwitchCommandRequest` 载荷（声明序：key(1) **fixed32**、state(2) bool、
/// device_id(3) uint32）。
pub(crate) fn switch_command_payload(key: u32, state: bool, device_id: u32) -> Vec<u8> {
    let mut out = Vec::new();
    pb_fixed32_field(1, key, &mut out);
    pb_varint_field(2, u64::from(state), &mut out);
    pb_varint_field(3, u64::from(device_id), &mut out);
    out
}

// ------------------------------------------------------------------
// HelloResponse 解码（字段走读 + 未知字段跳过）
// ------------------------------------------------------------------

/// `HelloResponse` 关键字段（major(1)/minor(2) varint、server_info(3)/name(4) string）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HelloResponseInfo {
    pub api_version_major: u32,
    pub api_version_minor: u32,
    pub server_info: String,
    pub name: String,
}

/// 解码 HelloResponse 载荷：按 wire key 走读，未知字段按 wire type 跳过
/// （向后兼容新版固件加字段）。
pub(crate) fn hello_response_decode(payload: &[u8]) -> Result<HelloResponseInfo, String> {
    let mut info = HelloResponseInfo {
        api_version_major: 0,
        api_version_minor: 0,
        server_info: String::new(),
        name: String::new(),
    };
    let mut rest = payload;
    while !rest.is_empty() {
        let (key, n) = varint_decode(rest, 5)?;
        rest = &rest[n..];
        let wire_type = (key & 0x7) as u32;
        let tag = (key >> 3) as u32;
        match wire_type {
            WIRE_VARINT => {
                let (v, n) = varint_decode(rest, 5)?;
                rest = &rest[n..];
                match tag {
                    1 => info.api_version_major = v as u32,
                    2 => info.api_version_minor = v as u32,
                    _ => {}
                }
            }
            WIRE_LEN => {
                let (len, n) = varint_decode(rest, 5)?;
                rest = &rest[n..];
                let len = len as usize;
                if rest.len() < len {
                    return Err("长度字段的载荷不完整".to_string());
                }
                let bytes = &rest[..len];
                rest = &rest[len..];
                match tag {
                    3 => info.server_info = String::from_utf8_lossy(bytes).into_owned(),
                    4 => info.name = String::from_utf8_lossy(bytes).into_owned(),
                    _ => {}
                }
            }
            WIRE_FIXED32 => {
                if rest.len() < 4 {
                    return Err("fixed32 字段不完整".to_string());
                }
                rest = &rest[4..];
            }
            1 => {
                if rest.len() < 8 {
                    return Err("fixed64 字段不完整".to_string());
                }
                rest = &rest[8..];
            }
            other => return Err(format!("未知 wire type：{other}")),
        }
    }
    Ok(info)
}
