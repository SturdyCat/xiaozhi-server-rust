//! S3u：`connector.esp-home.session` 事件面——设备侧会话（responder）的宿主步进接口。
//!
//! 事件模型与 tuya LAN 编排同纪律：**事件间无内存态**，会话状态（身份 / 实体目录 /
//! 订阅态 / 加密握手状态 / 流式缓冲）JSON 序列化进插件隔离 kv
//! （`esphome-session-state:<session>`），每个 op 读-步进-写回；结果写
//! `esphome-session:<req_id>` 通道。
//!
//! op 一览（I/O 与 TCP 由宿主 `net` 服务承载，插件只步进协议语义）：
//! - `open`：`{session, psk_hex?, identity, entities}` → 明文（无 psk）或
//!   Noise NNpsk0 加密（有 psk，临时密钥经宿主 `host-core.random`）会话；
//!   加密模式返回 `out_hex = ServerHello 帧`；
//! - `feed`：`{session, data_hex}`（对端原始字节，可半包/粘包）→ `out_hex` +
//!   `phase` + `messages`（解码后的对端消息）+ `subscribed` + `closed`；
//! - `state`：`{session, updates:[{key, state}]}` → 订阅态下输出状态响应帧 `out_hex`；
//! - `close`：删除会话状态（连接关闭/失败时调用，失败也好回收）。
//!
//! 边界：身份与实体目录随 `open` 载荷传入（数据面真相源仍 native：
//! `[esp_home]` 配置 + 实体状态观测，由宿主桥在后续切片接线）；psk 与
//! `connector.esp-home.noise` 同口径随载荷传入，生产接管时改由插件配置命名空间提供。

use serde_json::{json, Value};
use xiaozhi_plugin_sdk::xiaozhi::plugin::{host_core as host, types};

use crate::checked_req_id;
use crate::hex::{hex_decode, hex_encode};
use crate::noise::PSK_LEN;
use crate::noise_crypto::DH_LEN;
use crate::noise_wire as wire;
use crate::session::Session;
use crate::session_catalog;

/// 结果通道前缀（与 `esphome-result` / `esphome-frame` / `esphome-noise` 并列）。
const RESULT_PREFIX: &str = "esphome-session";
/// 会话状态 kv 前缀（值含握手凭据材料，只留插件隔离 kv）。
const STATE_PREFIX: &str = "esphome-session-state";
/// 并发会话上限（每个连接一条；防御性上限，正常远小于此）。
const MAX_SESSIONS: usize = 64;
/// 会话 id 长度上限 / 输入字节上限。
const MAX_SESSION_LEN: usize = 64;
const MAX_INPUT_LEN: usize = 64 * 1024;

fn result(value: Value, req_id: &str) -> types::Effect {
    types::Effect::KvPut(types::EffectKvPut {
        key: format!("{RESULT_PREFIX}:{req_id}"),
        value: value.to_string().into_bytes(),
    })
}

/// 事件入口：解析 req_id + op，任何业务性错误都写回 `ok:false`（不静默、不降级）。
pub(crate) fn handle(payload: &str) -> Result<Vec<types::Effect>, String> {
    let req_id = checked_req_id(payload)?;
    let value: Value =
        serde_json::from_str(payload).map_err(|e| format!("session 载荷非法：{e}"))?;
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
        .ok_or("载荷缺少 op（open|feed|state|close）")?;
    match op {
        "open" => open(value),
        "feed" => feed(value),
        "state" => state(value),
        "close" => close(value),
        other => Err(format!("未知 session op：{other}（open|feed|state|close）")),
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

fn load(session: &str) -> Result<Session, String> {
    let bytes = host::kv_get(&state_key(session))
        .ok_or_else(|| format!("会话不存在（open 未调用或已 close）：{session}"))?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("会话状态 kv 非法：{e}"))?;
    Session::from_json(&value)
}

fn save(session: &str, value: &Session) -> Result<(), String> {
    let encoded = value.to_json().to_string();
    host::kv_put(&state_key(session), encoded.as_bytes())
        .map_err(|e| format!("会话状态 kv 写入失败：{e}"))
}

// ------------------------------------------------------------------
// op：open / feed / state / close
// ------------------------------------------------------------------

fn open(value: &Value) -> Result<Value, String> {
    let session = session_of(value)?;
    if host::kv_get(&state_key(&session)).is_some() {
        return Err(format!("会话已存在（先 close 再重建）：{session}"));
    }
    if host::kv_keys(&format!("{STATE_PREFIX}:")).len() >= MAX_SESSIONS {
        return Err(format!("会话数超限（> {MAX_SESSIONS}）"));
    }
    let mut identity =
        session_catalog::validate_identity(value.get("identity").ok_or("载荷缺少 identity")?)?;
    let entities =
        session_catalog::validate_entities(value.get("entities").ok_or("载荷缺少 entities")?)?;
    let psk = match value.get("psk_hex") {
        None | Some(Value::Null) => None,
        Some(_) => Some(fixed_hex(value, "psk_hex")?),
    };
    identity["api_encryption_supported"] = json!(psk.is_some());
    let (state, out, mode) = match psk {
        None => (
            Session::new_plaintext(identity, entities),
            Vec::new(),
            "plaintext",
        ),
        Some(psk) => {
            let name = identity
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let mac = identity
                .get("mac")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let ephemeral =
                host::random(DH_LEN as u32).map_err(|e| format!("宿主随机源不可用（{e}）"))?;
            let ephemeral_len = ephemeral.len();
            let ephemeral: [u8; DH_LEN] = ephemeral
                .as_slice()
                .try_into()
                .map_err(|_| format!("宿主随机源返回长度非法（{ephemeral_len}）"))?;
            let handshake = Session::new_encrypted(identity, entities, psk, ephemeral);
            let out = wire::server_hello(&name, if mac.is_empty() { None } else { Some(&mac) })?;
            (handshake, out, "encrypted")
        }
    };
    save(&session, &state)?;
    host::log(
        "debug",
        &format!(
            "esphome session open：session={session} mode={mode} phase={} out={}B",
            state.phase(),
            out.len()
        ),
    );
    Ok(json!({
        "ok": true,
        "op": "open",
        "session": session,
        "mode": mode,
        "phase": state.phase(),
        "out_hex": hex_encode(&out),
    }))
}

fn feed(value: &Value) -> Result<Value, String> {
    let session = session_of(value)?;
    let mut state = load(&session)?;
    let data = hex_decode(required_hex(value, "data_hex")?, MAX_INPUT_LEN, "data")?;
    let outcome = match state.feed(&data) {
        Ok(outcome) => outcome,
        Err(error) => {
            // 一旦协议/认证失败即销毁密钥，不能重试旧状态或重用 nonce。
            host::kv_delete(&state_key(&session));
            return Err(error);
        }
    };
    save(&session, &state)?;
    Ok(json!({
        "ok": true,
        "op": "feed",
        "session": session,
        "phase": outcome.phase,
        "subscribed": outcome.subscribed,
        "closed": outcome.closed,
        "out_hex": hex_encode(&outcome.out),
        "messages": outcome.messages,
    }))
}

fn state(value: &Value) -> Result<Value, String> {
    let session = session_of(value)?;
    let mut state = load(&session)?;
    let updates = value.get("updates").ok_or("载荷缺少 updates 数组")?;
    if updates.as_array().is_none() {
        return Err("updates 必须是数组".to_string());
    }
    let out = state.apply_states(updates)?;
    save(&session, &state)?;
    Ok(json!({
        "ok": true,
        "op": "state",
        "session": session,
        "subscribed": state.subscribed(),
        "out_hex": hex_encode(&out),
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

fn fixed_hex(value: &Value, key: &str) -> Result<[u8; PSK_LEN], String> {
    let bytes = hex_decode(required_hex(value, key)?, PSK_LEN, "psk")?;
    if bytes.len() != PSK_LEN {
        return Err(format!("psk 长度非法（{}，应为 {PSK_LEN}）", bytes.len()));
    }
    let mut out = [0u8; PSK_LEN];
    out.copy_from_slice(&bytes);
    Ok(out)
}
