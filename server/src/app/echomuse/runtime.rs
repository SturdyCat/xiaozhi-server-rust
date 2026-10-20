//! Bounded, connection-scoped telemetry and control-session identity.
//! Wire source: EchoMuse main 858987c, device/internal/client/{control,stats}.go.

use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;

use super::ControlOut;

pub(crate) const MAX_TELEMETRY_BYTES: usize = 16 * 1024;

#[derive(Debug, Default)]
struct Runtime {
    stats: Option<Value>,
    listen_state: Option<Value>,
    volume_state: Option<Value>,
    mute_state: Option<Value>,
    updated_at: Option<u64>,
}

/// Peer comes from the accepted TCP connection, never a forwarded/client header.
#[derive(Debug)]
pub(crate) struct LiveControl {
    /// Opaque observation identity, never an authentication credential or persisted.
    pub session_id: uuid::Uuid,
    pub peer: Option<IpAddr>,
    pub authenticated: bool,
    state: Mutex<Runtime>,
}

impl LiveControl {
    pub(crate) fn new(peer: Option<IpAddr>, authenticated: bool) -> Self {
        Self {
            session_id: uuid::Uuid::new_v4(),
            peer,
            authenticated,
            state: Mutex::new(Runtime::default()),
        }
    }

    pub(crate) fn update(&self, value: &Value) {
        if value.to_string().len() > MAX_TELEMETRY_BYTES {
            return;
        }
        let mut state = self.state.lock().expect("echomuse runtime lock");
        match value.get("type").and_then(Value::as_str) {
            Some("stats") => {
                // Scalar allowlist: unknown nested blobs, tokens, credentials,
                // device logs and pairing material never enter runtime.
                const KEYS: &[&str] = &[
                    "cpuPct",
                    "memUsedMb",
                    "memTotalMb",
                    "storageUsedMb",
                    "storageTotalMb",
                    "wifiRssi",
                    "wifiSsid",
                    "linkSpeedMbps",
                    "wifiFreqMhz",
                    "wifiBssid",
                    "tcpUpRetrans",
                    "tcpUpSegs",
                    "txBytes",
                    "rxBytes",
                    "txErrors",
                    "txDropped",
                    "rxCrcErrors",
                    "ambientLux",
                    "cpuTempC",
                    "maxTempC",
                    "coresOnline",
                    "coresTotal",
                    "thermalCoreLimit",
                    "aecRef",
                ];
                let mut stats = serde_json::Map::new();
                for key in KEYS {
                    if let Some(v) = value.get(*key) {
                        if v.is_null()
                            || v.is_number()
                            || v.as_str().is_some_and(|s| s.len() <= 256)
                        {
                            stats.insert((*key).into(), v.clone());
                        }
                    }
                }
                state.stats = Some(Value::Object(stats));
            }
            Some("listen_state") => {
                let Some(mode @ ("local" | "stream" | "degraded")) =
                    value.get("state").and_then(Value::as_str)
                else {
                    return;
                };
                let mut report = json!({"state": mode});
                if let Some(reason) = value
                    .get("reason")
                    .and_then(Value::as_str)
                    .filter(|s| s.len() <= 256)
                {
                    report["reason"] = json!(reason);
                }
                state.listen_state = Some(report);
            }
            Some("volume_state") => {
                let Some(level) = volume_level(value) else {
                    return;
                };
                state.volume_state = Some(json!({"level": level}));
            }
            Some("mute_state") => {
                let Some(muted) = value.get("muted").and_then(Value::as_bool) else {
                    return;
                };
                state.mute_state = Some(json!({"muted": muted}));
            }
            _ => return,
        }
        state.updated_at = Some(super::registry::now_secs());
    }

    pub(crate) fn payload(&self, id: &str) -> Value {
        let s = self.state.lock().expect("echomuse runtime lock");
        json!({"device_id": id, "session_id": self.session_id.to_string(), "online": true, "stats": s.stats,
            "listen_state": s.listen_state, "volume_state": s.volume_state,
            "mute_state": s.mute_state, "updated_at": s.updated_at})
    }
}

pub(crate) fn offline_payload(id: &str) -> Value {
    json!({"device_id": id, "session_id": null, "online": false, "stats": null, "listen_state": null,
        "volume_state": null, "mute_state": null, "updated_at": null})
}

/// Raw tinymix speaker control index, not a percent (upstream volumeMax=127).
pub(crate) fn volume_level(body: &Value) -> Option<u64> {
    body.get("level")
        .and_then(Value::as_u64)
        .filter(|v| *v <= 127)
}

/// Dropping an old/aborted control task must never remove a replacement session.
pub(crate) struct ControlLease {
    pub id: String,
    pub ctl: UnboundedSender<ControlOut>,
    pub live: Arc<LiveControl>,
}

impl Drop for ControlLease {
    fn drop(&mut self) {
        super::hub_remove_session(&self.id, &self.ctl);
    }
}
