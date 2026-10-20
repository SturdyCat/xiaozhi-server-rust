//! Session-bound HTTP BLE bridge. No hub-by-id lookup may clean up an old op.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};
use tokio::sync::{mpsc::UnboundedSender, oneshot};

use super::{build_gatt_request, request, BleError, BleLink, GATT_WAIT};
use crate::app::echomuse::{self as em, admin, frames, registry, ControlOut, DataOut};
use crate::config::Config;
use crate::engine::Engines;

impl BleError {
    fn response(&self) -> Response {
        let (status, code, error) = match self {
            Self::NotFound => (404, "not_found", "device not found"),
            Self::Unapproved => (403, "unapproved", "device not approved"),
            Self::Unsupported => (
                422,
                "unsupported_capability",
                "device does not declare ble_connect",
            ),
            Self::MissingToken => (
                503,
                "token_required",
                "configure both server.expected_token and echomuse.token",
            ),
            Self::Unauthorized => (401, "unauthorized", "authenticated control required"),
            Self::Offline => (409, "offline", "device control is offline"),
            Self::SessionChanged => (
                409,
                "session_changed",
                "control session changed; refresh devices",
            ),
            Self::Disabled => (
                409,
                "ble_disabled",
                "BLE scanning and connections must both be enabled",
            ),
            Self::DataOffline => (409, "data_offline", "device data link is not ready"),
            Self::Busy => (429, "busy", "too many pending BLE requests"),
            Self::Timeout => (
                504,
                "ble_timeout",
                "BLE deadline exceeded; operation outcome unknown",
            ),
            Self::OutputLimit => (502, "output_limit", "device BLE output exceeded limit"),
            Self::Protocol => (502, "ble_protocol", "invalid device BLE result"),
            Self::BadRequest(message) => (400, "bad_request", message.as_str()),
        };
        admin::json(
            StatusCode::from_u16(status).expect("constant status"),
            json!({"code":code,"error":error}),
        )
    }
}

/// Generic only so route tests use an isolated registry without constructing the
/// speech engines. Production still calls the existing admin gate verbatim.
pub(super) trait Context: Clone + Send + Sync + 'static {
    fn config(&self) -> &Config;
    fn registry(&self) -> &registry::Registry;
    fn gate(&self, headers: &HeaderMap, query: &HashMap<String, String>) -> Option<Response> {
        if !crate::app::ws::auth_ok(headers, query, &self.config().server) {
            return Some(BleError::Unauthorized.response());
        }
        if !self.config().echomuse.enabled {
            return Some(admin::json(
                StatusCode::NOT_FOUND,
                json!({"error":"echomuse disabled"}),
            ));
        }
        None
    }
}

impl Context for Arc<Engines> {
    fn config(&self) -> &Config {
        &self.config
    }
    fn registry(&self) -> &registry::Registry {
        registry::global(&self.config)
    }
    fn gate(&self, headers: &HeaderMap, query: &HashMap<String, String>) -> Option<Response> {
        admin::gate(self, headers, query)
    }
}

pub(super) fn router<S: Context>() -> axum::Router<S> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/api/echomuse/devices/{id}/ble/adverts", get(adverts::<S>))
        .route("/api/echomuse/devices/{id}/ble/state", get(state::<S>))
        .route("/api/echomuse/devices/{id}/ble/gatt", post(gatt::<S>))
        .layer(DefaultBodyLimit::max(4096))
}

fn gate<S: Context>(
    context: &S,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Result<(), Response> {
    if let Some(response) = context.gate(headers, query) {
        return Err(response);
    }
    let config = context.config();
    if !config.echomuse.ble_proxy {
        return Err(admin::json(
            StatusCode::NOT_FOUND,
            json!({"error":"ble proxy disabled"}),
        ));
    }
    if config.server.expected_token.is_empty() || config.echomuse.token.is_empty() {
        return Err(BleError::MissingToken.response());
    }
    Ok(())
}

struct Capture {
    id: String,
    session: uuid::Uuid,
    ctl: UnboundedSender<ControlOut>,
    data: Option<UnboundedSender<DataOut>>,
    data_epoch: u64,
    link: Arc<Mutex<BleLink>>,
}

impl Capture {
    fn new(
        reg: &registry::Registry,
        id: &str,
        expected: Option<uuid::Uuid>,
        gatt: bool,
    ) -> Result<Self, BleError> {
        let record = reg.get(id).ok_or(BleError::NotFound)?;
        if !record.approved {
            return Err(BleError::Unapproved);
        }
        if !record.capabilities.iter().any(|c| c == "ble_connect") {
            return Err(BleError::Unsupported);
        }
        if gatt
            && ["bleProxyEnabled", "bleProxyConnections"]
                .iter()
                .any(|key| {
                    record
                        .settings
                        .get(*key)
                        .is_some_and(|v| v.as_bool() != Some(true))
                })
        {
            return Err(BleError::Disabled);
        }
        let hub = em::hub_iter();
        let entry = hub.get(id).ok_or(BleError::Offline)?;
        let ctl = entry
            .ctl_tx
            .as_ref()
            .filter(|c| !c.is_closed())
            .ok_or(BleError::Offline)?;
        if !entry.management.authenticated {
            return Err(BleError::Unauthorized);
        }
        if expected.is_some_and(|s| s != entry.management.session_id) {
            return Err(BleError::SessionChanged);
        }
        let data = entry.data_tx.as_ref().filter(|d| !d.is_closed()).cloned();
        if gatt && (entry.data_rx.is_some() || data.is_none()) {
            return Err(BleError::DataOffline);
        }
        // All simultaneous locks use hub → BLE, matching data restoration.
        let data_epoch = entry.ble.lock().expect("echomuse BLE lock").data_epoch;
        Ok(Self {
            id: id.into(),
            session: entry.management.session_id,
            ctl: ctl.clone(),
            data,
            data_epoch,
            link: entry.ble.clone(),
        })
    }

    /// Called under a single hub lock before sending and again after a result.
    /// The only nested lock order is hub → BLE; never hold BLE while taking hub.
    fn check(&self, entry: Option<&em::PendingDevice>, need_data: bool) -> Result<(), BleError> {
        let entry = entry.ok_or(BleError::Offline)?;
        if entry.management.session_id != self.session
            || !Arc::ptr_eq(&entry.ble, &self.link)
            || !entry
                .ctl_tx
                .as_ref()
                .is_some_and(|c| c.same_channel(&self.ctl))
        {
            return Err(BleError::SessionChanged);
        }
        if self.ctl.is_closed() {
            return Err(BleError::Offline);
        }
        if need_data
            && (entry.ble.lock().expect("echomuse BLE lock").data_epoch != self.data_epoch
                || entry.data_rx.is_some()
                || !entry.data_tx.as_ref().is_some_and(|d| {
                    !d.is_closed()
                        && self
                            .data
                            .as_ref()
                            .is_some_and(|captured| d.same_channel(captured))
                }))
        {
            return Err(BleError::DataOffline);
        }
        Ok(())
    }

    fn current(&self, need_data: bool) -> Result<(), BleError> {
        self.check(em::hub_iter().get(&self.id), need_data)
    }

    fn envelope(&self, mut value: Value) -> Value {
        value["device_id"] = self.id.clone().into();
        value["session_id"] = json!(self.session.to_string());
        value
    }
}

struct RequestLease {
    link: Arc<Mutex<BleLink>>,
    req: u32,
    owner: uuid::Uuid,
}
impl Drop for RequestLease {
    fn drop(&mut self) {
        self.link
            .lock()
            .expect("echomuse BLE lock")
            .forget_owned(self.req, self.owner);
    }
}

async fn execute(capture: &Capture, mut request: Value, wait: Duration) -> Result<Value, BleError> {
    let mut changes = em::subscribe_devices_version();
    capture.current(true)?;
    let (tx, mut rx) = oneshot::channel();
    let lease = {
        let mut link = capture.link.lock().expect("echomuse BLE lock");
        let req = link.alloc_req();
        let owner = link.register(req, request["t"] == "slots", tx)?;
        RequestLease {
            link: capture.link.clone(),
            req,
            owner,
        }
    };
    request["req"] = lease.req.into();
    {
        let hub = em::hub_iter();
        capture.check(hub.get(&capture.id), true)?;
        // Enqueue under the identity lock; never register on one session and
        // send via a second hub-by-id lookup belonging to its replacement.
        capture
            .data
            .as_ref()
            .ok_or(BleError::DataOffline)?
            .send(frames::frame_ble_gatt(&request.to_string()))
            .map_err(|_| BleError::DataOffline)?;
    }
    let deadline = tokio::time::sleep(wait);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => return Err(BleError::Timeout),
            _ = capture.ctl.closed() => return Err(BleError::Offline),
            changed = changes.changed() => {
                if changed.is_err() { return Err(BleError::Offline); }
                capture.current(true)?;
            }
            reply = &mut rx => {
                capture.current(true)?;
                return reply.map_err(|_| BleError::DataOffline)?;
            }
        }
    }
}

async fn adverts<S: Context>(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(context): State<S>,
) -> Response {
    snapshot(&context, &headers, &query, &id, false)
}
async fn state<S: Context>(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(context): State<S>,
) -> Response {
    snapshot(&context, &headers, &query, &id, true)
}

fn snapshot<S: Context>(
    context: &S,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
    id: &str,
    state: bool,
) -> Response {
    if let Err(response) = gate(context, headers, query) {
        return response;
    }
    let expected = match request::session_id(query.get("session_id").map(String::as_str)) {
        Ok(session) => session,
        Err(message) => return BleError::BadRequest(message).response(),
    };
    let capture = match Capture::new(context.registry(), id, expected, false) {
        Ok(capture) => capture,
        Err(error) => return error.response(),
    };
    let value = {
        let link = capture.link.lock().expect("echomuse BLE lock");
        if state {
            let (slots, events) = link.state_snapshot();
            json!({"slots":slots,"events":events})
        } else {
            let (at, adverts) = link.adverts_snapshot();
            json!({"updatedAt":if at == 0 { Value::Null } else { json!(at) },"count":adverts.len(),"adverts":adverts})
        }
    };
    match capture.current(false) {
        Ok(()) => admin::json(StatusCode::OK, capture.envelope(value)),
        Err(error) => error.response(),
    }
}

async fn gatt<S: Context>(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(context): State<S>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if let Err(response) = gate(&context, &headers, &query) {
        return response;
    }
    let request = match build_gatt_request(&body) {
        Ok(request) => request,
        Err(message) => return BleError::BadRequest(message).response(),
    };
    let expected = match request::session_id(body.get("session_id").and_then(Value::as_str)) {
        Ok(session) => session,
        Err(message) => return BleError::BadRequest(message).response(),
    };
    let capture = match Capture::new(context.registry(), &id, expected, true) {
        Ok(capture) => capture,
        Err(error) => return error.response(),
    };
    match execute(&capture, request, GATT_WAIT).await {
        // Device ok:false is a real reply, not success of the requested action.
        Ok(value) => admin::json(StatusCode::OK, capture.envelope(value)),
        Err(error) => error.response(),
    }
}

#[cfg(test)]
#[path = "ble_http_tests.rs"]
mod tests;
