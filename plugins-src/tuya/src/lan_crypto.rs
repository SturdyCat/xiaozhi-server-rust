//! LAN 域 AES-128-ECB 原语（加密 / 解密去填充），tuya 域共用。
//!
//! 规格对齐 rustuya `crypto.rs::TuyaCipher` 的 ECB 路径：
//! - 加密：`padding=true` 补 PKCS7 式填充（`pad_len = 16 - len%16`，对齐补满块）；
//!   `padding=false` 要求 16 字节对齐（v3.4 会话密钥库 blob 用）；
//! - 解密：长度必须 16 对齐；逐块解密后按末字节验证并剥离 PKCS7 填充
//!   （pad 非法 → 解密失败，与 rustuya `DecryptionFailed` 同口径）。

pub(crate) fn aes_ecb_encrypt(key: &[u8], data: &[u8], padding: bool) -> Result<Vec<u8>, String> {
    use aes::cipher::{consts::U16, Array, BlockCipherEncrypt as _, InOut, KeyInit as AesInit};
    use aes::Aes128;

    if key.len() != 16 {
        return Err(format!("ECB 密钥必须 16 字节（当前 {}）", key.len()));
    }
    let mut buffer = data.to_vec();
    if padding {
        let pad_len = 16 - data.len() % 16;
        buffer.resize(data.len() + pad_len, pad_len as u8);
    } else if data.len() % 16 != 0 {
        return Err(format!(
            "ECB 无填充模式要求 16 字节对齐（当前 {}）",
            data.len()
        ));
    }
    let cipher =
        <Aes128 as AesInit>::new_from_slice(key).map_err(|_| "ECB 密钥初始化失败".to_string())?;
    for chunk in buffer.chunks_exact_mut(16) {
        let block = <&mut Array<u8, U16>>::try_from(chunk).map_err(|_| "块长度错误".to_string())?;
        cipher.encrypt_block_inout(InOut::from(block));
    }
    Ok(buffer)
}

/// rustuya `cipher.encrypt(key, data, padding=true)` 的 PKCS7 包装。
pub(crate) fn aes_ecb_encrypt_pkcs7(key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    aes_ecb_encrypt(key, data, true)
}

/// AES-128-ECB 解密 + PKCS7 去填充（rustuya `cipher.decrypt` ECB 路径同口径）。
pub(crate) fn aes_ecb_decrypt_strip(key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    use aes::cipher::{consts::U16, Array, BlockCipherDecrypt as _, InOut, KeyInit as AesInit};
    use aes::Aes128;

    if key.len() != 16 {
        return Err(format!("ECB 密钥必须 16 字节（当前 {}）", key.len()));
    }
    if data.is_empty() {
        return Ok(Vec::new());
    }
    if data.len() % 16 != 0 {
        return Err(format!("ECB 解密要求 16 字节对齐（当前 {}）", data.len()));
    }
    let cipher =
        <Aes128 as AesInit>::new_from_slice(key).map_err(|_| "ECB 密钥初始化失败".to_string())?;
    let mut buffer = data.to_vec();
    for chunk in buffer.chunks_exact_mut(16) {
        let block = <&mut Array<u8, U16>>::try_from(chunk).map_err(|_| "块长度错误".to_string())?;
        cipher.decrypt_block_inout(InOut::from(block));
    }
    // PKCS7 验证与剥离（rustuya 同款：pad 非法整体判失败）。
    let pad_len = *buffer.last().expect("非空已检") as usize;
    if pad_len == 0 || pad_len > 16 || pad_len > buffer.len() {
        return Err("PKCS7 填充非法（解密失败）".to_string());
    }
    for i in 0..pad_len {
        if buffer[buffer.len() - 1 - i] != pad_len as u8 {
            return Err("PKCS7 填充字节不一致（解密失败）".to_string());
        }
    }
    buffer.truncate(buffer.len() - pad_len);
    Ok(buffer)
}
