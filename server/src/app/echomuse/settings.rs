//! Stored-settings wire compatibility. Incoming HTTP must use the real Go types.
//! Legacy Rust versions persisted owwOnDevice bool and wakeSoundLevel number;
//! Go could not decode those at all, rejecting the WHOLE config message.

use serde_json::Value;

pub(super) fn wire_value(key: &str, value: &Value) -> Option<Value> {
    match key {
        "owwOnDevice" => {
            if let Some(b) = value.as_bool() {
                return Some(Value::from(if b { "on" } else { "off" }));
            }
            value
                .as_str()
                .filter(|s| matches!(*s, "off" | "shadow" | "on"))
                .map(Value::from)
        }
        "wakeSoundLevel" => {
            // Old numeric values were NEVER applied, and have no upstream unit
            // mapping. Preserve upstream's actual existing default, not a guessed
            // percentage-to-loudness conversion.
            if value.is_number() {
                return Some(Value::from("medium"));
            }
            value
                .as_str()
                .filter(|s| matches!(*s, "quiet" | "medium" | "loud"))
                .map(Value::from)
        }
        "wakeMic" => value.as_u64().filter(|v| *v <= 6).map(Value::from),
        "owwThreshold" => value
            .as_f64()
            .filter(|v| *v > 0.0 && *v <= 1.0)
            .map(Value::from),
        _ => Some(value.clone()),
    }
}
