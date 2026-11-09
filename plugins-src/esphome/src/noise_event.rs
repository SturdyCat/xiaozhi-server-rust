//! S3t：`connector.esp-home.noise` 事件面——Noise NNpsk0 握手 / 传输帧的宿主步进接口。
//!
//! 事件模型与 tuya LAN 编排同纪律：**事件间无内存态**，握手状态（含 psk / 临时私钥 /
//! 传输密钥 / 流式接收缓冲）全部 JSON 序列化进插件隔离 kv（`esphome-noise-state:<session>`），
//! 每个 op 读-步进-写回；结果写 `esphome-noise:<req_id>` 通道。
//!
//! op 一览（宿主/编排层按连接生命周期调用，I/O 与 TCP 由宿主 `net` 服务承载）：
//! - `start`：`{role, psk_hex, server_name?, server_mac?}` → initiator 返回
//!   `out_hex = ClientHello + msg1 帧`；responder 返回 `out_hex = ServerHello 帧`
//!   （随后 `step` 消费对端字节并产出 msg2）。临时密钥经宿主 `host-core.random`。
//! - `step`：`{data_hex}`（对端原始字节，可半包/粘包）→ `out_hex` + `phase`
//!   （`await-server-hello|await-msg2|await-msg1|ready`）+ `peer_name/peer_mac`。
//! - `seal`：`{message, ...}` 或 `{type, payload_hex}` → `frame_hex`（AEAD 传输帧）。
//! - `open`：`{data_hex}` → `messages:[{type, message, payload_hex, ...解出的字段}]`。
//! - `close`：删除会话状态（连接关闭/握手失败时调用，失败也好回收）。
//!
//! 边界：psk 由编排层随事件载荷传入（与 tuya `lan-connect` 的 `local_key` 同口径）；
//! 生产接管时由插件配置命名空间（`config-secret`）提供，见 `plugins-src/esphome/README.md`。

use serde_json::{json, Value};
use xiaozhi_plugin_sdk::xiaozhi::plugin::{host_core as host, types};

use crate::hex::{hex_decode, hex_encode};
use crate::noise::{self, Handshake, Role};
use crate::noise_wire as wire;
use crate::{api_frame, attach_decoded, checked_req_id, message_payload};

/// 结果通道前缀（与 `esphome-result` / `esphome-frame` 并列）。
const RESULT_PREFIX: &str = "esphome-noise";
/// 会话状态 kv 前缀（值含握手凭据材料，只留插件隔离 kv）。
const STATE_PREFIX: &str = "esphome-noise-state";
/// 并发会话上限（每个连接一条；防御性上限，正常远小于此）。
const MAX_SESSIONS: usize = 64;
/// 会话 id 长度上限 / 输入字节上限。
const MAX_SESSION_LEN: usize = 64;
const MAX_INPUT_LEN: usize = wire::MAX_RX_BUF;
/// 传输帧 protobuf 载荷上限。
const MAX_PAYLOAD_LEN: usize = 64 * 1024;

fn result(value: Value, req_id: &str) -> types::Effect {
    types::Effect::KvPut(types::EffectKvPut {
        key: format!("{RESULT_PREFIX}:{req_id}"),
        value: value.to_string().into_bytes(),
    })
}

/// 事件入口：解析 req_id + op，任何业务性错误都写回 `ok:false`（不静默、不降级）。
pub(crate) fn handle(payload: &str) -> Result<Vec<types::Effect>, String> {
    let req_id = checked_req_id(payload)?;
    let value: Value = serde_json::from_str(payload).map_err(|e| format!("noise 载荷非法：{e}"))?;
    let outcome = match dispatch(&value) {
        Ok(out) => out,
        Err(error) => json!({"ok": false, "error": error}),
    };
    Ok(vec![result(outcome, &req_id)])
}

fn dispatch(value: &Value) -> Result<Value, String> {
    let op = value
        .get("op")
        .and_then(Value::as_str)
        .ok_or("载荷缺少 op（start|step|seal|open|close）")?;
    match op {
        "start" => start(value),
        "step" => step(value),
        "seal" => seal(value),
        "open" => open(value),
        "close" => close(value),
        other => Err(format!(
            "未知 noise op：{other}（start|step|seal|open|close）"
        )),
    }
}

// ------------------------------------------------------------------
// 会话状态 kv
// ------------------------------------------------------------------

fn session_of(value: &Value) -> Result<String, String> {
    let session = value
        .get("session")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if session.is_empty() || session.len() > MAX_SESSION_LEN {
        return Err(format!(
            "session 非法（长度须 1..={MAX_SESSION_LEN}，当前 {}）",
            session.len()
        ));
    }
    if !session
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(format!(
            "session 非法（只允许 ASCII 字母数字与 -_.）：{session}"
        ));
    }
    Ok(session.to_string())
}

fn state_key(session: &str) -> String {
    format!("{STATE_PREFIX}:{session}")
}

fn load(session: &str) -> Result<Handshake, String> {
    let bytes = host::kv_get(&state_key(session))
        .ok_or_else(|| format!("noise 会话不存在（start 未调用或已 close）：{session}"))?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("noise 会话状态 kv 非法：{e}"))?;
    Handshake::from_json(&value)
}

fn save(session: &str, handshake: &Handshake) -> Result<(), String> {
    let encoded = handshake.to_json().to_string();
    host::kv_put(&state_key(session), encoded.as_bytes())
        .map_err(|e| format!("noise 会话状态 kv 写入失败：{e}"))
}

// ------------------------------------------------------------------
// op：start / step / seal / open / close
// ------------------------------------------------------------------

fn start(value: &Value) -> Result<Value, String> {
    let session = session_of(value)?;
    let role = Role::parse(
        value
            .get("role")
            .and_then(Value::as_str)
            .ok_or("载荷缺少 role（initiator|responder）")?,
    )?;
    if host::kv_get(&state_key(&session)).is_some() {
        return Err(format!("noise 会话已存在（先 close 再重建）：{session}"));
    }
    if host::kv_keys(&format!("{STATE_PREFIX}:")).len() >= MAX_SESSIONS {
        return Err(format!("noise 会话数超限（> {MAX_SESSIONS}）"));
    }
    let psk = fixed_hex(value, "psk_hex", "psk")?;
    let ephemeral = host::random(32).map_err(|e| format!("宿主随机源不可用（{e}）"))?;
    if ephemeral.len() != 32 {
        return Err(format!("宿主随机源返回长度非法（{}）", ephemeral.len()));
    }
    let mut handshake = Handshake::start(role, psk, fixed_array(&ephemeral, "ephemeral")?);
    let out = match role {
        Role::Initiator => handshake.initiator_hello()?,
        Role::Responder => {
            let name = value
                .get("server_name")
                .and_then(Value::as_str)
                .ok_or("responder 的 start 缺少 server_name（ServerHello 身份）")?;
            let mac = value.get("server_mac").and_then(Value::as_str);
            wire::server_hello(name, mac)?
        }
    };
    save(&session, &handshake)?;
    host::log(
        "debug",
        &format!(
            "noise start：session={session} role={} phase={} out={}B",
            role.as_str(),
            handshake.phase().as_str(),
            out.len()
        ),
    );
    Ok(json!({
        "ok": true,
        "op": "start",
        "session": session,
        "role": role.as_str(),
        "phase": handshake.phase().as_str(),
        "out_hex": hex_encode(&out),
    }))
}

fn step(value: &Value) -> Result<Value, String> {
    let session = session_of(value)?;
    let mut handshake = load(&session)?;
    let data = hex_decode(required_hex(value, "data_hex")?, MAX_INPUT_LEN, "data")?;
    let outgoing = handshake.feed(&data)?;
    save(&session, &handshake)?;
    let mut out = json!({
        "ok": true,
        "op": "step",
        "session": session,
        "phase": handshake.phase().as_str(),
        "out_hex": hex_encode(&outgoing),
    });
    if let Some(name) = handshake.peer_name() {
        out["peer_name"] = json!(name);
    }
    if let Some(mac) = handshake.peer_mac() {
        out["peer_mac"] = json!(mac);
    }
    Ok(out)
}

fn seal(value: &Value) -> Result<Value, String> {
    let session = session_of(value)?;
    let mut handshake = load(&session)?;
    let (message_type, payload) = match value.get("message").and_then(Value::as_str) {
        Some(message) => (
            u16::from(api_frame::message_type_id(message)?),
            message_payload(message, value)?,
        ),
        None => {
            let message_type = value
                .get("type")
                .and_then(Value::as_u64)
                .ok_or("seal 需要 message 或 type")?;
            let payload = hex_decode(
                required_hex(value, "payload_hex")?,
                MAX_PAYLOAD_LEN,
                "payload",
            )?;
            (parse_message_type(message_type)?, payload)
        }
    };
    let frame = handshake.seal(message_type, &payload)?;
    save(&session, &handshake)?;
    Ok(json!({
        "ok": true,
        "op": "seal",
        "session": session,
        "type": message_type,
        "message": message_name(message_type),
        "frame_hex": hex_encode(&frame),
    }))
}

fn open(value: &Value) -> Result<Value, String> {
    let session = session_of(value)?;
    let mut handshake = load(&session)?;
    let data = hex_decode(required_hex(value, "data_hex")?, MAX_INPUT_LEN, "data")?;
    let messages = handshake.open(&data)?;
    save(&session, &handshake)?;
    let mut decoded = Vec::with_capacity(messages.len());
    for (message_type, payload) in messages {
        let mut item = json!({
            "type": message_type,
            "message": message_name(message_type),
            "payload_hex": hex_encode(&payload),
        });
        if u8::try_from(message_type).is_ok() {
            attach_decoded(message_type as u8, &payload, &mut item)?;
        }
        decoded.push(item);
    }
    Ok(json!({
        "ok": true,
        "op": "open",
        "session": session,
        "messages": decoded,
    }))
}

fn close(value: &Value) -> Result<Value, String> {
    let session = session_of(value)?;
    let removed = host::kv_delete(&state_key(&session));
    Ok(json!({
        "ok": true,
        "op": "close",
        "session": session,
        "removed": removed,
    }))
}

// ------------------------------------------------------------------
// 小工具
// ------------------------------------------------------------------

fn required_hex<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("载荷缺少 {key}"))
}

fn fixed_hex(value: &Value, key: &str, what: &str) -> Result<[u8; noise::PSK_LEN], String> {
    let bytes = hex_decode(required_hex(value, key)?, noise::PSK_LEN, what)?;
    if bytes.len() != noise::PSK_LEN {
        return Err(format!(
            "{what} 长度非法（{}，应为 {}）",
            bytes.len(),
            noise::PSK_LEN
        ));
    }
    fixed_array(&bytes, what)
}

fn fixed_array<const N: usize>(bytes: &[u8], what: &str) -> Result<[u8; N], String> {
    if bytes.len() != N {
        return Err(format!("{what} 长度非法（{}，应为 {N}）", bytes.len()));
    }
    let mut out = [0u8; N];
    out.copy_from_slice(bytes);
    Ok(out)
}

fn parse_message_type(value: u64) -> Result<u16, String> {
    u16::try_from(value).map_err(|_| format!("type 越界（{value} > 65535）"))
}

fn message_name(message_type: u16) -> Option<&'static str> {
    u8::try_from(message_type)
        .ok()
        .and_then(api_frame::message_type_name)
}
