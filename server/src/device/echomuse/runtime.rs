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
    listen_end: Option<Value>,
    playback_stats: Option<Value>,
    oww_shadow_cross: Option<Value>,
    wifi_result: Option<Value>,
    wifi_scan: Option<Value>,
    ambient_light: Option<Value>,
    pair_requested_at: Option<u64>,
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
    pub(crate) turn: super::turn::BoundedTurn,
    pub(crate) mic: super::mic::MicControl,
    pub(crate) fx: super::fx::Fx,
    pub(crate) media: Arc<super::media::MediaState>,
    state: Mutex<Runtime>,
}

impl LiveControl {
    pub(crate) fn new(peer: Option<IpAddr>, authenticated: bool) -> Self {
        Self {
            session_id: uuid::Uuid::new_v4(),
            peer,
            authenticated,
            turn: super::turn::BoundedTurn::default(),
            mic: super::mic::MicControl::default(),
            fx: super::fx::Fx::default(),
            media: Arc::new(super::media::MediaState::default()),
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
            // ── P2 上行面（官方文档 §/control Device→Controller；此前全部丢弃）──
            // 设备自己关会话的原因（ack_timeout/max_open/muted/link/stopped）。
            Some("listen_end") => {
                state.listen_end = Some(cloned(value));
            }
            // 一次完成的扬声器流（minDepth/primeWaitMs/underruns/barge…）：
            // 服务端自算"断音"告警之外的设备侧实况。
            Some("playback_stats") => {
                state.playback_stats = Some(cloned(value));
            }
            // 影子评分命中（只报不动作）：调 owwThreshold / 唤醒词模型的关键数据。
            Some("oww_shadow_cross") => {
                state.oww_shadow_cross = Some(cloned(value));
            }
            // 配网闭环的设备侧结果（wifi_result 会重发到 wifi_commit 为止）。
            Some("wifi_result" | "wifi_scan_result") => {
                if value["type"] == "wifi_result" {
                    state.wifi_result = Some(cloned(value));
                } else {
                    state.wifi_scan = Some(cloned(value));
                }
            }
            // 环境光（本机 tsl2540 未绑定则不会出现；字段留作通用）。
            Some("ambient_light") => {
                state.ambient_light = Some(cloned(value));
            }
            // 长按 action 5s 的配对申请：管理台凭 registry 审批，这里只记时刻。
            Some("pair_request") => {
                state.pair_requested_at = Some(super::registry::now_secs());
            }
            _ => return,
        }
        state.updated_at = Some(super::registry::now_secs());
    }

    pub(crate) fn payload(&self, id: &str) -> Value {
        let s = self.state.lock().expect("echomuse runtime lock");
        json!({"device_id": id, "session_id": self.session_id.to_string(), "online": true, "stats": s.stats,
            "listen_state": s.listen_state, "listen_end": s.listen_end,
            "playback_stats": s.playback_stats, "oww_shadow_cross": s.oww_shadow_cross,
            "wifi_result": s.wifi_result, "wifi_scan": s.wifi_scan,
            "ambient_light": s.ambient_light, "pair_requested_at": s.pair_requested_at,
            "volume_state": s.volume_state,
            "mute_state": s.mute_state, "updated_at": s.updated_at})
    }
}

pub(crate) fn offline_payload(id: &str) -> Value {
    json!({"device_id": id, "session_id": null, "online": false, "stats": null, "listen_state": null,
        "listen_end": null, "playback_stats": null, "oww_shadow_cross": null,
        "wifi_result": null, "wifi_scan": null, "ambient_light": null, "pair_requested_at": null,
        "volume_state": null, "mute_state": null, "updated_at": null})
}

/// 上行报告的受限拷贝：深度截断防恶意/异常固件把大对象塞进 runtime
/// （总字节上限已由 MAX_TELEMETRY_BYTES 把关，这里只防单键巨型值）。
fn cloned(value: &Value) -> Value {
    let mut v = value.clone();
    if let Some(obj) = v.as_object_mut() {
        obj.remove("type");
        let long_keys: Vec<String> = obj
            .iter()
            .filter(|(_, val)| val.as_str().is_some_and(|s| s.len() > 256))
            .map(|(k, _)| k.clone())
            .collect();
        for k in long_keys {
            if let Some(s) = obj.get(&k).and_then(Value::as_str) {
                obj.insert(k, json!(s.chars().take(256).collect::<String>()));
            }
        }
    }
    v
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
        self.live.media.close();
        super::hub_remove_session(&self.id, &self.ctl);
    }
}
