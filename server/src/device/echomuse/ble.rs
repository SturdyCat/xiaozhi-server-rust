//! EchoMuse BLE proxy state. Wire authority: main 858987c,
//! docs/device-controller-interface.md and device/internal/bluetooth/bridge.go.
//! No pairing/encryption, protocol codecs outside frames.rs, or arbitrary commands.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::oneshot;

#[path = "ble_http.rs"]
mod http;
#[path = "ble_request.rs"]
mod request;
pub(crate) use request::build_gatt_request;

pub(crate) const GATT_WAIT: Duration = Duration::from_secs(35);
pub(crate) const EVENT_CAP: usize = 100;
pub(crate) const ADVERT_CAP: usize = 256;
pub(crate) const STATE_EVENTS: usize = 50;
pub(super) const MAX_GATT_BYTES: usize = 64 * 1024;
const MAX_ITEM_BYTES: usize = 4096;
pub(super) const MAX_PENDING: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BleError {
    NotFound,
    Unapproved,
    Unsupported,
    MissingToken,
    Unauthorized,
    Offline,
    SessionChanged,
    Disabled,
    DataOffline,
    Busy,
    Timeout,
    OutputLimit,
    Protocol,
    BadRequest(String),
}

type Reply = Result<Value, BleError>;
#[derive(Debug)]
struct Pending {
    owner: uuid::Uuid,
    slots: bool,
    tx: oneshot::Sender<Reply>,
}

/// Connection-scoped, bounded state. A reset drops waiters, but never resets the
/// request counter: late frames must not answer a request from a later data link.
#[derive(Debug, Default)]
pub(crate) struct BleLink {
    /// Logical data-link identity, since reconnect reuses the same mpsc channel.
    pub(crate) data_epoch: u64,
    next_req: u32,
    pending: HashMap<u32, Pending>,
    events: VecDeque<Value>,
    slots: Option<Value>,
    adverts: HashMap<String, Value>,
    adverts_at: u64,
}

impl BleLink {
    pub(crate) fn alloc_req(&mut self) -> u32 {
        loop {
            self.next_req = self.next_req % u32::MAX + 1;
            if !self.pending.contains_key(&self.next_req) {
                return self.next_req;
            }
        }
    }

    pub(super) fn register(
        &mut self,
        req: u32,
        slots: bool,
        tx: oneshot::Sender<Reply>,
    ) -> Result<uuid::Uuid, BleError> {
        if self.pending.len() >= MAX_PENDING || self.pending.contains_key(&req) {
            return Err(BleError::Busy);
        }
        let owner = uuid::Uuid::new_v4();
        self.pending.insert(req, Pending { owner, slots, tx });
        Ok(owner)
    }

    fn forget_owned(&mut self, req: u32, owner: uuid::Uuid) {
        if self.pending.get(&req).is_some_and(|p| p.owner == owner) {
            self.pending.remove(&req);
        }
    }

    pub(crate) fn on_gatt(&mut self, payload: &[u8]) {
        if payload.len() > MAX_GATT_BYTES {
            // Do not deserialize unbounded device JSON. Fail bounded waiters
            // explicitly instead of turning an over-limit result into timeout.
            for (_, pending) in self.pending.drain() {
                let _ = pending.tx.send(Err(BleError::OutputLimit));
            }
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(payload) else {
            return;
        };
        match v.get("t").and_then(Value::as_str) {
            Some("result") => {
                let Some(req) = req_of(&v) else { return };
                if let Some(pending) = self.pending.remove(&req) {
                    let result = if pending.slots || !v.get("ok").is_some_and(Value::is_boolean) {
                        Err(BleError::Protocol)
                    } else {
                        Ok(v)
                    };
                    let _ = pending.tx.send(result);
                }
            }
            Some("slots") => {
                if !valid_slots(&v) {
                    return;
                }
                self.slots = Some(v.clone());
                if let Some(req) = req_of(&v) {
                    if let Some(pending) = self.pending.remove(&req) {
                        let result = if pending.slots {
                            Ok(v)
                        } else {
                            Err(BleError::Protocol)
                        };
                        let _ = pending.tx.send(result);
                    }
                }
            }
            Some("notify") | Some("disconnected") if valid_event(&v) => {
                self.events.push_back(v);
                while self.events.len() > EVENT_CAP {
                    self.events.pop_front();
                }
            }
            _ => {}
        }
    }

    pub(crate) fn merge_adverts(&mut self, batch: &[Value], now: u64) {
        for advert in batch.iter().take(ADVERT_CAP) {
            if !within_size(advert, MAX_ITEM_BYTES) {
                continue;
            }
            let Some(addr) = advert
                .get("addr")
                .and_then(Value::as_str)
                .and_then(|a| request::normalize_addr(a).ok())
            else {
                continue;
            };
            let mut entry = advert.clone();
            entry["addr"] = addr.clone().into();
            entry["seenAt"] = now.into();
            self.adverts.insert(addr, entry);
            while self.adverts.len() > ADVERT_CAP {
                let oldest = self
                    .adverts
                    .iter()
                    .min_by_key(|(_, v)| v.get("seenAt").and_then(Value::as_u64).unwrap_or(0))
                    .map(|(k, _)| k.clone());
                if let Some(key) = oldest {
                    self.adverts.remove(&key);
                } else {
                    break;
                }
            }
        }
        self.adverts_at = now;
    }

    pub(crate) fn adverts_snapshot(&self) -> (u64, Vec<Value>) {
        let mut items: Vec<Value> = self.adverts.values().cloned().collect();
        items.sort_by(|a, b| a["addr"].as_str().cmp(&b["addr"].as_str()));
        (self.adverts_at, items)
    }

    pub(crate) fn state_snapshot(&self) -> (Option<Value>, Vec<Value>) {
        let start = self.events.len().saturating_sub(STATE_EVENTS);
        (
            self.slots.clone(),
            self.events.iter().skip(start).cloned().collect(),
        )
    }

    pub(crate) fn reset(&mut self) {
        self.data_epoch = self.data_epoch.wrapping_add(1);
        self.pending.clear();
        self.events.clear();
        self.slots = None;
        self.adverts.clear();
        self.adverts_at = 0;
    }
}

fn req_of(v: &Value) -> Option<u32> {
    v.get("req")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n != 0)
}

fn valid_slots(v: &Value) -> bool {
    within_size(v, MAX_ITEM_BYTES)
        && v.get("free")
            .and_then(Value::as_u64)
            .is_some_and(|n| n <= 3)
        && v.get("limit")
            .and_then(Value::as_u64)
            .is_some_and(|n| n <= 3)
        && v.get("addrs").and_then(Value::as_array).is_some_and(|a| {
            a.len() <= 3
                && a.iter().all(|v| {
                    v.as_str()
                        .is_some_and(|s| request::normalize_addr(s).is_ok())
                })
        })
}

fn valid_event(v: &Value) -> bool {
    if !within_size(v, MAX_ITEM_BYTES)
        || !v
            .get("addr")
            .and_then(Value::as_str)
            .is_some_and(|s| request::normalize_addr(s).is_ok())
    {
        return false;
    }
    if v["t"] == "notify" {
        v.get("handle")
            .and_then(Value::as_u64)
            .is_some_and(|h| (1..=65535).contains(&h))
            && v.get("value")
                .and_then(Value::as_str)
                .is_some_and(|s| request::decode_value(s).is_ok())
            && v.get("ind").is_some_and(Value::is_boolean)
    } else {
        v.get("reason")
            .and_then(Value::as_u64)
            .is_some_and(|n| n <= 255)
    }
}

/// Count serialized bytes without allocating an attacker-sized temporary String.
fn within_size(v: &Value, cap: usize) -> bool {
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_sub(bytes.len())
                .ok_or_else(|| std::io::Error::other("limit"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Budget(cap), v).is_ok()
}

pub(crate) fn ble_routes() -> axum::Router<Arc<crate::runtime::Engines>> {
    http::router()
}
