//! S3t：Noise Protocol Framework **NNpsk0** 的密码学原语（guest 纯 Rust 栈）。
//!
//! 规格权威 = Noise Protocol Framework rev 34 与 ESPHome 的会话参数（aioesphomeapi
//! `_frame_helper/noise_encryption.py` / ESPHome 固件同款）：
//! - 协议名 `Noise_NNpsk0_25519_ChaChaPoly_SHA256`（[`PROTOCOL_NAME`]）；
//! - CipherState = ChaCha20-Poly1305；Noise nonce = 4 个零字节 ‖ u64 **LE**（与
//!   aioesphomeapi `<LQ` 打包一致），每帧自增；
//! - 对称状态 = SHA-256 哈希 + 链密钥（Noise §5.2/§5.3）：`mix_key` 的 HKDF 不是
//!   RFC5869，而是 `temp = HMAC(ck, ikm)` 之后按输出序号逐段 HMAC（`[1]` /
//!   `out1‖[2]` / `out2‖[3]`），与 `noise-protocol` crate 的 `Hash::hkdf` 同口径；
//! - DH = X25519；临时密钥由宿主 `host-core.random` 生成（guest 自身无熵源），
//!   本模块只接受显式私钥字节（可序列化状态，事件间全部经 kv 持久化）。
//!
//! 依赖：sha2 + hmac + chacha20poly1305 + x25519-dalek（全纯 Rust，wasm32 可编，
//! 关闭 chacha20poly1305 默认 getrandom——guest 随机源只走宿主）。

use chacha20poly1305::aead::{AeadInPlace, KeyInit as AeadKeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce, Tag};
use hmac::digest::{KeyInit as HmacKeyInit, Mac as MacTrait};
use hmac::Hmac;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

/// ESPHome 的 Noise 协议名（aioesphomeapi `NOISE_PROTOCOL_NAME` 同值）。
pub(crate) const PROTOCOL_NAME: &[u8] = b"Noise_NNpsk0_25519_ChaChaPoly_SHA256";
/// 握手 prologue（ESPHome 固件/aioesphomeapi 均为 `b"NoiseAPIInit\0\0"`）。
pub(crate) const PROLOGUE: &[u8] = b"NoiseAPIInit\0\0";
pub(crate) const KEY_LEN: usize = 32;
pub(crate) const HASH_LEN: usize = 32;
pub(crate) const TAG_LEN: usize = 16;
/// X25519 公钥/共享密钥字节数。
pub(crate) const DH_LEN: usize = 32;

// ------------------------------------------------------------------
// 哈希 / HMAC / DH
// ------------------------------------------------------------------

/// SHA-256(`parts` 顺次拼接)：Noise 的 `MixHash` / `HASH(h‖data)` 单源。
pub(crate) fn sha256(parts: &[&[u8]]) -> [u8; HASH_LEN] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

/// HMAC-SHA256(`key`, `parts` 顺次拼接)：Noise HKDF 的单块构造。
pub(crate) fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; HASH_LEN] {
    let mut mac =
        <Hmac<Sha256> as HmacKeyInit>::new_from_slice(key).expect("HMAC-SHA256 接受任意长度密钥");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

/// X25519 公钥派生（clamp 由 x25519-dalek 在派生时完成）。
pub(crate) fn x25519_public(secret: &[u8; DH_LEN]) -> [u8; DH_LEN] {
    let secret = StaticSecret::from(*secret);
    *PublicKey::from(&secret).as_bytes()
}

/// X25519 Diffie-Hellman；全零共享密钥（低阶点）显式拒绝。
pub(crate) fn x25519_dh(
    secret: &[u8; DH_LEN],
    peer_public: &[u8; DH_LEN],
) -> Result<[u8; DH_LEN], String> {
    let secret = StaticSecret::from(*secret);
    let shared = secret.diffie_hellman(&PublicKey::from(*peer_public));
    let bytes = shared.as_bytes();
    if bytes.iter().all(|b| *b == 0) {
        return Err("X25519 共享密钥为全零（对端公钥非法）".to_string());
    }
    Ok(*bytes)
}

// ------------------------------------------------------------------
// HKDF（Noise 版）
// ------------------------------------------------------------------

/// `HKDF(ck, ikm) -> (out1, out2)`：`temp = HMAC(ck, ikm)`，
/// `out1 = HMAC(temp, [1])`，`out2 = HMAC(temp, out1‖[2])`。
pub(crate) fn hkdf2(chaining_key: &[u8; HASH_LEN], ikm: &[u8]) -> ([u8; HASH_LEN], [u8; HASH_LEN]) {
    let temp = hmac_sha256(chaining_key, &[ikm]);
    let out1 = hmac_sha256(&temp, &[&[1u8]]);
    let out2 = hmac_sha256(&temp, &[&out1, &[2u8]]);
    (out1, out2)
}

/// `HKDF(ck, ikm) -> (out1, out2, out3)`（`MixKeyAndHash` 用）。
pub(crate) fn hkdf3(
    chaining_key: &[u8; HASH_LEN],
    ikm: &[u8],
) -> ([u8; HASH_LEN], [u8; HASH_LEN], [u8; HASH_LEN]) {
    let temp = hmac_sha256(chaining_key, &[ikm]);
    let out1 = hmac_sha256(&temp, &[&[1u8]]);
    let out2 = hmac_sha256(&temp, &[&out1, &[2u8]]);
    let out3 = hmac_sha256(&temp, &[&out2, &[3u8]]);
    (out1, out2, out3)
}

// ------------------------------------------------------------------
// 密钥状态 / 对称状态
// ------------------------------------------------------------------

/// Noise `CipherState`：一把 ChaCha20-Poly1305 密钥 + 自增 nonce。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CipherState {
    key: [u8; KEY_LEN],
    nonce: u64,
}

impl CipherState {
    pub(crate) fn new(key: [u8; KEY_LEN]) -> Self {
        Self { key, nonce: 0 }
    }

    pub(crate) fn key(&self) -> &[u8; KEY_LEN] {
        &self.key
    }

    pub(crate) fn nonce(&self) -> u64 {
        self.nonce
    }

    pub(crate) fn set_nonce(&mut self, nonce: u64) {
        self.nonce = nonce;
    }

    /// 12 字节 ChaCha20-Poly1305 nonce：4 零字节 ‖ u64 LE（Noise / aioesphomeapi 同口径）。
    fn full_nonce(nonce: u64) -> [u8; 12] {
        let mut full = [0u8; 12];
        full[4..].copy_from_slice(&nonce.to_le_bytes());
        full
    }

    /// AEAD 加密：返回 `密文 ‖ 16 字节 tag`，nonce 自增。
    pub(crate) fn encrypt_with_ad(
        &mut self,
        associated_data: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, String> {
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let mut buffer = plaintext.to_vec();
        let tag = cipher
            .encrypt_in_place_detached(
                Nonce::from_slice(&Self::full_nonce(self.nonce)),
                associated_data,
                &mut buffer,
            )
            .map_err(|_| "AEAD 加密失败".to_string())?;
        buffer.extend_from_slice(&tag);
        self.nonce += 1;
        Ok(buffer)
    }

    /// AEAD 解密（认证失败 = 密钥不匹配或密文被篡改），nonce 自增。
    pub(crate) fn decrypt_with_ad(
        &mut self,
        associated_data: &[u8],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, String> {
        if ciphertext.len() < TAG_LEN {
            return Err(format!(
                "密文过短（{} < {TAG_LEN} 字节 tag）",
                ciphertext.len()
            ));
        }
        let (body, tag) = ciphertext.split_at(ciphertext.len() - TAG_LEN);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&self.key));
        let mut buffer = body.to_vec();
        cipher
            .decrypt_in_place_detached(
                Nonce::from_slice(&Self::full_nonce(self.nonce)),
                associated_data,
                &mut buffer,
                Tag::from_slice(tag),
            )
            .map_err(|_| "AEAD 认证失败（密钥不匹配或密文被篡改）".to_string())?;
        self.nonce += 1;
        Ok(buffer)
    }
}

/// Noise `SymmetricState`：握手哈希 `h` + 链密钥 `ck` + 可选 CipherState。
#[derive(Clone, Debug)]
pub(crate) struct SymmetricState {
    hash: [u8; HASH_LEN],
    chaining_key: [u8; HASH_LEN],
    cipher: Option<CipherState>,
}

impl SymmetricState {
    /// `InitializeSymmetric(protocol_name)` + `MixHash(prologue)`（NNpsk0 无 pre-message）。
    pub(crate) fn new() -> Self {
        let hash = if PROTOCOL_NAME.len() <= HASH_LEN {
            let mut padded = [0u8; HASH_LEN];
            padded[..PROTOCOL_NAME.len()].copy_from_slice(PROTOCOL_NAME);
            padded
        } else {
            sha256(&[PROTOCOL_NAME])
        };
        let mut state = Self {
            hash,
            chaining_key: hash,
            cipher: None,
        };
        state.mix_hash(PROLOGUE);
        state
    }

    pub(crate) fn hash(&self) -> &[u8; HASH_LEN] {
        &self.hash
    }

    pub(crate) fn chaining_key(&self) -> &[u8; HASH_LEN] {
        &self.chaining_key
    }

    pub(crate) fn cipher(&self) -> Option<&CipherState> {
        self.cipher.as_ref()
    }

    pub(crate) fn set_cipher(&mut self, cipher: Option<CipherState>) {
        self.cipher = cipher;
    }

    pub(crate) fn set_hash(&mut self, hash: [u8; HASH_LEN]) {
        self.hash = hash;
    }

    pub(crate) fn set_chaining_key(&mut self, chaining_key: [u8; HASH_LEN]) {
        self.chaining_key = chaining_key;
    }

    /// Noise `MixHash(data)`：`h = HASH(h ‖ data)`。
    pub(crate) fn mix_hash(&mut self, data: &[u8]) {
        self.hash = sha256(&[&self.hash, data]);
    }

    /// Noise `MixKey(ikm)`：`ck, k = HKDF(ck, ikm)`，装一把新 CipherState（nonce=0）。
    pub(crate) fn mix_key(&mut self, ikm: &[u8]) {
        let (ck, key) = hkdf2(&self.chaining_key, ikm);
        self.chaining_key = ck;
        self.cipher = Some(CipherState::new(key));
    }

    /// Noise `MixKeyAndHash(ikm)`：`ck, temp_h, temp_k = HKDF3(ck, ikm)`，
    /// `MixHash(temp_h)`，装 `temp_k` 的 CipherState（NNpsk0 的 psk token 用）。
    pub(crate) fn mix_key_and_hash(&mut self, ikm: &[u8]) {
        let (ck, temp_h, temp_k) = hkdf3(&self.chaining_key, ikm);
        self.chaining_key = ck;
        self.mix_hash(&temp_h);
        self.cipher = Some(CipherState::new(temp_k));
    }

    /// Noise `EncryptAndHash(plaintext)`：有密钥时以 `h` 作 AD 加密，再 `MixHash(密文)`。
    pub(crate) fn encrypt_and_hash(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let ciphertext = match self.cipher.as_mut() {
            Some(cipher) => cipher.encrypt_with_ad(&self.hash, plaintext)?,
            None => plaintext.to_vec(),
        };
        self.mix_hash(&ciphertext);
        Ok(ciphertext)
    }

    /// Noise `DecryptAndHash(ciphertext)`：以 `h` 作 AD 解密，再 `MixHash(密文)`。
    pub(crate) fn decrypt_and_hash(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        let plaintext = match self.cipher.as_mut() {
            Some(cipher) => cipher.decrypt_with_ad(&self.hash, ciphertext)?,
            None => ciphertext.to_vec(),
        };
        self.mix_hash(ciphertext);
        Ok(plaintext)
    }

    /// Noise `Split()`：`k1, k2 = HKDF(ck, 空)`；返回 (c1 发送侧候选, c2 接收侧候选)。
    pub(crate) fn split(&self) -> (CipherState, CipherState) {
        let (k1, k2) = hkdf2(&self.chaining_key, &[]);
        (CipherState::new(k1), CipherState::new(k2))
    }
}
