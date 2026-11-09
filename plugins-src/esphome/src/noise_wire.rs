//! S3t：ESPHome Native API 的 **noise 帧布局**（guest 单源）。
//!
//! 与 [`crate::api_frame`] 的明文帧（`0x00 ‖ varint ‖ type ‖ payload`）并列，
//! 本文件只处理加密会话的线格式；规格权威 = aioesphomeapi `_frame_helper/noise.py`
//! 与 `esphome-native-api` 3.0.0 `frame.rs` 加密分支：
//!
//! - **加密帧**：`0x01 ‖ u16 BE 密文长度 ‖ 密文`（密文 = AEAD 输出，含 16 字节 tag）；
//! - **ClientHello**：固定 3 字节 `01 00 00`（加密帧头的零长度帧）；
//! - **ServerHello**：加密帧内容的 `0x01 ‖ name ‖ 0x00 [‖ mac ‖ 0x00]`——首字节是
//!   会话选定的加密协议（当前只有 `0x01` = NNpsk0/25519/ChaChaPoly/SHA256）；
//! - **握手帧**：加密帧内容 = `0x00 ‖ noise 握手消息`（首字节 0 表示手 σ 消息，
//!   非 0 = 对端错误文本帧，例如 `0x01 "Handshake MAC failure"`）；
//! - **传输帧**：加密帧内容的明文 = `type(u16 BE) ‖ payload_len(u16 BE) ‖ payload`
//!   ——与本仓明文帧的 `type + protobuf` 是同一消息面，仅长度字段改成 u16 BE。
//!
//! 长度字段是 u16，故单帧内容上限 65535 字节（含 tag）。

/// 加密帧起始标记。
pub(crate) const ENCRYPTED_MARKER: u8 = 0x01;
/// 握手帧内容首字节：noise 握手消息。
pub(crate) const HANDSHAKE_TYPE: u8 = 0x00;
/// ServerHello 内容首字节：选定的加密协议（0x01 = Noise NNpsk0 25519 ChaChaPoly SHA256）。
pub(crate) const SERVER_HELLO_PROTOCOL: u8 = 0x01;
/// ClientHello 固定 3 字节。
pub(crate) const CLIENT_HELLO: [u8; 3] = [ENCRYPTED_MARKER, 0x00, 0x00];
/// u16 长度字段上限（帧内容）。
pub(crate) const MAX_FRAME_CONTENT: usize = u16::MAX as usize;
/// ServerHello 内 name / mac 字段上限（aioesphomeapi `MAX_NAME_LEN`/`MAX_MAC_LEN` 同量级）。
const MAX_LABEL_LEN: usize = 128;
/// 单次 feed 的接收缓冲上限（握手/传输流式重组；防御性，正常远小于此）。
pub(crate) const MAX_RX_BUF: usize = 64 * 1024;

/// 加密帧包装：`0x01 ‖ u16 BE(len) ‖ content`。
pub(crate) fn frame_encode(content: &[u8]) -> Result<Vec<u8>, String> {
    if content.len() > MAX_FRAME_CONTENT {
        return Err(format!(
            "加密帧内容超限（{} > {MAX_FRAME_CONTENT} 字节）",
            content.len()
        ));
    }
    let mut out = Vec::with_capacity(3 + content.len());
    out.push(ENCRYPTED_MARKER);
    out.extend_from_slice(&(content.len() as u16).to_be_bytes());
    out.extend_from_slice(content);
    Ok(out)
}

/// ClientHello：`01 00 00`（aioesphomeapi `NOISE_HELLO` 同字节）。
pub(crate) fn client_hello() -> Vec<u8> {
    CLIENT_HELLO.to_vec()
}

/// ServerHello 帧：内容 `01 ‖ name ‖ 00 [‖ mac ‖ 00]`（参考实现
/// `generate_server_hello_frame` 同布局）。
pub(crate) fn server_hello(name: &str, mac: Option<&str>) -> Result<Vec<u8>, String> {
    if name.is_empty() || name.len() > MAX_LABEL_LEN {
        return Err(format!(
            "server_hello.name 长度非法（{}，须 1..={MAX_LABEL_LEN}）",
            name.len()
        ));
    }
    if name.as_bytes().contains(&0) {
        return Err("server_hello.name 不得含 NUL".to_string());
    }
    let mut content = Vec::with_capacity(name.len() + 4);
    content.push(SERVER_HELLO_PROTOCOL);
    content.extend_from_slice(name.as_bytes());
    content.push(0);
    if let Some(mac) = mac {
        if mac.len() > MAX_LABEL_LEN || mac.as_bytes().contains(&0) {
            return Err(format!("server_hello.mac 非法（{} 字节）", mac.len()));
        }
        content.extend_from_slice(mac.as_bytes());
        content.push(0);
    }
    frame_encode(&content)
}

/// ServerHello 解析结果（客户端取对端 name/mac 做身份核对与日志）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerHelloInfo {
    /// 选定的加密协议字节（必须 [`SERVER_HELLO_PROTOCOL`]）。
    pub protocol: u8,
    pub name: String,
    pub mac: Option<String>,
}

/// 解析 ServerHello 帧内容；协议字节不匹配/非 UTF-8/缺 name 终止符均显式报错。
pub(crate) fn parse_server_hello(content: &[u8]) -> Result<ServerHelloInfo, String> {
    let protocol = *content.first().ok_or("ServerHello 为空")?;
    if protocol != SERVER_HELLO_PROTOCOL {
        return Err(format!(
            "ServerHello 选定的加密协议未知：{protocol:#04x}（仅支持 0x01 = Noise NNpsk0）"
        ));
    }
    let name_end = content
        .iter()
        .position(|b| *b == 0)
        .filter(|i| *i > 1)
        .ok_or("ServerHello 缺少 name 终止符（0x00）")?;
    let name = String::from_utf8_lossy(&content[1..name_end]).into_owned();
    if name.len() > MAX_LABEL_LEN {
        return Err("ServerHello name 超长".to_string());
    }
    // mac 段是可选的（2025.4 起固件才带）：name 终止符后还有非空内容才解析。
    let mac = match content.get(name_end + 1..) {
        Some(rest) if !rest.is_empty() => {
            let mac_end = rest
                .iter()
                .position(|b| *b == 0)
                .ok_or("ServerHello mac 缺少终止符（0x00）")?;
            Some(String::from_utf8_lossy(&rest[..mac_end]).into_owned())
        }
        _ => None,
    };
    Ok(ServerHelloInfo {
        protocol,
        name,
        mac,
    })
}

/// 握手帧：`0x00 ‖ noise 消息` 的加密帧包装。
pub(crate) fn handshake_frame(noise_message: &[u8]) -> Result<Vec<u8>, String> {
    let mut content = Vec::with_capacity(1 + noise_message.len());
    content.push(HANDSHAKE_TYPE);
    content.extend_from_slice(noise_message);
    frame_encode(&content)
}

/// 对端错误文本帧解析：首字节 `0x01`（非握手消息）后的文本（aioesphomeapi
/// `_error_on_incorrect_preamble` 同口径，如 `Handshake MAC failure`）。
pub(crate) fn parse_error_frame(content: &[u8]) -> String {
    let text = content.get(1..).unwrap_or_default();
    String::from_utf8_lossy(text).into_owned()
}

/// 从流式缓冲切出**下一个**完整加密帧内容（保留半包在 `buf`）；起始标记非法报错。
pub(crate) fn next_frame(buf: &mut Vec<u8>) -> Result<Option<Vec<u8>>, String> {
    if buf.len() < 3 {
        return Ok(None);
    }
    let marker = buf[0];
    if marker != ENCRYPTED_MARKER {
        return Err(format!(
            "加密会话收到非法帧标记 {marker:#04x}（协议不匹配：对端可能走明文或帧头损坏）"
        ));
    }
    let len = u16::from_be_bytes([buf[1], buf[2]]) as usize;
    if buf.len() < 3 + len {
        return Ok(None); // 半包：等待后续字节
    }
    let content = buf[3..3 + len].to_vec();
    buf.drain(..3 + len);
    Ok(Some(content))
}

/// 传输帧明文：`type(u16 BE) ‖ payload_len(u16 BE) ‖ payload`。
pub(crate) fn transport_plaintext(message_type: u16, payload: &[u8]) -> Result<Vec<u8>, String> {
    if payload.len() > u16::MAX as usize {
        return Err(format!("传输帧载荷超限（{} > 65535）", payload.len()));
    }
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&message_type.to_be_bytes());
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// 解析传输帧明文 → (消息类型, protobuf 载荷)。
///
/// 长度字段与参考实现（aioesphomeapi / esphome-native-api）同口径**不信任**：
/// 只要求明文 ≥ 4 字节，载荷取 `[4..]` 的余量。
pub(crate) fn transport_parse(plaintext: &[u8]) -> Result<(u16, Vec<u8>), String> {
    if plaintext.len() < 4 {
        return Err(format!("传输帧明文过短（{} < 4）", plaintext.len()));
    }
    let message_type = u16::from_be_bytes([plaintext[0], plaintext[1]]);
    Ok((message_type, plaintext[4..].to_vec()))
}
