//! `/api/echomuse/*` 管理面（HTTP，经 `auth_ok` 鉴权，风格对齐 [ws::router]）。
//!
//! 端点：
//! - `GET  /api/echomuse/devices`：registry 全表 + hub 实时合并（online/connected_secs）。
//! - `GET  /api/echomuse/devices/{id}`：单台详情（404 不存在）。
//! - `POST /api/echomuse/devices/{id}/approve`：`{"label"?}` 认领/改名（404 不存在）。
//! - `POST /api/echomuse/devices/{id}/command`：`{"action":"reboot"}` 或
//!   `{"action":"wifi","ssid":..,"psk":..}`——在线（hub 有 ctl_tx）经 `ControlOut::Json`
//!   下发并返回 202；离线 409；`[echomuse].enabled=false` 时全部 404（与 /em/control 一致）。
//!
//! 响应决策抽成纯函数（[`devices_payload`]/[`approve`]/[`command_plan`]）供外置测试
//! channel 直驱断言；axum handler 只做鉴权、取参和发送。
//!
//! [ws::router]: crate::app::ws::router

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::engine::Engines;

use super::registry::{self, Registry};
use super::{hub_ctl, hub_status, ControlOut};

/// 挂载管理面路由（由 [`crate::app::ws::router`] merge，与 em_routes 并列）。
pub(crate) fn admin_routes() -> axum::Router<Arc<Engines>> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/api/echomuse/devices", get(devices_list))
        .route("/api/echomuse/devices/{id}", get(device_get))
        .route("/api/echomuse/devices/{id}/approve", post(device_approve))
        .route("/api/echomuse/devices/{id}/command", post(device_command))
        .route("/api/echomuse/devices/{id}/settings", post(device_settings))
}

// ---- 响应构造（纯函数，测试直驱） ----

/// 统一 JSON 响应（`/api` 风格：application/json; charset=utf-8）。ble 管理面共用。
pub(super) fn json(code: StatusCode, body: serde_json::Value) -> Response {
    match serde_json::to_string(&body) {
        Ok(text) => (
            code,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            text,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("序列化失败: {e}"),
        )
            .into_response(),
    }
}

/// `[echomuse].enabled=false` 时的统一 404（管理面对关闭状态不暴露任何信息）。
fn disabled_404() -> Response {
    json(
        StatusCode::NOT_FOUND,
        serde_json::json!({ "error": "echomuse disabled" }),
    )
}

/// devices 列表体：registry 全表（按 device_id 排序）left join hub 在线状态。
pub(crate) fn devices_payload(enabled: bool, reg: &Registry) -> serde_json::Value {
    let online: HashMap<String, u64> = hub_status().into_iter().collect();
    let devices: Vec<serde_json::Value> = reg
        .list()
        .iter()
        .map(|r| r.payload(online.get(&r.device_id).copied()))
        .collect();
    serde_json::json!({ "enabled": enabled, "devices": devices })
}

/// 单台详情体；registry 无记录返回 None（handler 转 404）。
pub(crate) fn device_payload(reg: &Registry, id: &str) -> Option<serde_json::Value> {
    let online: HashMap<String, u64> = hub_status().into_iter().collect();
    reg.get(id).map(|r| r.payload(online.get(id).copied()))
}

/// approve 决策：Some = 更新后的记录体；None = 404。
pub(crate) fn approve(
    reg: &Registry,
    id: &str,
    body: &serde_json::Value,
) -> Option<serde_json::Value> {
    let label = body.get("label").and_then(|l| l.as_str());
    let secs = hub_status()
        .into_iter()
        .find(|(id2, _)| id2.as_str() == id)
        .map(|(_, s)| s);
    reg.approve(id, label).map(|r| r.payload(secs))
}

/// command 消息体：按 action 构造下发 JSON（命名遵循 EchoMuse
/// `device-controller-interface.md`；设备不认识的 type 会忽略，真机支持待验证）。
fn command_out(body: &serde_json::Value) -> Result<String, &'static str> {
    match body.get("action").and_then(|a| a.as_str()) {
        Some("reboot") => Ok(serde_json::json!({ "type": "reboot" }).to_string()),
        Some("wifi") => {
            let ssid = body
                .get("ssid")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .trim();
            if ssid.is_empty() {
                return Err("wifi 需要 ssid");
            }
            let psk = body.get("psk").and_then(|s| s.as_str()).unwrap_or("");
            Ok(serde_json::json!({ "type": "wifi_change", "ssid": ssid, "psk": psk }).to_string())
        }
        _ => Err("action 只支持 reboot / wifi"),
    }
}

/// settings 白名单：唤醒词相关且设备实际执行的 config 键（协议 camelCase，
/// 部分"零值忽略"更新；其余设备键走默认，不需要管理面暴露）。
/// `bleProxyEnabled`/`bleProxyConnections` 是蓝牙代理的每设备开关（接入时由
/// `config_baseline` 打基线，单台可覆盖，例如关连接留扫描）。
const SETTINGS_KEYS: &[(&str, &str)] = &[
    ("wakeWordEnabled", "bool"),
    ("owwOnDevice", "bool"),
    ("owwModel", "string"),
    ("owwThreshold", "number"),
    ("wakeSound", "bool"),
    ("wakeSoundLevel", "number"),
    ("wakeMic", "number"),
    ("bleProxyEnabled", "bool"),
    ("bleProxyConnections", "bool"),
];

/// 过滤并校验 settings body：只留白名单键且类型匹配（`number` 接受整/浮）。
/// 返回 (过滤后键集, 首个错误描述)。
pub(crate) fn filter_settings(
    body: &serde_json::Value,
) -> (
    std::collections::HashMap<String, serde_json::Value>,
    Option<String>,
) {
    let mut out = HashMap::new();
    let Some(obj) = body.as_object() else {
        return (out, Some("body 必须是 JSON 对象".into()));
    };
    for (k, v) in obj {
        let Some((_, kind)) = SETTINGS_KEYS.iter().find(|(name, _)| name == k) else {
            return (HashMap::new(), Some(format!("不支持的设置键: {k}")));
        };
        let ok = match *kind {
            "bool" => v.is_boolean(),
            "string" => v.is_string(),
            _ => v.is_number(),
        };
        if !ok {
            return (HashMap::new(), Some(format!("设置键 {k} 需要 {kind}")));
        }
        out.insert(k.clone(), v.clone());
    }
    (out, None)
}

/// settings 决策：enabled 闸门 → 存在性 → body 校验 → 在线才下发。
/// 返回 (状态码, 响应体, 待下发 config 列表)。离线时配置仍持久化（设备下次
/// 接入由 control_session 推送）。
pub(crate) fn settings_plan(
    enabled: bool,
    online: bool,
    known: bool,
    body: &serde_json::Value,
) -> (StatusCode, serde_json::Value, Vec<ControlOut>) {
    if !enabled {
        return (
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "echomuse disabled" }),
            Vec::new(),
        );
    }
    if !known {
        return (
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "device not found" }),
            Vec::new(),
        );
    }
    let (settings, err) = filter_settings(body);
    if let Some(err) = err {
        return (
            StatusCode::BAD_REQUEST,
            serde_json::json!({ "error": err }),
            Vec::new(),
        );
    }
    let push = super::config_push_with(&serde_json::Value::Object(
        settings.clone().into_iter().collect(),
    ));
    let outs: Vec<ControlOut> = if online {
        push.into_iter().map(ControlOut::Json).collect()
    } else {
        Vec::new()
    };
    (
        if online {
            StatusCode::ACCEPTED
        } else {
            StatusCode::OK
        },
        serde_json::json!({ "persisted": true, "delivered": online }),
        outs,
    )
}

/// command 三段决策（enabled / online / body 合法性），返回 (状态码, 响应体, 待下发消息)。
pub(crate) fn command_plan(
    enabled: bool,
    online: bool,
    body: &serde_json::Value,
) -> (StatusCode, serde_json::Value, Option<ControlOut>) {
    if !enabled {
        return (
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "echomuse disabled" }),
            None,
        );
    }
    if !online {
        return (
            StatusCode::CONFLICT,
            serde_json::json!({ "error": "device offline" }),
            None,
        );
    }
    match command_out(body) {
        Ok(text) => (
            StatusCode::ACCEPTED,
            serde_json::json!({ "queued": true }),
            Some(ControlOut::Json(text)),
        ),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            serde_json::json!({ "error": err }),
            None,
        ),
    }
}

// ---- axum handlers（鉴权 → enabled 闸门 → 纯函数决策 → 发送） ----

/// 管理面共用前置：鉴权 + `[echomuse].enabled` 闸门；Some = 已定型的错误响应。
pub(super) fn gate(
    engines: &Engines,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Option<Response> {
    if !crate::app::ws::auth_ok(headers, query, &engines.config.server) {
        return Some((StatusCode::UNAUTHORIZED, "unauthorized").into_response());
    }
    if !engines.config.echomuse.enabled {
        return Some(disabled_404());
    }
    None
}

async fn devices_list(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    let enabled = engines.config.echomuse.enabled;
    json(
        StatusCode::OK,
        devices_payload(enabled, registry::global(&engines.config)),
    )
}

async fn device_get(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    match device_payload(registry::global(&engines.config), &id) {
        Some(v) => json(StatusCode::OK, v),
        None => json(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "device not found" }),
        ),
    }
}

async fn device_approve(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
    body: Option<axum::Json<serde_json::Value>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    let payload = body.map(|axum::Json(v)| v).unwrap_or(serde_json::json!({}));
    match approve(registry::global(&engines.config), &id, &payload) {
        Some(v) => json(StatusCode::OK, v),
        None => json(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "device not found" }),
        ),
    }
}

async fn device_settings(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
    body: Option<axum::Json<serde_json::Value>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    let payload = body.map(|axum::Json(v)| v).unwrap_or(serde_json::json!({}));
    let reg = registry::global(&engines.config);
    let known = reg.get(&id).is_some();
    let online = hub_ctl(&id).is_some();
    let (code, reply, outs) = settings_plan(true, online, known, &payload);
    if code == StatusCode::BAD_REQUEST || code == StatusCode::NOT_FOUND {
        return json(code, reply);
    }
    // 合法性已验证：持久化（离线也保存，设备下次接入时由 control_session 推送）。
    let (settings, _) = filter_settings(&payload);
    if reg.set_settings(&id, settings).is_none() {
        return json(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "device not found" }),
        );
    }
    // 在线才可能有 outs；发送失败（control 会话刚退出）按离线口径回已持久化。
    let mut delivered = false;
    for out in outs {
        match hub_ctl(&id) {
            Some(ctl) if ctl.send(out).is_ok() => delivered = true,
            _ => {}
        }
    }
    json(
        code,
        serde_json::json!({ "persisted": true, "delivered": delivered }),
    )
}

async fn device_command(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
    body: Option<axum::Json<serde_json::Value>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    let payload = body.map(|axum::Json(v)| v).unwrap_or(serde_json::json!({}));
    let online = hub_ctl(&id).is_some();
    let (code, reply, out) = command_plan(true, online, &payload);
    // 在线才可能有 out；发送失败（control 会话刚退出）按离线口径回 409。
    if let Some(out) = out {
        match hub_ctl(&id) {
            Some(ctl) if ctl.send(out).is_ok() => {}
            _ => {
                return json(
                    StatusCode::CONFLICT,
                    serde_json::json!({ "error": "device offline" }),
                );
            }
        }
    }
    json(code, reply)
}
