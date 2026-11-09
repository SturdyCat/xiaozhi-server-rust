//! S3u：设备会话身份、目录和状态更新校验（无 I/O）。
use crate::api_frame;
use serde_json::{json, Value};
pub(crate) const MAX_ENTITIES: usize = 64;
const MAX_LABEL_LEN: usize = 128;

// ------------------------------------------------------------------
// 入参校验 / 归一化（身份与实体目录由调用方（宿主桥）提供）
// ------------------------------------------------------------------

/// 归一化身份：`name` 必填；其余字段缺省按 native 桥默认值补齐。
pub(crate) fn validate_identity(value: &Value) -> Result<Value, String> {
    if !value.is_object() {
        return Err("identity 必须是对象".to_string());
    }
    let name = required_label(value, "name", "identity.name")?;
    let mut out = value.clone();
    out["name"] = json!(name);
    let defaults = [
        ("mac", ""),
        ("esphome_version", "2026.6.2"),
        ("model", "xiaozhi-server"),
        ("manufacturer", "xiaozhi"),
        ("suggested_area", ""),
    ];
    for (key, default) in defaults {
        let text = value.get(key).and_then(Value::as_str).unwrap_or(default);
        if text.len() > MAX_LABEL_LEN {
            return Err(format!("identity.{key} 超长（> {MAX_LABEL_LEN}）"));
        }
        out[key] = json!(text);
    }
    let friendly = value
        .get("friendly_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    out["friendly_name"] = if friendly.is_empty() {
        json!(name)
    } else {
        json!(required_label(
            value,
            "friendly_name",
            "identity.friendly_name"
        )?)
    };
    out["api_encryption_supported"] = json!(value
        .get("api_encryption_supported")
        .and_then(Value::as_bool)
        .unwrap_or(false));
    for key in [
        "legacy_bluetooth_proxy_version",
        "bluetooth_proxy_feature_flags",
    ] {
        out[key] = json!(value.get(key).and_then(Value::as_u64).unwrap_or(0) as u32);
    }
    Ok(out)
}

/// 归一化实体目录：key 唯一且 < [`MAX_ENTITIES`]，`kind ∈ {binary_sensor, number}`，
/// object_id / name 必填；状态初值按实体种类校验。
pub(crate) fn validate_entities(value: &Value) -> Result<Value, String> {
    let list = value.as_array().ok_or("entities 必须是数组")?;
    if list.len() > MAX_ENTITIES {
        return Err(format!("实体目录超限（{} > {MAX_ENTITIES}）", list.len()));
    }
    let mut seen = Vec::with_capacity(list.len());
    let mut out = Vec::with_capacity(list.len());
    for entity in list {
        if !entity.is_object() {
            return Err("实体条目必须是对象".to_string());
        }
        let key = entity_field_u32(entity, "key")?;
        if usize::try_from(key).map_or(true, |k| k >= MAX_ENTITIES) {
            return Err(format!("实体 key 越界：{key}（须 < {MAX_ENTITIES}）"));
        }
        if seen.contains(&key) {
            return Err(format!("实体 key 重复：{key}"));
        }
        seen.push(key);
        let kind = entity_field_str(entity, "kind")?;
        let mut normalized = entity.clone();
        normalized["object_id"] = json!(required_label(entity, "object_id", "entity.object_id")?);
        normalized["name"] = json!(required_label(entity, "name", "entity.name")?);
        match kind {
            "binary_sensor" => {
                if let Some(class) = entity.get("device_class").and_then(Value::as_str) {
                    if class.len() > MAX_LABEL_LEN {
                        return Err("entity.device_class 超长".to_string());
                    }
                }
                let state = entity
                    .get("state")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                normalized["state"] = json!(state);
            }
            "number" => {
                for field in ["min_value", "max_value", "step"] {
                    let number = entity_number(entity, field)?;
                    if !number.is_finite() {
                        return Err(format!("实体 {key} 的 {field} 必须有限"));
                    }
                    normalized[field] = json!(number);
                }
                normalized["unit_of_measurement"] = json!(entity
                    .get("unit_of_measurement")
                    .and_then(Value::as_str)
                    .unwrap_or_default());
                normalized["mode"] =
                    json!(entity.get("mode").and_then(Value::as_u64).unwrap_or(0) as u32);
                let state = entity
                    .get("state")
                    .and_then(Value::as_f64)
                    .map(|n| n as f32)
                    .unwrap_or(0.0);
                if !state.is_finite() {
                    return Err(format!("实体 {key} 的 state 必须有限"));
                }
                normalized["state"] = json!(state);
            }
            other => return Err(format!("实体 {key} 种类非法：{other}")),
        }
        out.push(normalized);
    }
    Ok(json!(out))
}

fn required_label(value: &Value, key: &str, what: &str) -> Result<String, String> {
    let text = value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if text.is_empty() || text.len() > MAX_LABEL_LEN {
        return Err(format!("{what} 非法（长度须 1..={MAX_LABEL_LEN}）"));
    }
    Ok(text.to_string())
}

pub(crate) fn entities_of(entities: &Value) -> Vec<&Value> {
    entities
        .as_array()
        .map(|list| list.iter().collect())
        .unwrap_or_default()
}

pub(crate) fn entity_field_str<'a>(entity: &'a Value, key: &str) -> Result<&'a str, String> {
    entity
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("实体缺少 {key}"))
}

pub(crate) fn entity_field_u32(entity: &Value, key: &str) -> Result<u32, String> {
    entity
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| format!("实体缺少 {key}（u32）"))
}

pub(crate) fn entity_number(entity: &Value, key: &str) -> Result<f32, String> {
    entity
        .get(key)
        .and_then(Value::as_f64)
        .map(|n| n as f32)
        .ok_or_else(|| format!("实体缺少 {key}（number）"))
}

/// 校验状态更新并归一化为 `{key,state}` 列表；未知 key、重复 key 与类型漂移拒绝。
pub(crate) fn validate_updates(entities: &Value, updates: &Value) -> Result<Vec<Value>, String> {
    let updates = updates.as_array().ok_or("state.updates 必须是数组")?;
    let mut out = Vec::with_capacity(updates.len());
    for update in updates {
        let key = update
            .get("key")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("state update 缺少 u32 key")?;
        if out.iter().any(|item: &Value| item["key"] == key) {
            return Err(format!("state update key 重复：{key}"));
        }
        let entity = entities
            .as_array()
            .and_then(|items| items.iter().find(|item| item["key"] == key))
            .ok_or_else(|| format!("state update 未知实体 key：{key}"))?;
        let state = update.get("state").ok_or("state update 缺少 state")?;
        let kind = entity_field_str(entity, "kind")?;
        let valid = match kind {
            "binary_sensor" => state.is_boolean(),
            "number" => state
                .as_f64()
                .is_some_and(|value| (value as f32).is_finite()),
            _ => false,
        };
        if !valid {
            return Err(format!("state update 类型与实体 {key} 不匹配"));
        }
        out.push(json!({"key": key, "state": state}));
    }
    Ok(out)
}

pub(crate) fn message_name(message_type: u16) -> Option<&'static str> {
    u8::try_from(message_type)
        .ok()
        .and_then(api_frame::message_type_name)
}
