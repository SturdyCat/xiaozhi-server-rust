//! Strict controller GATT input. Source: EchoMuse main 858987c,
//! device/internal/bluetooth/bridge.go. session_id is HTTP metadata only.
use base64::Engine as _;
use serde_json::{json, Value};

pub(super) const MAX_WRITE: usize = 512;

pub(crate) fn build_gatt_request(body: &Value) -> Result<Value, String> {
    let object = body.as_object().ok_or("body must be an object")?;
    let t = body.get("t").and_then(Value::as_str).ok_or("missing t")?;
    let keys: &[&str] = match t {
        "connect" => &["t", "addr", "addr_type", "session_id"],
        "disconnect" | "services" => &["t", "addr", "session_id"],
        "read" => &["t", "addr", "handle", "session_id"],
        "write" => &["t", "addr", "handle", "value", "response", "session_id"],
        "slots" => &["t", "session_id"],
        _ => return Err("unsupported t (connect/disconnect/services/read/write/slots)".into()),
    };
    if object.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err("unknown request field".into());
    }
    if let Some(session) = object.get("session_id") {
        session_id(Some(
            session.as_str().ok_or("session_id must be a UUID string")?,
        ))?;
    }
    let mut out = json!({"t":t});
    if t != "slots" {
        let addr = body
            .get("addr")
            .and_then(Value::as_str)
            .ok_or("missing addr")?;
        out["addr"] = normalize_addr(addr)?.into();
    }
    if t == "connect" {
        let at = match body.get("addr_type") {
            None => 0,
            Some(v) => v
                .as_u64()
                .filter(|n| *n <= 1)
                .ok_or("addr_type must be integer 0 or 1")?,
        };
        out["addr_type"] = at.into();
    }
    if matches!(t, "read" | "write") {
        let handle = body
            .get("handle")
            .and_then(Value::as_u64)
            .filter(|h| (1..=65535).contains(h))
            .ok_or("handle must be integer 1..65535")?;
        out["handle"] = handle.into();
    }
    if t == "write" {
        let raw = body
            .get("value")
            .and_then(Value::as_str)
            .ok_or("missing base64 value")?;
        let value = decode_value(raw)?;
        out["value"] = base64::engine::general_purpose::STANDARD
            .encode(value)
            .into();
        out["response"] = match body.get("response") {
            None => true,
            Some(v) => v.as_bool().ok_or("response must be a boolean")?,
        }
        .into();
    }
    Ok(out)
}

pub(super) fn session_id(raw: Option<&str>) -> Result<Option<uuid::Uuid>, String> {
    raw.map(|s| uuid::Uuid::parse_str(s).map_err(|_| "session_id must be a UUID string".into()))
        .transpose()
}

pub(super) fn decode_value(raw: &str) -> Result<Vec<u8>, String> {
    if raw.len() > MAX_WRITE.div_ceil(3) * 4 {
        return Err("value exceeds 512 bytes".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw)
        .map_err(|_| "value must be standard base64")?;
    if bytes.len() > MAX_WRITE {
        return Err("value exceeds 512 bytes".into());
    }
    Ok(bytes)
}

pub(super) fn normalize_addr(addr: &str) -> Result<String, String> {
    let addr = addr.trim();
    if addr.len() != 17
        || addr.split(':').count() != 6
        || addr
            .split(':')
            .any(|p| p.len() != 2 || !p.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("addr must be aa:bb:cc:dd:ee:ff".into());
    }
    Ok(addr.to_ascii_lowercase())
}
