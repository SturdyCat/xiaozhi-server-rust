//! WASM LAN effect consumer for Tuya.
//!
//! The guest can enqueue only typed status/set-DP commands. This module is the
//! native data-plane handoff: device lookup, rustuya session, per-device gate,
//! timeout/error classification, cache/listener updates, and result routing all
//! remain in `TuyaEngine`.

use serde_json::{json, Value};

use crate::plugins::wasm::{LanSocketCommand, WasmHost};

use super::{lan, TuyaEngine};

const MAX_RESULT_BYTES: usize = 16 * 1024;

impl TuyaEngine {
    /// Drain bounded guest commands and route each result back to its plugin KV
    /// namespace. A malformed/native failure is recorded as a typed error; it
    /// never silently falls back to another strategy.
    pub(crate) async fn drain_wasm_commands(&self, host: &WasmHost, max: usize) -> usize {
        let commands = host.drain_lan_commands(max);
        let count = commands.len();
        for command in commands {
            let (plugin_id, request_id, result) = match command {
                LanSocketCommand::Status {
                    plugin_id,
                    request_id,
                    device_handle,
                } => {
                    let result = self.wasm_status(&device_handle).await;
                    (plugin_id, request_id, result)
                }
                LanSocketCommand::SetDp {
                    plugin_id,
                    request_id,
                    device_handle,
                    dp,
                    value_json,
                } => {
                    let result = match serde_json::from_str::<Value>(&value_json) {
                        Ok(value) => self
                            .set_dp(&device_handle, &dp, value)
                            .await
                            .map(|dps| json!({ "ok": true, "dps": dps }))
                            .unwrap_or_else(|failure| failure_json(&failure)),
                        Err(error) => json!({
                            "ok": false,
                            "error": { "kind": "unsupported", "message": error.to_string() }
                        }),
                    };
                    (plugin_id, request_id, result)
                }
            };
            let value = bounded_result(result);
            if let Err(error) =
                host.kv()
                    .put(&plugin_id, &format!("lan-result:{request_id}"), &value)
            {
                tracing::warn!(plugin = %plugin_id, "WASM LAN 结果写回失败：{error}");
            }
        }
        count
    }

    async fn wasm_status(&self, device_id: &str) -> Value {
        let gate = self.inflight_gate(device_id);
        let _guard = gate.lock().await;
        let row = match self.get_device(device_id).await {
            Ok(Some(row)) => row,
            Ok(None) => {
                return json!({
                    "ok": false,
                    "error": { "kind": "offline", "message": "设备不在目录中" }
                })
            }
            Err(error) => {
                return json!({
                    "ok": false,
                    "error": { "kind": "offline", "message": error }
                })
            }
        };
        if !row.online {
            return json!({
                "ok": false,
                "error": { "kind": "offline", "message": "设备离线" }
            });
        }
        let device = match self.lan.get_or_build(&row).await {
            Ok(device) => device,
            Err(failure) => return failure_json(&failure),
        };
        self.lan.ensure_listener(device_id);
        match lan::with_call_timeout(device.status()).await {
            Ok(Some(payload)) => {
                let dps = lan::parse_dps(&payload);
                if !dps.is_empty() {
                    self.dp_cache
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(device_id.to_string(), dps.clone());
                }
                lan::mark_online(&self.store, device_id, true, Some(&row.ip));
                json!({ "ok": true, "dps": dps })
            }
            Ok(None) => json!({ "ok": true, "dps": {} }),
            Err(failure) => failure_json(&failure),
        }
    }
}

fn failure_json(failure: &lan::ControlFailure) -> Value {
    json!({
        "ok": false,
        "error": {
            "kind": failure.kind,
            "message": failure.message,
            "advice": failure.advice,
        }
    })
}

pub(crate) fn bounded_result(result: Value) -> Vec<u8> {
    let bytes = serde_json::to_vec(&result).unwrap_or_else(|_| b"{\"ok\":false}".to_vec());
    if bytes.len() <= MAX_RESULT_BYTES {
        bytes
    } else {
        "{\"ok\":false,\"error\":{\"kind\":\"unsupported\",\"message\":\"结果超出宿主上限\"}}"
            .as_bytes()
            .to_vec()
    }
}
