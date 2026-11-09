//! S3j/S3o：Tuya LAN **0x6699（v3.5）帧编解码 + v3.5 控制命令帧**。
//!
//! 规格权威 = rustuya `protocol::pack_message / parse_header / unpack_message` 的
//! 6699 分支 + `ProtocolV35`：`prefix(4BE)=6699 + 未知 u16 + seqno(4) + cmd(4) +
//! payload_len(4)=iv(12)+raw(含可选 retcode)+tag(16) + iv + ct + tag + suffix(4)=9966`；
//! AES-128-GCM 的 AAD = 头 `[4..18]`（seqno/cmd/len 三字段），IV 随帧明文携带
//! （由调用方传入：事件面/对拍用固定向量，编排侧经宿主随机源 + `lan_session::derive_iv`）。
//! retcode 启发式与 rustuya 同款（**正向**要求 `payload[4] ∈ {'{','3'}`）；会话/
//! 数据面二进制载荷走 `parse_frame_6699_exact`（启发式只适用 JSON 面）。
//!
//! 对拍：`plugins/wasm/tests/tuya_lan.rs` / `tuya_lan_session_v35.rs` /
//! `tuya_lan_orchestrate.rs` 用 rustuya 本尊生成/解析黄金帧。
//!
//! 由 `lan_codec.rs` 拆出（S3o）：该文件回到 55AA + v3.3/v3.4 命令面，600 行红线。

#![allow(deprecated)]

use aes_gcm::aead::{AeadInPlace, KeyInit as AeadKeyInit, Nonce, Payload};
use aes_gcm::Aes128Gcm;

use crate::lan_codec::{
    be_u32, be_u32_at, command_payload_34, MAX_PAYLOAD_LEN, NO_PROTOCOL_HEADER_CMDS,
};

pub(crate) const PREFIX_6699: u32 = 0x0000_6699;
pub(crate) const SUFFIX_6699: u32 = 0x0000_9966;
/// 0x6699：头 18（prefix4 + 未知 u16 + seqno4 + cmd4 + payload_len4）+ suffix 4。
const HEADER_6699_LEN: usize = 18;
const IV_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// 编码 0x6699（3.5）帧：AES-128-GCM（nonce = `iv` 12B，AAD = 头 `data[4..18]`）。
///
/// `frame = 头(18) + iv(12) + ct||tag + suffix(4)`；`payload_len` 字段 =
/// `iv + raw + tag`（raw = retcode? + payload）。IV 由调用方传入（guest 无 rand 函数，
/// rustuya 参考 `msg.iv` 传入时同样不用随机数（黄金帧对拍用固定 IV）。
/// `key` 必须 16 字节（AES-128）。
pub(crate) fn pack_frame_6699(
    seqno: u32,
    cmd: u32,
    payload: &[u8],
    retcode: Option<u32>,
    key: &[u8],
    iv: &[u8],
) -> Result<Vec<u8>, String> {
    if payload.len() + 4 > MAX_PAYLOAD_LEN {
        return Err(format!(
            "payload 超限（{} > {MAX_PAYLOAD_LEN}）",
            payload.len()
        ));
    }
    if key.len() != 16 {
        return Err(format!("GCM 密钥必须 16 字节（当前 {}）", key.len()));
    }
    if iv.len() != IV_LEN {
        return Err(format!("IV 必须 {IV_LEN} 字节（当前 {}）", iv.len()));
    }
    let raw_len = if retcode.is_some() { 4 } else { 0 } + payload.len();
    let total_payload = IV_LEN + raw_len + TAG_LEN;

    let mut header_bytes = Vec::with_capacity(HEADER_6699_LEN);
    be_u32(&mut header_bytes, PREFIX_6699);
    header_bytes.extend_from_slice(&[0u8, 0u8]); // 未知 u16（rustuya 写 0）
    be_u32(&mut header_bytes, seqno);
    be_u32(&mut header_bytes, cmd);
    be_u32(&mut header_bytes, total_payload as u32);

    let mut raw = Vec::with_capacity(raw_len);
    if let Some(rc) = retcode {
        be_u32(&mut raw, rc);
    }
    raw.extend_from_slice(payload);

    let cipher = <Aes128Gcm as AeadKeyInit>::new_from_slice(key)
        .map_err(|_| "GCM 密钥初始化失败".to_string())?;
    let nonce = Nonce::<Aes128Gcm>::from_slice(&iv[..IV_LEN]);
    let payload_msg = Payload {
        msg: raw.as_slice(),
        aad: &header_bytes[4..],
    };
    let mut buffer = raw.clone();
    let tag = cipher
        .encrypt_in_place_detached(nonce, payload_msg.aad, &mut buffer)
        .map_err(|_| "GCM 加密失败".to_string())?;

    let mut data = header_bytes;
    data.extend_from_slice(&iv[..IV_LEN]);
    data.extend_from_slice(&buffer);
    data.extend_from_slice(&tag);
    be_u32(&mut data, SUFFIX_6699);
    Ok(data)
}

/// 0x6699 解析结果：payload 为解密后剔除 retcode 的余量。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedFrame6699 {
    pub seqno: u32,
    pub cmd: u32,
    pub retcode: Option<u32>,
    pub payload: Vec<u8>,
    pub iv: Vec<u8>,
}

/// 解析 0x6699 帧：头校验 → AES-128-GCM 解密（AAD = 头 `[4..18]`，tag 附于尾）
/// → retcode 启发式（与 55AA 不同：**正向**要求 `payload[4] ∈ {'{','3'}`，rustuya
/// 同款）。
pub(crate) fn parse_frame_6699(data: &[u8], key: &[u8]) -> Result<ParsedFrame6699, String> {
    parse_frame_6699_impl(data, key, true)
}

/// **精确解析**（不剥 retcode）：会话/数据面二进制载荷专用——rustuya 接收路径把
/// retcode 启发式作用在会话密文/非 JSON 载荷上属实现缺陷（S3m 同款口径），编排按
/// 协议语义实现。
pub(crate) fn parse_frame_6699_exact(data: &[u8], key: &[u8]) -> Result<ParsedFrame6699, String> {
    parse_frame_6699_impl(data, key, false)
}

fn parse_frame_6699_impl(
    data: &[u8],
    key: &[u8],
    strip_retcode: bool,
) -> Result<ParsedFrame6699, String> {
    if data.len() < HEADER_6699_LEN {
        return Err("6699 帧头过短（<18）".to_string());
    }
    if key.len() != 16 {
        return Err(format!("GCM 密钥必须 16 字节（当前 {}）", key.len()));
    }
    let prefix = be_u32_at(data, 0).ok_or("帧头不可读")?;
    if prefix != PREFIX_6699 {
        return Err(format!("非 0x6699 前缀：{prefix:#010x}"));
    }
    let seqno = be_u32_at(data, 6).ok_or("seqno 不可读")?;
    let cmd = be_u32_at(data, 10).ok_or("cmd 不可读")?;
    let payload_len = be_u32_at(data, 14).ok_or("payload_len 不可读")?;
    if payload_len as usize > MAX_PAYLOAD_LEN {
        return Err(format!(
            "头部长度谎报：{payload_len} 超过上限 {MAX_PAYLOAD_LEN}"
        ));
    }
    let msg_len = HEADER_6699_LEN + payload_len as usize + 4;
    if data.len() < msg_len {
        return Err(format!("数据短于帧声明（{} < {msg_len}）", data.len()));
    }

    let with_iv_tag = &data[HEADER_6699_LEN..HEADER_6699_LEN + payload_len as usize];
    if with_iv_tag.len() < IV_LEN + TAG_LEN {
        return Err("6699 载荷不足 iv+tag".to_string());
    }
    let iv = with_iv_tag[..IV_LEN].to_vec();
    let cipher = <Aes128Gcm as AeadKeyInit>::new_from_slice(key)
        .map_err(|_| "GCM 密钥初始化失败".to_string())?;
    let nonce = Nonce::<Aes128Gcm>::from_slice(&iv);
    // ct 与 tag 分离（aead 0.6 detached API 的第 4 参）；ct 从 IV 之后开始
    let split = with_iv_tag.len() - TAG_LEN;
    let mut buffer = with_iv_tag[IV_LEN..split].to_vec();
    let tag = aes_gcm::aead::Tag::<Aes128Gcm>::from_slice(&with_iv_tag[split..]);
    cipher
        .decrypt_in_place_detached(nonce, &data[4..HEADER_6699_LEN], &mut buffer, &tag)
        .map_err(|_| "GCM 解密失败（tag/AAD 不匹配或密钥错误）".to_string())?;

    let mut payload = buffer;
    let mut retcode = None;
    let retcode_len = 4;
    let should_parse_retcode = strip_retcode
        && payload.len() >= retcode_len
        && payload[0] != b'{'
        && (payload.len() > retcode_len
            && (payload[retcode_len] == b'{' || payload[retcode_len] == b'3'));
    if should_parse_retcode && payload.len() >= retcode_len {
        retcode = Some(u32::from_be_bytes([
            payload[0], payload[1], payload[2], payload[3],
        ]));
        payload = payload[retcode_len..].to_vec();
    }

    Ok(ParsedFrame6699 {
        seqno,
        cmd,
        retcode,
        payload,
        iv,
    })
}

// ============================================================
// S3j：v3.5 TCP 命令帧（0x6699/GCM 会话密钥模式；rustuya ProtocolV35 对拍）
// ============================================================

/// v3.5 控制命令**完整帧**：载荷臂与 v3.4 完全一致（rustuya 两代共享同一批
/// generate_payload helper，复用 `command_payload_34`）；`pack_payload` 仅拼
/// "3.5" 头**不做帧前加密**，加密在帧级 0x6699 AES-128-GCM（nonce=调用方传
/// IV，AAD=帧头 [4..18]）。`key` 为会话密钥 blob。返回 (effective_cmd, frame)。
pub(crate) fn command_frame_35(
    seqno: u32,
    device_id: &str,
    cid: Option<&str>,
    t: u64,
    command: &str,
    dps: serde_json::Value,
    key: &[u8],
    iv: &[u8],
) -> Result<(u32, Vec<u8>), String> {
    let (cmd, payload) = command_payload_34(device_id, cid, t, command, dps)?;
    let mut plain = payload.to_string().into_bytes();
    if !NO_PROTOCOL_HEADER_CMDS.contains(&cmd) {
        let mut with_header = b"3.5".to_vec();
        with_header.extend_from_slice(&[0u8; 12]);
        with_header.extend_from_slice(&plain);
        plain = with_header;
    }
    let frame = pack_frame_6699(seqno, cmd, &plain, None, key, iv)?;
    Ok((cmd, frame))
}
