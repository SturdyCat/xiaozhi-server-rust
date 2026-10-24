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
use crate::config::Config;
use crate::device::echomuse::{self as em, admin, frames, registry, ControlOut, DataOut};
use crate::runtime::Engines;

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
        if !crate::gateway::ws::auth_ok(headers, query, &self.config().server) {
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
    // 产品决策（局域网默认可管理）：不再要求双 token 非空。管理面鉴权由上方
    // `context.gate`（= auth_ok：expected_token 非空才要求 token）承担；
    // 设备侧鉴权看 echomuse.token（空 = 局域网信任，非空 = 必须带 X-Em-Token）。
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

// ==== 服务内组件门面 ========================================================
//
// ESPHome `bluetooth_proxy` 桥（`device/esp_home/bluetooth.rs`）与未来的小米设备类
// 适配器都**只经这里**访问设备 BLE，不各自复制一套 hub/BLE 交互。
//
// 复用 HTTP 路由的同一套语义：`Capture` 负责设备审批、能力声明、管理认证、
// 会话一致性与数据链就绪校验，`execute` 负责请求号关联与超时。区别只有一处：
// 调用方是进程内可信模块，故不经过 HTTP 的设备 token 鉴权。

/// 设备 BLE 快照：连接槽、最近事件（`notify` / `disconnected`）、广播缓存。
#[cfg(feature = "esp_home")]
#[derive(Debug, Clone, Default)]
pub(crate) struct BleSnapshot {
    /// 设备上报的连接槽（`{"free","limit","addrs"}`），未上报为 `None`。
    pub slots: Option<Value>,
    /// 最近事件（最多 50 条，含 `t:"notify"` 与 `t:"disconnected"`）。
    pub events: Vec<Value>,
    /// 广播缓存（按 addr 去重后的最新一份，已按 addr 排序）。
    pub adverts: Vec<Value>,
}

/// 发一条 GATT 命令到设备并等应答。`request` 必须已是设备控制帧格式
/// （`{"t":"connect"|...}`），可用 [`build_gatt_request`] 从 HTTP 形态转换。
#[cfg(feature = "esp_home")]
pub(crate) async fn gatt_call(
    config: &Config,
    id: &str,
    request: Value,
    wait: Duration,
) -> Result<Value, BleError> {
    let capture = Capture::new(registry::global(config), id, None, true)?;
    execute(&capture, request, wait).await
}

/// 读一次设备 BLE 快照（只读，不涉及控制通道命令）。
///
/// ⚠️ 锁顺序必须与 [`snapshot`] 路由一致：先取 BLE 锁读快照并**释放**，再调
/// `current()` 复核会话。若持着 BLE 锁去调 `current()`（内部会再取 BLE 锁）会自锁死。
#[cfg(feature = "esp_home")]
pub(crate) fn read_snapshot(config: &Config, id: &str) -> Result<BleSnapshot, BleError> {
    let capture = Capture::new(registry::global(config), id, None, false)?;
    let (slots, events, adverts) = {
        let link = capture.link.lock().expect("echomuse BLE lock");
        let (slots, events) = link.state_snapshot();
        let (_, adverts) = link.adverts_snapshot();
        (slots, events, adverts)
    };
    capture.current(false)?;
    Ok(BleSnapshot {
        slots,
        events,
        adverts,
    })
}
