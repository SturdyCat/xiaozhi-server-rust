//! Tuya LAN 0x55AA 帧编解码（阶段三 S3f：LAN 域协议语义 guest 侧单源）。
//!
//! 规格权威 = 外部 crate `rustuya`（当前 native LAN 栈所有者）的
//! `protocol::pack_message / parse_header / unpack_message` 55AA 分支：
//! `prefix(4BE)=55AA + seqno(4) + cmd(4) + payload_len(4)=payload+尾差
//! （CRC 变体 4+4、HMAC 变体 32+4）+ payload + [CRC32-ISO-HDLC(4) |
//! HMAC-SHA256(32)] + suffix(4)=AA55`；校验码覆盖其前全部字节；retcode
//! 启发式与 rustuya 逐条对齐（payload ≥4 且首字节非 `{` 且（首字节==0
//! 或（len>4 且首字节≠`3`）））。0x6699（3.5/AES-GCM）不在本文件范围。
//!
//! 与宿主的对拍：`plugins/wasm/tests/tuya_lan.rs` 用 **rustuya 本尊**在宿主侧
//! 生成黄金帧，与 guest 本文件输出逐字节比较——P8c LAN 域第一块无需真机的
//! 迁移证据。宿主当前继续经 rustuya async facade 使用 LAN 数据面（worker/lan）；
//! 本文件是「协议语义可迁插件」的落点，待全链路证据齐备后再谈 native 删除。
//!
//! 依赖刻意最小：hmac + sha2 + aes-gcm（S3g 起含 0x6699/AES-GCM），CRC 手写位
//! 循环（不引 crc crate）。

// aead 0.6 的 detached API 已标 deprecated（改用 inout），本文件保留旧形态最小改动。
#![allow(deprecated)]

use hmac::digest::KeyInit as HmacKeyInit;
use hmac::digest::Mac as MacTrait;
use hmac::Hmac;
use sha2::Sha256;

use crate::lan_crypto::aes_ecb_encrypt_pkcs7;

pub(crate) const PREFIX_55AA: u32 = 0x0000_55AA;
pub(crate) const SUFFIX_55AA: u32 = 0x0000_AA55;
/// 与 rustuya `MAX_PAYLOAD_LEN` 同口径（256KB），拒绝头部长度谎报。
pub(crate) const MAX_PAYLOAD_LEN: usize = 256 * 1024;
const HEADER_LEN: usize = 16;
const CRC_TRAILER: usize = 8; // crc(4) + suffix(4)
const HMAC_TRAILER: usize = 36; // hmac(32) + suffix(4)

/// CRC-32/ISO-HDLC（反射多项式 0xEDB88320，init/final-xor 0xFFFFFFFF）——
/// 即 crc crate `CRC_32_ISO_HDLC`（zlib 同款），位循环实现免查表免依赖。
pub(crate) fn crc32_iso_hdlc(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

pub(crate) fn be_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

pub(crate) fn be_u32_at(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// 编码 0x55AA 帧：`hmac_key` 有 → HMAC-SHA256 尾（3.3/3.4 安全帧），
/// 无 → CRC32 尾（明文帧）。`payload_len` 字段按 rustuya 口径含尾差。
pub(crate) fn pack_frame_55aa(
    seqno: u32,
    cmd: u32,
    payload: &[u8],
    hmac_key: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(format!(
            "payload 超限（{} > {MAX_PAYLOAD_LEN}）",
            payload.len()
        ));
    }
    let trailer = if hmac_key.is_some() {
        HMAC_TRAILER
    } else {
        CRC_TRAILER
    };
    let payload_len_field = payload.len() + trailer;
    let mut data = Vec::with_capacity(HEADER_LEN + payload_len_field);
    be_u32(&mut data, PREFIX_55AA);
    be_u32(&mut data, seqno);
    be_u32(&mut data, cmd);
    be_u32(&mut data, payload_len_field as u32);
    data.extend_from_slice(payload);
    match hmac_key {
        Some(key) => {
            let mut mac =
                Hmac::<Sha256>::new_from_slice(key).map_err(|_| "HMAC 密钥长度非法".to_string())?;
            mac.update(&data);
            data.extend_from_slice(&mac.finalize().into_bytes());
        }
        None => {
            let checksum = crc32_iso_hdlc(&data);
            be_u32(&mut data, checksum);
        }
    }
    be_u32(&mut data, SUFFIX_55AA);
    Ok(data)
}

/// 解析结果：retcode 启发式与 rustuya 同款（见模块注释）；payload 为剔除
/// retcode 后的余量（JSON 命令 / DP 载荷）。
///
/// 两个包装器：`parse_frame_55aa`（剥 retcode，JSON 面既有行为）与
/// `parse_frame_55aa_exact`（不剥，S3m 会话编排的**二进制载荷面**专用——
/// rustuya 接收路径把启发式作用在会话密文上属实现缺陷，编排按协议语义实现）。
pub(crate) fn parse_frame_55aa_exact(
    data: &[u8],
    hmac_key: Option<&[u8]>,
) -> Result<ParsedFrame, String> {
    parse_frame_55aa(data, hmac_key, false)
}

pub(crate) fn parse_frame_55aa_strip(
    data: &[u8],
    hmac_key: Option<&[u8]>,
) -> Result<ParsedFrame, String> {
    parse_frame_55aa(data, hmac_key, true)
}

/// 带启发式开关的核心解析。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedFrame {
    pub seqno: u32,
    pub cmd: u32,
    pub retcode: Option<u32>,
    pub payload: Vec<u8>,
}

/// 解析 0x55AA 帧：头校验（长度谎报拒绝）→ 尾校验（CRC / HMAC）→ retcode 启发式。
pub(crate) fn parse_frame_55aa(
    data: &[u8],
    hmac_key: Option<&[u8]>,
    strip_retcode: bool,
) -> Result<ParsedFrame, String> {
    if data.len() < HEADER_LEN {
        return Err("帧头过短（<16）".to_string());
    }
    let prefix = be_u32_at(data, 0).ok_or("帧头不可读")?;
    if prefix != PREFIX_55AA {
        return Err(format!("非 0x55AA 前缀：{prefix:#010x}"));
    }
    let seqno = be_u32_at(data, 4).ok_or("seqno 不可读")?;
    let cmd = be_u32_at(data, 8).ok_or("cmd 不可读")?;
    let payload_len = be_u32_at(data, 12).ok_or("payload_len 不可读")?;
    if payload_len as usize > MAX_PAYLOAD_LEN {
        return Err(format!(
            "头部长度谎报：{payload_len} 超过上限 {MAX_PAYLOAD_LEN}"
        ));
    }
    let msg_len = HEADER_LEN + payload_len as usize;
    if data.len() < msg_len {
        return Err(format!("数据短于帧声明（{} < {msg_len}）", data.len()));
    }

    let trailer = if hmac_key.is_some() {
        HMAC_TRAILER
    } else {
        CRC_TRAILER
    };
    // 防御式减法（防 msg_len < trailer 的下溢 panic）
    let Some(payload_end) = msg_len.checked_sub(trailer) else {
        return Err(format!("帧长（{msg_len}）短于尾差（{trailer}）"));
    };
    if payload_end < HEADER_LEN {
        return Err(format!(
            "载荷结束点（{payload_end}）越过帧头（{HEADER_LEN}）"
        ));
    }

    // 尾校验：覆盖 payload_end 之前的全部字节（rustuya 同口径）
    let checksum_data = &data[..payload_end];
    let footer = &data[payload_end..msg_len];
    match hmac_key {
        Some(key) => {
            let mut mac =
                Hmac::<Sha256>::new_from_slice(key).map_err(|_| "HMAC 密钥长度非法".to_string())?;
            mac.update(checksum_data);
            if mac.finalize().into_bytes().as_slice() != &footer[..32] {
                return Err("HMAC 不匹配".to_string());
            }
        }
        None => {
            let expect = crc32_iso_hdlc(checksum_data);
            let recv = u32::from_be_bytes([
                *footer.first().unwrap_or(&0),
                *footer.get(1).unwrap_or(&0),
                *footer.get(2).unwrap_or(&0),
                *footer.get(3).unwrap_or(&0),
            ]);
            if expect != recv {
                return Err(format!(
                    "CRC 不匹配（期望 {expect:#010x}，收到 {recv:#010x}）"
                ));
            }
        }
    }

    // retcode 启发式（逐条对齐 rustuya unpack 55AA 分支）
    let mut payload_start = HEADER_LEN;
    let mut retcode = None;
    let remaining = payload_end - payload_start;
    // retcode 启发式只适用于 JSON 面；二进制载荷（会话协商密文）必须走
    // `strip_retcode=false` 的 exact 语义（否则随机首字节几乎必剥 4 字节）。
    let should_parse_retcode = strip_retcode
        && remaining >= 4
        && data[payload_start] != b'{'
        && (data[payload_start] == 0 || (remaining > 4 && data[payload_start] != b'3'));
    if should_parse_retcode && remaining >= 4 {
        retcode = be_u32_at(data, payload_start);
        payload_start += 4;
    }

    Ok(ParsedFrame {
        seqno,
        cmd,
        retcode,
        payload: data[payload_start..payload_end].to_vec(),
    })
}

// ============================================================
// S3h：v3.3 控制命令全链路（载荷形状 → ECB 加密 → 帧组装）
// ============================================================
// 规格逐条对齐 rustuya `ProtocolV33`（generate_payload / pack_payload）与
// `create_base_payload` / `apply_update_dps` / NO_PROTOCOL_HEADER_CMDS。
// serde_json Map 两侧都是字典序（BTreeMap 默认），序列化字节序一致。

/// rustuya `NO_PROTOCOL_HEADER_CMDS`：这些命令的载荷**不加** "3.3" 协议头
/// （DpQuery/DpQueryNew/UpdateDps/HeartBeat/SessKeyNeg×3/LanExtStream）。
pub(crate) const NO_PROTOCOL_HEADER_CMDS: &[u32] =
    &[0x0a, 0x10, 0x12, 0x09, 0x03, 0x04, 0x05, 0x40];

/// v3.3 `generate_payload`（Control / UpdateDps 子集，镜像 rustuya 同名逻辑）：
/// 返回 (effective_cmd, 载荷 JSON)。
/// - `control`（0x07）：基础载荷去掉 `gwId`；
/// - `update_dps`（0x12）：只留 `cid`，插入 `dpId`（data 或缺省 [18,19,20]）。
pub(crate) fn command_payload_33(
    device_id: &str,
    cid: Option<&str>,
    t: u64,
    command: &str,
    dps: serde_json::Value,
) -> Result<(u32, serde_json::Value), String> {
    use serde_json::{Map, Value};

    let mut payload = Map::new();
    payload.insert("gwId".into(), device_id.into());
    payload.insert("devId".into(), cid.unwrap_or(device_id).into());
    payload.insert("uid".into(), device_id.into());
    if let Some(c) = cid {
        payload.insert("cid".into(), c.into());
    }
    payload.insert("t".into(), t.to_string().into());
    payload.insert("dps".into(), dps.clone());

    match command {
        "control" => {
            payload.remove("gwId");
            Ok((0x07, Value::Object(payload)))
        }
        "update_dps" => {
            payload.retain(|k, _| k == "cid");
            let dp_id = if !dps.is_null() {
                dps
            } else {
                serde_json::json!([18, 19, 20])
            };
            payload.insert("dpId".into(), dp_id);
            Ok((0x12, Value::Object(payload)))
        }
        other => Err(format!(
            "暂不支持的 v3.3 命令：{other}（control / update_dps）"
        )),
    }
}

/// v3.3/v3.4 `pack_payload`：AES-128-ECB+PKCS7 加密 + 非裸头命令的协议头。
/// **头的位置两代不同**（rustuya 对拍实测差异，S3i 勘误）：
/// - v3.3：先加密、后拼头（头以明文出现在密文外）；
/// - v3.4：先拼头、再整体加密（头进密文）。
fn pack_payload_3x(
    payload_json: &serde_json::Value,
    cmd: u32,
    key: &[u8],
    version_tag: &str,
    header_on_plaintext: bool,
) -> Result<Vec<u8>, String> {
    let use_header = !NO_PROTOCOL_HEADER_CMDS.contains(&cmd);
    let mut plaintext = payload_json.to_string().into_bytes();
    if header_on_plaintext && use_header {
        let mut with_header = version_tag.as_bytes().to_vec();
        with_header.extend_from_slice(&[0u8; 12]);
        with_header.extend_from_slice(&plaintext);
        plaintext = with_header;
    }
    let mut packed = aes_ecb_encrypt_pkcs7(key, &plaintext)?;
    if !header_on_plaintext && use_header {
        let mut with_header = version_tag.as_bytes().to_vec();
        with_header.extend_from_slice(&[0u8; 12]);
        with_header.extend_from_slice(&packed);
        packed = with_header;
    }
    Ok(packed)
}

/// v3.3 控制命令**完整帧**：载荷 → ECB 加密 → 头拼在密文外（v3.3 `pack_payload`
/// 同款）→ 0x55AA 帧（v3.3 `get_hmac_key` 返回 None → CRC32 尾）。
/// 返回 (effective_cmd, frame)。
pub(crate) fn command_frame_33(
    seqno: u32,
    device_id: &str,
    cid: Option<&str>,
    t: u64,
    command: &str,
    dps: serde_json::Value,
    key: &[u8],
) -> Result<(u32, Vec<u8>), String> {
    let (cmd, payload) = command_payload_33(device_id, cid, t, command, dps)?;
    let packed = pack_payload_3x(&payload, cmd, key, "3.3", false)?;
    let frame = pack_frame_55aa(seqno, cmd, &packed, None)?;
    Ok((cmd, frame))
}

// ============================================================
// S3i：v3.4 TCP 命令帧（会话密钥模式；rustuya ProtocolV34 对拍）
// ============================================================

/// v3.4 `generate_payload`（control / update_dps / heartbeat 子集，镜像
/// rustuya `ProtocolV34`）：
/// - `control`：**modern envelope** `{protocol:5, t(数值!), data:{cid?,ctype?,dps?}}`，
///   effective cmd = ControlNew (0x0d)；注意 v3.4 的 `t` 是数字（v3.3 是字符串）；
/// - `update_dps`：只留 `cid` + `dpId`（data 或缺省 [18,19,20]，effective 0x12）；
/// - `heartbeat`：`strip_status_heartbeat` → `{gwId, devId}`（去 uid/t）。
pub(crate) fn command_payload_34(
    device_id: &str,
    cid: Option<&str>,
    t: u64,
    command: &str,
    dps: serde_json::Value,
) -> Result<(u32, serde_json::Value), String> {
    use serde_json::{Map, Value};

    let has_dps = !dps.is_null();
    match command {
        "control" => {
            let mut data_obj = Map::new();
            if let Some(c) = cid {
                data_obj.insert("cid".into(), c.into());
                data_obj.insert("ctype".into(), 0.into());
            }
            if has_dps {
                data_obj.insert("dps".into(), dps);
            }
            let mut payload = Map::new();
            payload.insert("protocol".into(), 5.into());
            payload.insert("t".into(), t.into());
            payload.insert("data".into(), Value::Object(data_obj));
            Ok((0x0d, Value::Object(payload)))
        }
        "update_dps" | "heartbeat" => {
            let mut payload = Map::new();
            payload.insert("gwId".into(), device_id.into());
            payload.insert("devId".into(), cid.unwrap_or(device_id).into());
            payload.insert("uid".into(), device_id.into());
            if let Some(c) = cid {
                payload.insert("cid".into(), c.into());
            }
            payload.insert("t".into(), t.to_string().into());
            // rustuya `create_base_payload`：data 为 None（dps null）时不插入 dps 键。
            if has_dps {
                payload.insert("dps".into(), dps.clone());
            }
            if command == "update_dps" {
                payload.retain(|k, _| k == "cid");
                let dp_id = if has_dps {
                    dps
                } else {
                    serde_json::json!([18, 19, 20])
                };
                payload.insert("dpId".into(), dp_id);
                Ok((0x12, Value::Object(payload)))
            } else {
                payload.remove("uid");
                payload.remove("t");
                Ok((0x09, Value::Object(payload)))
            }
        }
        other => Err(format!(
            "暂不支持的 v3.4 命令：{other}（control / update_dps / heartbeat）"
        )),
    }
}

/// v3.4 控制命令**完整帧**：载荷 → ECB 加密（+ "3.4" 头）→ 0x55AA **HMAC 尾**
/// 帧（v3.4 `get_hmac_key` 返回 Some(key)，与 v3.3 的 CRC 尾不同）。`key` 为
/// 会话密钥 blob（协商后）或 local_key（协商前）。返回 (effective_cmd, frame)。
pub(crate) fn command_frame_34(
    seqno: u32,
    device_id: &str,
    cid: Option<&str>,
    t: u64,
    command: &str,
    dps: serde_json::Value,
    key: &[u8],
) -> Result<(u32, Vec<u8>), String> {
    let (cmd, payload) = command_payload_34(device_id, cid, t, command, dps)?;
    let packed = pack_payload_3x(&payload, cmd, key, "3.4", true)?;
    let frame = pack_frame_55aa(seqno, cmd, &packed, Some(key))?;
    Ok((cmd, frame))
}
