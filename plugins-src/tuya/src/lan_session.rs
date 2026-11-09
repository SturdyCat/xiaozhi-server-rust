//! S3i/S3j/S3s：Tuya LAN **v3.4 / v3.5 TCP 会话层状态机**（纯函数为主；仅
//! [`random_nonce`] 经宿主 `host-core.random` 取熵，其余无 I/O 无随机源）。
//!
//! 规格权威 = 外部 crate rustuya 的 `device/actor.rs::negotiate_session_key`
//! + `protocol/v34.rs` / `v35.rs`（prepare/verify/finalize/encrypt_session_key）
//! + `crypto.rs`（AES-128-ECB / AES-128-GCM）。v3.4/v3.5 TCP 连接建立后必须先
//! 协商会话密钥，之后所有帧（心跳/控制）都改用会话密钥（`get_cipher` 取
//! `session_key`；HMAC 尾 / GCM 密钥同一 blob）。
//!
//! 三步握手（两代共享验证与派生，仅「打包 + 密钥库形态」不同）：
//! 1. 客户端 → 设备：cmd 0x03（SessKeyNegStart），payload = 16B 本地随机数。
//!    v3.4：nonce 经 local_key ECB+PKCS7 加密 + 0x55AA HMAC 尾；v3.5：nonce
//!    **明文**进 0x6699 帧（帧级 AES-128-GCM，nonce=调用方传 IV）；
//! 2. 设备 → 客户端：cmd 0x04（SessKeyNegResp），解密后 payload =
//!    `remote_nonce(16) || HMAC-SHA256(local_key, local_nonce)(32)`，验证不过即断链；
//! 3. 客户端 → 设备：cmd 0x05（SessKeyNegFinish），payload = finish HMAC。
//!    会话密钥 = local_nonce ⊕ remote_nonce 循环 XOR。
//!    **密钥库形态（blob）**：v3.4 = `ECB(local_key, 会话密钥)` 无填充；
//!    v3.5 = `GCM(local_key, nonce=local_nonce[..12])密文前 16 字节`（不带 tag，
//!    rustuya `encrypt_session_key` 的 `[12..28]` 切片怪癖）——设备侧同法推导，
//!    两侧都用这个 blob 当后续帧的密钥（`get_cipher` 同口径）。
//!
//! nonce / IV：对拍测试用固定向量；生产编排的 nonce 经宿主随机源取熵（S3s
//! 固定向量）。宿主侧对拍：`plugins/wasm/tests/tuya_lan_session.rs` 用 rustuya
//! 本尊（TuyaProtocol trait）逐字节对拍。
#![allow(deprecated)]

use aes_gcm::aead::{AeadInPlace, Nonce};
use aes_gcm::Aes128Gcm;
// 两个 KeyInit 分属两棵 digest 树：aes_gcm 用于 Aes128Gcm，hmac(digest) 用于 Hmac。
use aes_gcm::aead::KeyInit as AeadKeyInit;
use hmac::digest::Mac as MacTrait;
use hmac::Hmac;
use sha2::{Digest as _, Sha256};

use xiaozhi_plugin_sdk::xiaozhi::plugin::host_core as host;

use crate::lan_codec::{pack_frame_55aa, parse_frame_55aa_exact};
use crate::lan_crypto::{aes_ecb_decrypt_strip, aes_ecb_encrypt};
use crate::lan_frame_6699::pack_frame_6699;

pub(crate) const SESS_KEY_NEG_START: u32 = 0x03;
/// 设备 → 客户端方向，guest 只验证不构造；保留作协议文档（闲置属预期）。
#[allow(dead_code)]
pub(crate) const SESS_KEY_NEG_RESP: u32 = 0x04;
pub(crate) const SESS_KEY_NEG_FINISH: u32 = 0x05;

const NONCE_LEN: usize = 16;
const RESP_MIN_LEN: usize = 48; // remote_nonce(16) + hmac(32)
const KEY_LEN: usize = 16;
const IV_LEN: usize = 12;

pub(crate) fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key).map_err(|_| "HMAC 密钥长度非法".to_string())?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

/// rustuya `TuyaCipher::encrypt` 的 GCM 形态：返回 `iv(12) || ct || tag(16)`。
fn gcm_encrypt(key: &[u8], iv12: &[u8], aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    if key.len() != KEY_LEN {
        return Err(format!("GCM 密钥必须 16 字节（当前 {}）", key.len()));
    }
    if iv12.len() != IV_LEN {
        return Err(format!("IV 必须 {IV_LEN} 字节（当前 {}）", iv12.len()));
    }
    let cipher = <Aes128Gcm as AeadKeyInit>::new_from_slice(key)
        .map_err(|_| "GCM 密钥初始化失败".to_string())?;
    let mut buffer = plaintext.to_vec();
    let tag = cipher
        .encrypt_in_place_detached(
            Nonce::<Aes128Gcm>::from_slice(&iv12[..IV_LEN]),
            aad,
            &mut buffer,
        )
        .map_err(|_| "GCM 加密失败".to_string())?;
    let mut out = Vec::with_capacity(IV_LEN + buffer.len() + 16);
    out.extend_from_slice(&iv12[..IV_LEN]);
    out.extend_from_slice(&buffer);
    out.extend_from_slice(tag.as_slice());
    Ok(out)
}

/// 通用前置校验（两代共用）。
fn check_inputs(local_key: &[u8], local_nonce: &[u8]) -> Result<(), String> {
    if local_key.len() != KEY_LEN {
        return Err(format!(
            "local_key 必须 16 字节（当前 {}）",
            local_key.len()
        ));
    }
    if local_nonce.len() != NONCE_LEN {
        return Err(format!("nonce 必须 16 字节（当前 {}）", local_nonce.len()));
    }
    Ok(())
}

/// 第 2 步共享核心：验证回包 payload → 派生 finish HMAC 与会话密钥原始值。
/// 验证失败显式报错（对齐 rustuya `KeyOrVersionError` 的断链语义）。
fn verify_and_derive(
    local_key: &[u8],
    local_nonce: &[u8],
    resp_payload: &[u8],
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
    if resp_payload.len() < RESP_MIN_LEN {
        return Err(format!(
            "会话回包过短（{} < {RESP_MIN_LEN}）",
            resp_payload.len()
        ));
    }
    let remote_nonce = &resp_payload[..NONCE_LEN];
    let remote_hmac = &resp_payload[NONCE_LEN..RESP_MIN_LEN];
    let expect = hmac_sha256(local_key, local_nonce)?;
    if expect.as_slice() != remote_hmac {
        return Err("会话回包 HMAC 验证失败（密钥不匹配或被篡改）".to_string());
    }
    // finish HMAC 覆盖 remote_nonce；会话密钥 = XOR 逐字节循环 remote_nonce。
    let finish_hmac = hmac_sha256(local_key, remote_nonce)?;
    let session_key_raw: Vec<u8> = local_nonce
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ remote_nonce[i % remote_nonce.len()])
        .collect();
    Ok((remote_nonce.to_vec(), finish_hmac, session_key_raw))
}

/// 第 3 步完成后的全量产物：finish 帧 + 密钥库 blob + 中间量（对拍与编排都要用）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SessionFinish {
    /// 设备随机数（resp payload 前 16 字节，验证通过后回传）。
    pub remote_nonce: Vec<u8>,
    /// 发给设备的 SessKeyNegFinish 完整帧（cmd 0x05）。
    pub frame: Vec<u8>,
    /// 密钥库形态（16B）：v3.4 = ECB 无填充；v3.5 = GCM 密文前 16 字节。
    /// 后续帧把它当 AES/HMAC/GCM 密钥（rustuya `get_cipher` 同口径）。
    pub session_key_blob: Vec<u8>,
}

/// **v3.4** 第 1 步：SessKeyNegStart 完整帧（0x55AA HMAC 尾，payload =
/// nonce 的 ECB+PKCS7 密文）。
pub(crate) fn session_start_frame(
    seqno: u32,
    local_key: &[u8],
    local_nonce: &[u8],
) -> Result<Vec<u8>, String> {
    check_inputs(local_key, local_nonce)?;
    let payload = aes_ecb_encrypt(local_key, local_nonce, true)?;
    pack_frame_55aa(seqno, SESS_KEY_NEG_START, &payload, Some(local_key))
}

/// **v3.4** 第 2+3 步：验证回包 → ECB blob → SessKeyNegFinish 帧（0x55AA HMAC 尾，
/// payload = finish_hmac 的 ECB+PKCS7 密文）。
pub(crate) fn session_finish(
    seqno: u32,
    local_key: &[u8],
    local_nonce: &[u8],
    resp_payload: &[u8],
) -> Result<SessionFinish, String> {
    check_inputs(local_key, local_nonce)?;
    let (remote_nonce, finish_hmac, session_key_raw) =
        verify_and_derive(local_key, local_nonce, resp_payload)?;
    // 密钥库形态：ECB(local_key, session_key_raw)，无填充（16B 对齐）。
    let blob = aes_ecb_encrypt(local_key, &session_key_raw, false)?;
    let finish_payload = aes_ecb_encrypt(local_key, &finish_hmac, true)?;
    let frame = pack_frame_55aa(seqno, SESS_KEY_NEG_FINISH, &finish_payload, Some(local_key))?;
    Ok(SessionFinish {
        remote_nonce,
        frame,
        session_key_blob: blob,
    })
}

/// **v3.5** 第 1 步：SessKeyNegStart 完整帧（0x6699/GCM，payload = nonce **明文**
/// ——v3.5 `pack_payload` 不做帧前加密，加密在帧级 GCM）。
pub(crate) fn session_start_frame_35(
    seqno: u32,
    local_key: &[u8],
    local_nonce: &[u8],
    iv: &[u8],
) -> Result<Vec<u8>, String> {
    check_inputs(local_key, local_nonce)?;
    pack_frame_6699(seqno, SESS_KEY_NEG_START, local_nonce, None, local_key, iv)
}

/// **v3.5** 第 2+3 步：验证回包（与 v3.4 同款）→ GCM blob → SessKeyNegFinish 帧
/// （0x6699/GCM，payload = finish_hmac 明文）。
pub(crate) fn session_finish_35(
    seqno: u32,
    local_key: &[u8],
    local_nonce: &[u8],
    resp_payload: &[u8],
    iv: &[u8],
) -> Result<SessionFinish, String> {
    check_inputs(local_key, local_nonce)?;
    let (remote_nonce, finish_hmac, session_key_raw) =
        verify_and_derive(local_key, local_nonce, resp_payload)?;
    // 密钥库形态：GCM(local_key, nonce=local_nonce[..12]) 的 iv||ct||tag 取 [12..28]
    // （rustuya v35 `encrypt_session_key` 切片怪癖：密文前 16 字节，无 tag）。
    let blob_iv = &local_nonce[..IV_LEN];
    let encrypted = gcm_encrypt(local_key, blob_iv, &[], &session_key_raw)?;
    let blob = encrypted[IV_LEN..IV_LEN + KEY_LEN].to_vec();
    let frame = pack_frame_6699(
        seqno,
        SESS_KEY_NEG_FINISH,
        &finish_hmac,
        None,
        local_key,
        iv,
    )?;
    Ok(SessionFinish {
        remote_nonce,
        frame,
        session_key_blob: blob,
    })
}

// ==================================================================
// S3m：会话编排辅助（wire 帧步进 + nonce 派生）
// ==================================================================

/// 从 **wire 原始帧**步进到已解密的会话回包 payload：
/// `parse_frame_55aa_exact`（HMAC 尾，**不剥 retcode**——启发式作用于二进制
/// 密文属 rustuya 接收路径的实现缺陷，编排按协议语义实现）→ ECB 解密去填充
/// → 校验 cmd == SessKeyNegResp → 返回 (帧 seqno, ≥48B payload)。
pub(crate) fn wire_resp_to_payload(
    wire_frame: &[u8],
    local_key: &[u8],
) -> Result<(u32, Vec<u8>), String> {
    let frame = parse_frame_55aa_exact(wire_frame, Some(local_key))?;
    if frame.cmd != SESS_KEY_NEG_RESP {
        return Err(format!(
            "期待 SessKeyNegResp(0x04)，收到 cmd={:#04x}",
            frame.cmd
        ));
    }
    let payload = aes_ecb_decrypt_strip(local_key, &frame.payload)?;
    if payload.len() < RESP_MIN_LEN {
        return Err(format!(
            "会话回包过短（{} < {RESP_MIN_LEN}）",
            payload.len()
        ));
    }
    Ok((frame.seqno, payload))
}

/// **v3.5** 从 wire 原始帧步进到已解密的会话回包 payload：
/// `parse_frame_6699_exact`（GCM under local_key，IV 随帧、**不剥 retcode**——
/// 同 `wire_resp_to_payload` 口径）→ 校验 cmd == SessKeyNegResp → 返回
/// (帧 seqno, ≥48B payload)。v3.5 的载荷加密在帧级完成，这里不再做二次解密。
pub(crate) fn wire_resp_to_payload_35(
    wire_frame: &[u8],
    local_key: &[u8],
) -> Result<(u32, Vec<u8>), String> {
    let frame = crate::lan_frame_6699::parse_frame_6699_exact(wire_frame, local_key)?;
    if frame.cmd != SESS_KEY_NEG_RESP {
        return Err(format!(
            "期待 SessKeyNegResp(0x04)，收到 cmd={:#04x}",
            frame.cmd
        ));
    }
    if frame.payload.len() < RESP_MIN_LEN {
        return Err(format!(
            "会话回包过短（{} < {RESP_MIN_LEN}）",
            frame.payload.len()
        ));
    }
    Ok((frame.seqno, frame.payload))
}

/// 生产 nonce：优先宿主 OS 熵源（S3s `host-core.random`，16 字节）——guest 自身
/// 无熵源，宿主随机源是唯一不可预测来源；随机源不可用时退化为确定性派生
/// （[`derive_nonce`]），保证不因熵源故障无法建链（但可预测性仅够防重放）。
pub(crate) fn random_nonce(local_key: &[u8], device_id: &str, now_ms: u64) -> [u8; KEY_LEN] {
    if let Ok(bytes) = host::random(KEY_LEN as u32) {
        if bytes.len() == KEY_LEN {
            let mut nonce = [0u8; KEY_LEN];
            nonce.copy_from_slice(&bytes);
            return nonce;
        }
    }
    derive_nonce(local_key, device_id, now_ms)
}

/// guest 无随机源：确定性回退派生——`SHA-256(local_key ‖ device_id ‖ now_ms)[..16]`。
/// 熵来自宿主墙上时钟（仅当 OS 随机源不可用时使用；只保证每次连接不同，防跨
/// 连接重放同一握手，不保证不可预测）。
pub(crate) fn derive_nonce(local_key: &[u8], device_id: &str, now_ms: u64) -> [u8; KEY_LEN] {
    let mut hasher = Sha256::new();
    hasher.update(local_key);
    hasher.update(device_id.as_bytes());
    hasher.update(now_ms.to_be_bytes());
    let digest = hasher.finalize();
    let mut nonce = [0u8; KEY_LEN];
    nonce.copy_from_slice(&digest[..KEY_LEN]);
    nonce
}

/// guest 无随机源：v3.5 帧级 GCM 的 IV 由确定性材料派生——
/// `SHA-256(seed ‖ seqno_be ‖ tag)[..12]`。
///
/// 唯一性依据（GCM 硬要求）：握手阶段 `seed` = 本连接 nonce（每连接不同，含
/// 宿主时钟），Established 阶段 `seed` = 会话密钥 blob（由两侧 nonce 派生，每
/// 连接不同），且同连接内 seqno 单调递增；`tag` 再区分同 seqno 的不同用途帧。
/// IV 不要求保密，只要求同密钥下不重复。
pub(crate) fn derive_iv(seed: &[u8], seqno: u32, tag: &str) -> [u8; IV_LEN] {
    let mut hasher = Sha256::new();
    hasher.update(seed);
    hasher.update(seqno.to_be_bytes());
    hasher.update(tag.as_bytes());
    let digest = hasher.finalize();
    let mut iv = [0u8; IV_LEN];
    iv.copy_from_slice(&digest[..IV_LEN]);
    iv
}
