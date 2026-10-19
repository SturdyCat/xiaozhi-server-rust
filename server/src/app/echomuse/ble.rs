//! EchoMuse 蓝牙代理（`ble_connect` + `ble_adverts_data`）：把设备的 BLE 扫描器与
//! GATT 连接桥接到服务端——采集广播、直驱外围设备读写。
//!
//! 协议要点（权威来源：EchoMuse 仓库 `docs/device-controller-interface.md`，命名以
//! 代码为准）：
//!
//! - 采集：设备被动扫描的广播批次经 `0x06` 帧上报（旧固件回退 control 面
//!   `ble_adverts` JSON），条目形如 `{"addr","addrType","rssi","data"(base64)}`；
//!   仅当服务器 ack 声明 `ble_adverts_data` 后设备才发送。
//! - GATT：双方都在 ack 声明 `ble_connect` 且设备配置 `bleProxyEnabled` +
//!   `bleProxyConnections` 打开时，服务器经 `0x08` 帧下发 `{"t","req",..}` 请求，
//!   设备以 `{"t":"result","req",ok,..}` 应答同一 `req`；`slots` 查询的应答是携带
//!   `req` 的 `slots` 事件本身；`notify`/`disconnected` 是设备主动事件。
//! - 无配对：对端 Security Request 按协议答 Pairing Not Supported，需要加密的
//!   属性以 `att:5` 失败——服务端只透传，不引入密钥体系。
//!
//! 状态载体 [`BleLink`] 挂在 [`super::PendingDevice`] 上（会话生命周期 = 设备在线
//! 期，与协议"control 连接断开设备即丢弃全部 BLE 链路"一致）；本文件同时承载
//! 管理面端点 `/api/echomuse/devices/{id}/ble/*`（经 admin 的 gate 鉴权）。
//!
//! [`super::PendingDevice`]: super::PendingDevice

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use base64::Engine as _;

use crate::engine::Engines;

use super::admin::{gate, json};
use super::{frames, hub_ble, hub_data, registry};

/// GATT 结果等待上限：设备自守时 connect 20s、ATT 30s（超时自己回错误 result），
/// 35s 让设备先答，这里只兜"链路静默死亡"。
pub(crate) const GATT_WAIT: Duration = Duration::from_secs(35);
/// 事件环容量（notify/disconnected；超出丢最旧）。
pub(crate) const EVENT_CAP: usize = 100;
/// 广播合并表容量上限（活跃空口一批可上百条，超出按 seenAt 淘汰最旧）。
pub(crate) const ADVERT_CAP: usize = 256;
/// `state` 端点返回的最近事件条数上限。
pub(crate) const STATE_EVENTS: usize = 50;

/// 一台在线设备的 BLE 链路状态：请求关联 + 采集数据（会话结束随 hub 条目消失）。
#[derive(Debug, Default)]
pub(crate) struct BleLink {
    next_req: u32,
    /// req → 等待结果的 oneshot 发送端。
    pending: HashMap<u32, tokio::sync::oneshot::Sender<serde_json::Value>>,
    /// notify / disconnected 事件环（slots 单独存，不进环）。
    events: VecDeque<serde_json::Value>,
    /// 最近一次 slots 事件（连接槽位：free/limit/addrs）。
    slots: Option<serde_json::Value>,
    /// 广播合并表：addr → 条目（附加 seenAt unix 秒）。
    adverts: HashMap<String, serde_json::Value>,
    /// 最近一次收到广播批次的 unix 秒（0 = 尚未收到）。
    adverts_at: u64,
}

impl BleLink {
    /// 分配请求号（协议 `req`：结果原样回显；从 1 起、u32 环绕，与 EchoMuse
    /// 控制器同口径）。
    pub(crate) fn alloc_req(&mut self) -> u32 {
        self.next_req = self.next_req % u32::MAX + 1;
        self.next_req
    }

    /// 登记待应答请求（结果到达时由 [`Self::on_gatt`] 唤醒）。
    pub(crate) fn register(
        &mut self,
        req: u32,
        tx: tokio::sync::oneshot::Sender<serde_json::Value>,
    ) {
        self.pending.insert(req, tx);
    }

    /// 超时/发送失败时回收登记，防 pending 残留。
    pub(crate) fn forget(&mut self, req: u32) {
        self.pending.remove(&req);
    }

    /// 一条上行 `0x08` JSON：`result` 按关联唤醒等待方；`slots` 更新槽位快照
    /// （携带匹配 `req` 时兼作查询应答）；`notify`/`disconnected` 入事件环；
    /// 未知类型按协议忽略。
    pub(crate) fn on_gatt(&mut self, payload: &[u8]) {
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(payload) else {
            return;
        };
        if !v.is_object() {
            return;
        }
        match v.get("t").and_then(|t| t.as_str()) {
            Some("result") => {
                let Some(req) = req_of(&v) else { return };
                if let Some(tx) = self.pending.remove(&req) {
                    let _ = tx.send(v);
                }
            }
            Some("slots") => {
                self.slots = Some(v.clone());
                if let Some(req) = req_of(&v) {
                    if let Some(tx) = self.pending.remove(&req) {
                        let _ = tx.send(v);
                    }
                }
            }
            Some("notify") | Some("disconnected") => {
                self.events.push_back(v);
                while self.events.len() > EVENT_CAP {
                    self.events.pop_front();
                }
            }
            _ => {}
        }
    }

    /// 合并一批广播（`0x06` 或旧 control 面 `ble_adverts`）：缺 addr 的条目忽略，
    /// 同 addr 新批次覆盖旧条目，容量超限按 seenAt 淘汰最旧。
    pub(crate) fn merge_adverts(&mut self, batch: &[serde_json::Value], now: u64) {
        for advert in batch {
            let Some(addr) = advert.get("addr").and_then(|v| v.as_str()) else {
                continue;
            };
            let mut entry = advert.clone();
            entry["seenAt"] = serde_json::Value::from(now);
            self.adverts.insert(addr.to_string(), entry);
            while self.adverts.len() > ADVERT_CAP {
                let oldest = self
                    .adverts
                    .iter()
                    .min_by_key(|(_, v)| v.get("seenAt").and_then(|s| s.as_u64()).unwrap_or(0))
                    .map(|(k, _)| k.clone());
                match oldest {
                    Some(k) => {
                        self.adverts.remove(&k);
                    }
                    None => break,
                }
            }
        }
        self.adverts_at = now;
    }

    /// 广播快照：(最近批次时刻, 按 addr 排序的条目列表)。
    pub(crate) fn adverts_snapshot(&self) -> (u64, Vec<serde_json::Value>) {
        let mut items: Vec<serde_json::Value> = self.adverts.values().cloned().collect();
        items.sort_by(|a, b| {
            let ka = a.get("addr").and_then(|v| v.as_str()).unwrap_or("");
            let kb = b.get("addr").and_then(|v| v.as_str()).unwrap_or("");
            ka.cmp(kb)
        });
        (self.adverts_at, items)
    }

    /// 状态快照：(slots, 最近事件，最旧在前)。
    pub(crate) fn state_snapshot(&self) -> (Option<serde_json::Value>, Vec<serde_json::Value>) {
        let start = self.events.len().saturating_sub(STATE_EVENTS);
        (
            self.slots.clone(),
            self.events.iter().skip(start).cloned().collect(),
        )
    }

    /// 链路收尾（data 任务退出）：丢弃全部待应答请求（等待方立即得到"离线"）
    /// 并清空采集数据。
    pub(crate) fn reset(&mut self) {
        self.pending.clear();
        self.events.clear();
        self.slots = None;
        self.adverts.clear();
        self.adverts_at = 0;
    }
}

fn req_of(v: &serde_json::Value) -> Option<u32> {
    v.get("req").and_then(|r| r.as_u64()).map(|r| r as u32)
}

// ---- GATT 请求构造（纯函数，测试直驱） ----

/// 校验并构造一条 GATT 请求体（不含 `req`；调用方分配后插入）。字段与必填按协议：
/// `connect(addr[, addr_type])`、`disconnect/services(addr)`、`read(addr, handle)`、
/// `write(addr, handle, value(base64)[, response])`、`slots()`。
pub(crate) fn build_gatt_request(body: &serde_json::Value) -> Result<serde_json::Value, String> {
    let t = body.get("t").and_then(|t| t.as_str()).ok_or("缺少 t")?;
    let mut out = serde_json::Map::new();
    out.insert("t".into(), serde_json::Value::from(t));
    match t {
        "connect" => {
            insert_addr(&mut out, body)?;
            let at = body.get("addr_type").and_then(|v| v.as_u64()).unwrap_or(0);
            if at > 1 {
                return Err("addr_type 只支持 0（public）/ 1（random）".into());
            }
            out.insert("addr_type".into(), serde_json::Value::from(at));
        }
        "disconnect" | "services" => insert_addr(&mut out, body)?,
        "read" => {
            insert_addr(&mut out, body)?;
            insert_handle(&mut out, body)?;
        }
        "write" => {
            insert_addr(&mut out, body)?;
            insert_handle(&mut out, body)?;
            let raw = body
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or("write 需要 value（base64）")?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(raw.trim())
                .map_err(|_| "value 不是合法 base64")?;
            // 重编码为规范形（设备侧按标准 base64 解码）。
            out.insert(
                "value".into(),
                serde_json::Value::from(base64::engine::general_purpose::STANDARD.encode(bytes)),
            );
            let response = body
                .get("response")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            out.insert("response".into(), serde_json::Value::from(response));
        }
        "slots" => {}
        other => {
            return Err(format!(
                "不支持的 t: {other}（支持 connect/disconnect/services/read/write/slots）"
            ))
        }
    }
    Ok(serde_json::Value::Object(out))
}

fn insert_addr(
    out: &mut serde_json::Map<String, serde_json::Value>,
    body: &serde_json::Value,
) -> Result<(), String> {
    let addr = body
        .get("addr")
        .and_then(|v| v.as_str())
        .ok_or("请求需要 addr（aa:bb:cc:dd:ee:ff）")?;
    out.insert(
        "addr".into(),
        serde_json::Value::from(normalize_addr(addr)?),
    );
    Ok(())
}

fn insert_handle(
    out: &mut serde_json::Map<String, serde_json::Value>,
    body: &serde_json::Value,
) -> Result<(), String> {
    let h = body
        .get("handle")
        .and_then(|v| v.as_u64())
        .ok_or("请求需要 handle（0~65535）")?;
    if h > u16::MAX as u64 {
        return Err(format!("handle 超出 16 位：{h}"));
    }
    out.insert("handle".into(), serde_json::Value::from(h));
    Ok(())
}

/// 地址规范化：冒号分隔的 6 对十六进制，统一输出小写。
fn normalize_addr(addr: &str) -> Result<String, String> {
    let a = addr.trim();
    let parts: Vec<&str> = a.split(':').collect();
    if parts.len() != 6 {
        return Err(format!("addr 形如 aa:bb:cc:dd:ee:ff，实际 {a}"));
    }
    let mut out = Vec::with_capacity(17);
    for (i, p) in parts.iter().enumerate() {
        if p.len() != 2 || !p.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("addr 形如 aa:bb:cc:dd:ee:ff，实际 {a}"));
        }
        if i > 0 {
            out.push(b':');
        }
        out.extend(p.to_ascii_lowercase().bytes());
    }
    Ok(String::from_utf8(out).expect("hex 字节必为 UTF-8"))
}

// ---- 管理面端点（/api/echomuse/devices/{id}/ble/*，同 admin gate 鉴权） ----

/// 挂载蓝牙代理管理面路由（由 [`crate::app::ws::router`] merge，与 admin_routes 并列）。
pub(crate) fn ble_routes() -> axum::Router<Arc<Engines>> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/api/echomuse/devices/{id}/ble/adverts", get(ble_adverts))
        .route("/api/echomuse/devices/{id}/ble/state", get(ble_state))
        .route("/api/echomuse/devices/{id}/ble/gatt", post(ble_gatt))
}

/// `[echomuse].ble_proxy=false` 时的统一 404（代理未开启不暴露任何信息）。
fn ble_disabled_404() -> Response {
    json(
        StatusCode::NOT_FOUND,
        serde_json::json!({ "error": "ble proxy disabled" }),
    )
}

fn offline_409() -> Response {
    json(
        StatusCode::CONFLICT,
        serde_json::json!({ "error": "device offline" }),
    )
}

async fn ble_adverts(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    if !engines.config.echomuse.ble_proxy {
        return ble_disabled_404();
    }
    let Some((at, adverts)) = hub_ble(&id, |l| l.adverts_snapshot()) else {
        return offline_409();
    };
    json(
        StatusCode::OK,
        serde_json::json!({
            "device_id": id,
            "updatedAt": if at == 0 { serde_json::Value::Null } else { serde_json::json!(at) },
            "count": adverts.len(),
            "adverts": adverts,
        }),
    )
}

async fn ble_state(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    if !engines.config.echomuse.ble_proxy {
        return ble_disabled_404();
    }
    let Some((slots, events)) = hub_ble(&id, |l| l.state_snapshot()) else {
        return offline_409();
    };
    json(
        StatusCode::OK,
        serde_json::json!({
            "device_id": id,
            "slots": slots.unwrap_or(serde_json::Value::Null),
            "events": events,
        }),
    )
}

/// GATT 直驱：校验 body → 设备能力/在线检查 → 分配 req 并登记等待 → `0x08` 下发 →
/// 等待同 `req` 的 result（超时/离线明确回错）。返回设备的 result JSON 原文
/// （`ok:false` 也是有效应答，错误码语义见协议）。
async fn ble_gatt(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
    body: Option<axum::Json<serde_json::Value>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    if !engines.config.echomuse.ble_proxy {
        return ble_disabled_404();
    }
    let payload = body.map(|axum::Json(v)| v).unwrap_or(serde_json::json!({}));
    let request = match build_gatt_request(&payload) {
        Ok(v) => v,
        Err(e) => return json(StatusCode::BAD_REQUEST, serde_json::json!({ "error": e })),
    };
    // 能力门槛：设备 register 未声明 ble_connect 时按协议不下发（设备会静默忽略）。
    let capable = registry::global(&engines.config)
        .get(&id)
        .map(|r| r.capabilities.iter().any(|c| c == "ble_connect"));
    match capable {
        None => {
            return json(
                StatusCode::NOT_FOUND,
                serde_json::json!({ "error": "device not found" }),
            )
        }
        Some(false) => {
            return json(
                StatusCode::CONFLICT,
                serde_json::json!({ "error": "device not ble capable" }),
            )
        }
        Some(true) => {}
    }
    // 分配 req + 登记（offline = 不在 hub = 会话未建立/已结束）。
    let (tx, rx) = tokio::sync::oneshot::channel();
    let Some(req_id) = hub_ble(&id, |l| {
        let req = l.alloc_req();
        l.register(req, tx);
        req
    }) else {
        return offline_409();
    };
    let mut full = request;
    if let Some(obj) = full.as_object_mut() {
        obj.insert("req".into(), serde_json::Value::from(req_id));
    }
    // 0x08 下发；发送失败（会话刚退出）按离线口径并回收登记。
    let Some(data) = hub_data(&id) else {
        hub_ble(&id, |l| l.forget(req_id));
        return offline_409();
    };
    if data
        .send(frames::frame_ble_gatt(&full.to_string()))
        .is_err()
    {
        hub_ble(&id, |l| l.forget(req_id));
        return offline_409();
    }
    match tokio::time::timeout(GATT_WAIT, rx).await {
        Ok(Ok(result)) => json(StatusCode::OK, result),
        // 链路收尾（reset 丢弃 sender）或会话结束：等待方视为离线。
        Ok(Err(_)) => offline_409(),
        Err(_) => {
            hub_ble(&id, |l| l.forget(req_id));
            json(
                StatusCode::GATEWAY_TIMEOUT,
                serde_json::json!({ "error": "ble request timeout" }),
            )
        }
    }
}
