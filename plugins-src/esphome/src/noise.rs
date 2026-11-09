//! S3t：ESPHome **Noise NNpsk0 会话状态机**（initiator / responder 双角色）。
//!
//! 线格式在 [`crate::noise_wire`]，密码学原语在 [`crate::noise_crypto`]；本文件只做
//! 协议相位推进，状态**全部可序列化为 JSON**（`to_json` / `from_json`）落插件隔离 kv
//! ——事件间无内存态，与 tuya LAN 会话编排同纪律（`lib.rs` 的 `noise` 事件）。
//!
//! 握手（NNpsk0 = `-> psk, e` / `<- e, ee`）：
//! 1. initiator `start` → `initiator_hello()` 返回 ClientHello(`01 00 00`) + msg1 帧
//!    （msg1 = `e_pub(32) ‖ AEAD(tag 16)`，psk 在 E 之前混入）；
//! 2. responder `start` → 宿主发 ServerHello 帧（name/mac 身份）；`feed` 消费
//!    ClientHello 空帧 + msg1 帧，产出 msg2 帧（msg2 = `e_pub ‖ tag`，EE 混入）；
//! 3. initiator `feed` 消费 ServerHello 帧 + msg2 帧 → `Ready`；
//! 4. `Ready` 后 `seal` / `open` 走传输帧（`type‖len‖protobuf` 的 AEAD 封装）。
//!
//! 错误语义（fail-closed）：对端 MAC 失败 / 协议字节未知 / 帧标记非法都返回显式
//! Err（事件层写 `ok:false`），不降级明文；错误文本帧（`0x01 "Handshake MAC failure"`）
//! 按 aioesphomeapi 口径翻译成可读错误。

use serde_json::{json, Value};

use crate::hex::{hex_decode, hex_encode};
use crate::noise_crypto as crypto;
use crate::noise_wire as wire;

/// ESPHome noise psk 长度（base64 解码后固定 32 字节）。
pub(crate) const PSK_LEN: usize = crypto::KEY_LEN;

/// 会话角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// 主动连接方（客户端；aioesphomeapi / HA 侧行为）。
    Initiator,
    /// 被连接方（ESPHome 设备侧；本仓 native 桥的真实角色）。
    Responder,
}

impl Role {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        match text {
            "initiator" => Ok(Self::Initiator),
            "responder" => Ok(Self::Responder),
            other => Err(format!("未知角色：{other}（initiator|responder）")),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Initiator => "initiator",
            Self::Responder => "responder",
        }
    }
}

/// 握手相位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    /// initiator 等待 ServerHello 帧。
    AwaitServerHello,
    /// initiator 等待 msg2 帧。
    AwaitMsg2,
    /// responder 等待 ClientHello + msg1 帧。
    AwaitMsg1,
    /// 握手完成，传输密钥就绪。
    Ready,
}

impl Phase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::AwaitServerHello => "await-server-hello",
            Self::AwaitMsg2 => "await-msg2",
            Self::AwaitMsg1 => "await-msg1",
            Self::Ready => "ready",
        }
    }

    fn parse(text: &str) -> Result<Self, String> {
        match text {
            "await-server-hello" => Ok(Self::AwaitServerHello),
            "await-msg2" => Ok(Self::AwaitMsg2),
            "await-msg1" => Ok(Self::AwaitMsg1),
            "ready" => Ok(Self::Ready),
            other => Err(format!("非法相位：{other}")),
        }
    }
}

/// Noise 握手状态（含传输密钥与流式接收缓冲）。
pub(crate) struct Handshake {
    role: Role,
    phase: Phase,
    symmetric: crypto::SymmetricState,
    local_ephemeral: [u8; crypto::DH_LEN],
    peer_ephemeral: Option<[u8; crypto::DH_LEN]>,
    /// responder 在读到 msg1 前保留 psk（initiator 在写 msg1 时即消费）。
    psk: Option<[u8; crypto::KEY_LEN]>,
    send: Option<crypto::CipherState>,
    recv: Option<crypto::CipherState>,
    rx_buf: Vec<u8>,
    peer_name: Option<String>,
    peer_mac: Option<String>,
}

impl Handshake {
    /// 新建握手：临时密钥必须由调用方（宿主 `host-core.random`）提供，
    /// guest 自身无熵源，绝不在这里生成可预测密钥。
    pub(crate) fn start(
        role: Role,
        psk: [u8; crypto::KEY_LEN],
        ephemeral: [u8; crypto::DH_LEN],
    ) -> Self {
        Self {
            role,
            phase: match role {
                Role::Initiator => Phase::AwaitServerHello,
                Role::Responder => Phase::AwaitMsg1,
            },
            symmetric: crypto::SymmetricState::new(),
            local_ephemeral: ephemeral,
            peer_ephemeral: None,
            psk: Some(psk),
            send: None,
            recv: None,
            rx_buf: Vec::new(),
            peer_name: None,
            peer_mac: None,
        }
    }

    pub(crate) fn phase(&self) -> Phase {
        self.phase
    }

    pub(crate) fn peer_name(&self) -> Option<&str> {
        self.peer_name.as_deref()
    }

    pub(crate) fn peer_mac(&self) -> Option<&str> {
        self.peer_mac.as_deref()
    }

    /// initiator 首包：ClientHello(`01 00 00`) + msg1 帧（与 aioesphomeapi 首写同布局）。
    pub(crate) fn initiator_hello(&mut self) -> Result<Vec<u8>, String> {
        if self.role != Role::Initiator {
            return Err("只有 initiator 能发 ClientHello".to_string());
        }
        let msg1 = self.write_msg1()?;
        let mut out = wire::client_hello();
        out.extend_from_slice(&wire::handshake_frame(&msg1)?);
        Ok(out)
    }

    /// 写 msg1：`psk token` → `e`（MixHash + MixKey）→ 空载荷 EncryptAndHash。
    fn write_msg1(&mut self) -> Result<Vec<u8>, String> {
        let psk = self.psk.take().ok_or("msg1 已写出（psk 已消费）")?;
        self.symmetric.mix_key_and_hash(&psk);
        let public = crypto::x25519_public(&self.local_ephemeral);
        self.symmetric.mix_hash(&public);
        self.symmetric.mix_key(&public);
        let ciphertext = self.symmetric.encrypt_and_hash(&[])?;
        self.phase = Phase::AwaitServerHello;
        let mut message = public.to_vec();
        message.extend_from_slice(&ciphertext);
        Ok(message)
    }

    /// 读 msg1（responder）：psk token → 对端 `e`（MixHash + MixKey）→ DecryptAndHash。
    fn read_msg1(&mut self, message: &[u8]) -> Result<(), String> {
        if message.len() != crypto::DH_LEN + crypto::TAG_LEN {
            return Err(format!(
                "msg1 长度非法（{}，应为 {}）",
                message.len(),
                crypto::DH_LEN + crypto::TAG_LEN
            ));
        }
        let psk = self.psk.take().ok_or("msg1 已读过（psk 已消费）")?;
        self.symmetric.mix_key_and_hash(&psk);
        let mut peer = [0u8; crypto::DH_LEN];
        peer.copy_from_slice(&message[..crypto::DH_LEN]);
        self.symmetric.mix_hash(&peer);
        self.symmetric.mix_key(&peer);
        // 握手载荷按 ESPHome 协议约定为空；非空载荷解出后忽略（向前兼容）。
        self.symmetric
            .decrypt_and_hash(&message[crypto::DH_LEN..])?;
        self.peer_ephemeral = Some(peer);
        Ok(())
    }

    /// 写 msg2（responder）：`e`（MixHash + MixKey）→ `ee`（MixKey）→ 空载荷加密
    /// → Split（responder 发送 c2 / 接收 c1）。
    fn write_msg2(&mut self) -> Result<Vec<u8>, String> {
        let peer = self
            .peer_ephemeral
            .ok_or("msg2 前置条件缺失（未读到对端 e）")?;
        let public = crypto::x25519_public(&self.local_ephemeral);
        self.symmetric.mix_hash(&public);
        self.symmetric.mix_key(&public);
        let shared = crypto::x25519_dh(&self.local_ephemeral, &peer)?;
        self.symmetric.mix_key(&shared);
        let ciphertext = self.symmetric.encrypt_and_hash(&[])?;
        let (c1, c2) = self.symmetric.split();
        self.send = Some(c2);
        self.recv = Some(c1);
        self.phase = Phase::Ready;
        let mut message = public.to_vec();
        message.extend_from_slice(&ciphertext);
        Ok(message)
    }

    /// 读 msg2（initiator）：对端 `e`（MixHash + MixKey）→ `ee`（MixKey）→ 解密
    /// → Split（initiator 发送 c1 / 接收 c2）。
    fn read_msg2(&mut self, message: &[u8]) -> Result<(), String> {
        if message.len() != crypto::DH_LEN + crypto::TAG_LEN {
            return Err(format!(
                "msg2 长度非法（{}，应为 {}）",
                message.len(),
                crypto::DH_LEN + crypto::TAG_LEN
            ));
        }
        let mut peer = [0u8; crypto::DH_LEN];
        peer.copy_from_slice(&message[..crypto::DH_LEN]);
        self.symmetric.mix_hash(&peer);
        self.symmetric.mix_key(&peer);
        let shared = crypto::x25519_dh(&self.local_ephemeral, &peer)?;
        self.symmetric.mix_key(&shared);
        self.symmetric
            .decrypt_and_hash(&message[crypto::DH_LEN..])?;
        let (c1, c2) = self.symmetric.split();
        self.send = Some(c1);
        self.recv = Some(c2);
        self.peer_ephemeral = Some(peer);
        self.phase = Phase::Ready;
        Ok(())
    }

    /// 喂入对端原始字节（可半包/粘包），推进握手并返回需要写回的帧（可能为空）。
    ///
    /// 返回后 `phase()` 指示下一步：`Ready` 之后的对端字节请走 [`Self::open`]。
    pub(crate) fn feed(&mut self, data: &[u8]) -> Result<Vec<u8>, String> {
        if self.phase == Phase::Ready {
            return Err("握手已完成（传输帧请走 open）".to_string());
        }
        if self.rx_buf.len() + data.len() > wire::MAX_RX_BUF {
            return Err(format!("握手接收缓冲超限（> {} 字节）", wire::MAX_RX_BUF));
        }
        self.rx_buf.extend_from_slice(data);
        let mut out = Vec::new();
        while self.phase != Phase::Ready {
            let Some(content) = wire::next_frame(&mut self.rx_buf)? else {
                break; // 无完整帧：等待后续字节
            };
            if content.is_empty() {
                continue; // ClientHello 零长度帧（加密帧头的握手前导）
            }
            match self.phase {
                Phase::AwaitServerHello => {
                    if content[0] != wire::SERVER_HELLO_PROTOCOL {
                        return Err(format!(
                            "期待 ServerHello（首字节 0x01），收到 {:#04x}",
                            content[0]
                        ));
                    }
                    let info = wire::parse_server_hello(&content)?;
                    self.peer_name = Some(info.name);
                    self.peer_mac = info.mac;
                    self.phase = Phase::AwaitMsg2;
                }
                Phase::AwaitMsg2 => {
                    if content[0] != wire::HANDSHAKE_TYPE {
                        return Err(format!("握手失败：{}", wire::parse_error_frame(&content)));
                    }
                    self.read_msg2(&content[1..])?;
                }
                Phase::AwaitMsg1 => {
                    if content[0] != wire::HANDSHAKE_TYPE {
                        return Err(format!(
                            "期待握手帧（首字节 0x00），收到 {:#04x}",
                            content[0]
                        ));
                    }
                    self.read_msg1(&content[1..])?;
                    let msg2 = self.write_msg2()?;
                    out.extend_from_slice(&wire::handshake_frame(&msg2)?);
                }
                Phase::Ready => break,
            }
        }
        Ok(out)
    }

    /// 传输帧封装：`type‖len‖payload` 的 AEAD 加密 + 加密帧包装。
    pub(crate) fn seal(&mut self, message_type: u16, payload: &[u8]) -> Result<Vec<u8>, String> {
        if self.phase != Phase::Ready {
            return Err("会话未就绪（握手未完成）".to_string());
        }
        let plaintext = wire::transport_plaintext(message_type, payload)?;
        let send = self.send.as_mut().ok_or("发送密钥缺失")?;
        let ciphertext = send.encrypt_with_ad(&[], &plaintext)?;
        wire::frame_encode(&ciphertext)
    }

    /// 传输帧解封（可半包/粘包）：返回本次切出的完整消息 `(type, payload)`。
    pub(crate) fn open(&mut self, data: &[u8]) -> Result<Vec<(u16, Vec<u8>)>, String> {
        if self.phase != Phase::Ready {
            return Err("会话未就绪（握手未完成）".to_string());
        }
        if self.rx_buf.len() + data.len() > wire::MAX_RX_BUF {
            return Err(format!("接收缓冲超限（> {} 字节）", wire::MAX_RX_BUF));
        }
        self.rx_buf.extend_from_slice(data);
        let mut messages = Vec::new();
        loop {
            let Some(content) = wire::next_frame(&mut self.rx_buf)? else {
                break;
            };
            if content.is_empty() {
                continue;
            }
            let recv = self.recv.as_mut().ok_or("接收密钥缺失")?;
            let plaintext = recv.decrypt_with_ad(&[], &content)?;
            messages.push(wire::transport_parse(&plaintext)?);
        }
        Ok(messages)
    }

    // ------------------------------------------------------------------
    // 序列化（插件隔离 kv；含 psk / 临时私钥等凭据材料，绝不下发宿主）
    // ------------------------------------------------------------------

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "role": self.role.as_str(),
            "phase": self.phase.as_str(),
            "h": hex_encode(self.symmetric.hash()),
            "ck": hex_encode(self.symmetric.chaining_key()),
            "cipher": self.symmetric.cipher().map(cipher_to_json),
            "e_priv": hex_encode(&self.local_ephemeral),
            "peer_e": self.peer_ephemeral.as_ref().map(|b| hex_encode(b)),
            "psk": self.psk.as_ref().map(|b| hex_encode(b)),
            "send": self.send.as_ref().map(cipher_to_json),
            "recv": self.recv.as_ref().map(cipher_to_json),
            "rx_buf": hex_encode(&self.rx_buf),
            "peer_name": self.peer_name,
            "peer_mac": self.peer_mac,
        })
    }

    pub(crate) fn from_json(value: &Value) -> Result<Self, String> {
        let text = |key: &str| -> Result<String, String> {
            value
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("握手状态缺少 {key}"))
        };
        let role = Role::parse(&text("role")?)?;
        let phase = Phase::parse(&text("phase")?)?;
        let mut symmetric = crypto::SymmetricState::new();
        symmetric.set_hash(fixed_hex::<{ crypto::HASH_LEN }>(&text("h")?, "h")?);
        symmetric.set_chaining_key(fixed_hex::<{ crypto::HASH_LEN }>(&text("ck")?, "ck")?);
        symmetric.set_cipher(cipher_from_json(value.get("cipher"))?);
        let peer_e = match value.get("peer_e") {
            Some(Value::String(hex)) => Some(fixed_hex::<{ crypto::DH_LEN }>(hex, "peer_e")?),
            Some(Value::Null) | None => None,
            other => return Err(format!("peer_e 非法：{other:?}")),
        };
        let psk = match value.get("psk") {
            Some(Value::String(hex)) => Some(fixed_hex::<{ crypto::KEY_LEN }>(hex, "psk")?),
            Some(Value::Null) | None => None,
            other => return Err(format!("psk 非法：{other:?}")),
        };
        Ok(Self {
            role,
            phase,
            symmetric,
            local_ephemeral: fixed_hex::<{ crypto::DH_LEN }>(&text("e_priv")?, "e_priv")?,
            peer_ephemeral: peer_e,
            psk,
            send: cipher_from_json(value.get("send"))?,
            recv: cipher_from_json(value.get("recv"))?,
            rx_buf: hex_decode(&text("rx_buf")?, wire::MAX_RX_BUF, "rx_buf")?,
            peer_name: value
                .get("peer_name")
                .and_then(Value::as_str)
                .map(str::to_string),
            peer_mac: value
                .get("peer_mac")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }
}

fn cipher_to_json(cipher: &crypto::CipherState) -> Value {
    json!({
        "key": hex_encode(cipher.key()),
        "nonce": cipher.nonce(),
    })
}

fn cipher_from_json(value: Option<&Value>) -> Result<Option<crypto::CipherState>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let key_hex = value
        .get("key")
        .and_then(Value::as_str)
        .ok_or("CipherState 缺少 key")?;
    let nonce = value
        .get("nonce")
        .and_then(Value::as_u64)
        .ok_or("CipherState 缺少 nonce")?;
    let mut cipher =
        crypto::CipherState::new(fixed_hex::<{ crypto::KEY_LEN }>(key_hex, "cipher.key")?);
    cipher.set_nonce(nonce);
    Ok(Some(cipher))
}

fn fixed_hex<const N: usize>(hex: &str, what: &str) -> Result<[u8; N], String> {
    let bytes = hex_decode(hex, N, what)?;
    if bytes.len() != N {
        return Err(format!("{what} 长度非法（{}，应为 {N}）", bytes.len()));
    }
    let mut out = [0u8; N];
    out.copy_from_slice(&bytes);
    Ok(out)
}

/// guest 内自检（不触网、不读写 kv）：双角色握手 + 双向传输 + 错误 psk MAC 失败。
pub(crate) fn self_check() -> Result<(), String> {
    let psk = [0x42u8; crypto::KEY_LEN];
    let mut initiator = Handshake::start(Role::Initiator, psk, [0x11u8; crypto::DH_LEN]);
    let mut responder = Handshake::start(Role::Responder, psk, [0x22u8; crypto::DH_LEN]);
    let hello = initiator.initiator_hello()?;
    if hello.len() != 3 + 3 + 1 + (crypto::DH_LEN + crypto::TAG_LEN) {
        return Err(format!("ClientHello+msg1 长度异常：{}", hello.len()));
    }
    let msg2 = responder.feed(&hello)?;
    if msg2.is_empty() {
        return Err("responder 未产出 msg2".to_string());
    }
    let server_hello = wire::server_hello("esphome-selfcheck", Some("AA:BB:CC:DD:EE:FF"))?;
    let mut inbound = server_hello;
    inbound.extend_from_slice(&msg2);
    initiator.feed(&inbound)?;
    if initiator.phase() != Phase::Ready || responder.phase() != Phase::Ready {
        return Err("握手未进入 Ready".to_string());
    }
    if initiator.peer_name() != Some("esphome-selfcheck") {
        return Err("ServerHello name 未解析".to_string());
    }
    // 双向传输帧：initiator→responder / responder→initiator。
    let request = b"noise-selfcheck-request";
    let sealed = initiator.seal(1, request)?;
    let opened = responder.open(&sealed)?;
    if opened != vec![(1u16, request.to_vec())] {
        return Err(format!("传输帧往返不一致：{opened:?}"));
    }
    let sealed_back = responder.seal(2, b"noise-selfcheck-response")?;
    let opened_back = initiator.open(&sealed_back)?;
    if opened_back != vec![(2u16, b"noise-selfcheck-response".to_vec())] {
        return Err(format!("反向传输帧不一致：{opened_back:?}"));
    }
    // 错误 psk：responder 读同一 msg1 必须 MAC 失败（不静默降级）。
    let mut wrong = Handshake::start(
        Role::Responder,
        [0x43u8; crypto::KEY_LEN],
        [0x33u8; crypto::DH_LEN],
    );
    if wrong.feed(&hello).is_ok() {
        return Err("错误 psk 的 msg1 未被拒绝".to_string());
    }
    Ok(())
}
