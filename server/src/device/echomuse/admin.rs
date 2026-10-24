//! `/api/echomuse/*` 管理面（HTTP，经 `auth_ok` 鉴权，风格对齐 [ws::router]）。
//!
//! 端点：
//! - `GET  /api/echomuse/devices`：registry 全表 + hub 实时合并（online/connected_secs），
//!   回包携带设备面 `version`（与管理台 WS `GET /api/admin/ws` 的 hello/事件帧同源）。
//! - 变更通知走管理台 WebSocket `GET /api/admin/ws`（见 [`crate::gateway::ws::admin_ws`]：
//!   连接即发 hello 带当前版本、变化推 `echomuse_devices_changed`、ping/pong 应用层
//!   心跳）；本面的 `/devices/wait` 长挂轮询端点已删除。
//! - `GET  /api/echomuse/devices/{id}`：单台详情（404 不存在）。
//! - `POST /api/echomuse/devices/{id}/approve`：`{"label"?}` 认领/改名（404 不存在）。
//! - `POST /api/echomuse/devices/{id}/command`：`{"action":"wifi","ssid":..,"psk":..}`
//!   在线经 `ControlOut::Json` 排队并返回 202（非设备应用确认）；离线 409。
//!   旧 action=reboot 明确拒绝；确认重启、系统诊断、runtime/volume 见 system.rs。
//!   `[echomuse].enabled=false` 时全部 404（与 /em/control 一致）。
//! - `DELETE /api/echomuse/devices/{id}` 与等价别名 `POST .../delete`（客户端
//!   NetworkModule 只有 GET/POST）：删除登记记录（清理 mock/联调设备），只删静态
//!   登记、不断开在线会话，设备再 register 会重新登记。
//!
//! 响应决策抽成纯函数（[`devices_payload`]/[`approve`]/[`command_plan`]）供外置测试
//! channel 直驱断言；axum handler 只做鉴权、取参和发送。
//!
//! [ws::router]: crate::gateway::ws::router

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::runtime::Engines;

use super::registry::{self, Registry};
use super::{hub_ctl, ControlOut};

/// 挂载管理面路由（由 [`crate::gateway::ws::router`] merge，与 em_routes 并列）。
pub(crate) fn admin_routes() -> axum::Router<Arc<Engines>> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/api/echomuse/devices", get(devices_list))
        .route(
            "/api/echomuse/devices/{id}",
            get(device_get).delete(device_delete),
        )
        .route("/api/echomuse/devices/{id}/delete", post(device_delete))
        .route("/api/echomuse/devices/{id}/approve", post(device_approve))
        .route("/api/echomuse/devices/{id}/command", post(device_command))
        .route("/api/echomuse/devices/{id}/settings", post(device_settings))
        .merge(super::system::routes())
        .merge(super::oww::routes())
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

/// devices 列表体：registry 全表（按 device_id 排序）left join hub 在线状态；
/// `version` 是设备面变化号（管理台 WS hello/事件帧携带的版本号，客户端凭它对齐）。
pub(crate) fn devices_payload(enabled: bool, reg: &Registry) -> serde_json::Value {
    // Read the cursor BEFORE snapshots: a concurrent change must leave an older
    // cursor (and trigger the next /wait), never old rows with a newer cursor.
    let version = super::devices_version();
    let devices: Vec<serde_json::Value> = reg.list().iter().map(record_payload).collect();
    serde_json::json!({
        "enabled": enabled,
        "version": version,
        "devices": devices,
    })
}

/// 单台详情体；registry 无记录返回 None（handler 转 404）。
pub(crate) fn device_payload(reg: &Registry, id: &str) -> Option<serde_json::Value> {
    reg.get(id).as_ref().map(record_payload)
}

/// Online duration and identity are read under ONE hub lock, so a fast
/// replacement cannot combine the old connection age with the new session id.
fn record_payload(record: &registry::DeviceRecord) -> serde_json::Value {
    let hub = super::hub_iter();
    let live = hub.get(&record.device_id);
    let mut body = record.payload(live.map(|p| p.since.elapsed().as_secs()));
    body["session_id"] = live
        .map(|p| serde_json::json!(p.management.session_id.to_string()))
        .unwrap_or(serde_json::Value::Null);
    body
}

/// approve 决策：Some = 更新后的记录体；None = 404。
pub(crate) fn approve(
    reg: &Registry,
    id: &str,
    body: &serde_json::Value,
) -> Option<serde_json::Value> {
    let label = body.get("label").and_then(|l| l.as_str());
    reg.approve(id, label).as_ref().map(record_payload)
}

/// command 消息体：按 action 构造下发 JSON（命名遵循 EchoMuse
/// `device-controller-interface.md`；设备不认识的 type 会忽略，真机支持待验证）。
fn command_out(body: &serde_json::Value) -> Result<String, &'static str> {
    match body.get("action").and_then(|a| a.as_str()) {
        Some("reboot") => Err("reboot requires POST system/reboot with confirm=true"),
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
        _ => Err("action 只支持 wifi；重启请使用确认 system/reboot 接口"),
    }
}

/// settings 白名单：唤醒词相关且设备实际执行的 config 键（协议 camelCase，
/// 部分"零值忽略"更新）。
/// `bleProxyEnabled`/`bleProxyConnections` 是蓝牙代理的每设备开关（接入时由
/// `config_baseline` 打基线，单台可覆盖，例如关连接留扫描）。
///
/// **音频链路子集**（官方 `device/internal/config/config.go` 的 `ConfigMessage`
/// 全量字段里与"听到 / 听清"直接相关的那几项）：
/// - 设备默认值：`beamformingEnabled=true`、`beamAngle=-1`（自动追踪最响源）、
///   `aecEnabled=true`、`agcEnabled=true`、`vadThreshold=0.004`、`micGainDb=24`
///   （设备侧 clamp 0..42）、`adcDigitalGain=88`、`adcMicpga=40`。
/// - 服务端此前一个都不下发，设备只吃存储值/默认值；波束 baseline 校准不出来
///   导致 mic 静音时，管理面无从干预。暴露出来是为了现场排障：
///   `beamformingEnabled:false` 退回单麦（`wakeMic`，默认 0=中心麦）可一步验证
///   "是不是波束把信号吃掉了"；`micGainDb` / `vadThreshold` 用于调门限；
///   `aecEnabled` / `agcEnabled` 用于 A/B。
const SETTINGS_KEYS: &[(&str, &str)] = &[
    ("wakeWordEnabled", "bool"),
    ("owwOnDevice", "off|shadow|on"),
    ("owwModel", "string"),
    ("owwThreshold", "(0,1]"),
    ("wakeSound", "bool"),
    ("wakeSoundLevel", "quiet|medium|loud"),
    ("wakeMic", "integer 0..6"),
    ("bleProxyEnabled", "bool"),
    ("bleProxyConnections", "bool"),
    // 音频链路（0 值是合法设置，设备按指针类型区分"未设置"与"设为零"）
    ("beamformingEnabled", "bool"),
    ("beamAngle", "number -1..360"),
    ("aecEnabled", "bool"),
    ("agcEnabled", "bool"),
    ("vadThreshold", "(0,1]"),
    ("micGainDb", "integer 0..42"),
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
            "off|shadow|on" => matches!(v.as_str(), Some("off" | "shadow" | "on")),
            "quiet|medium|loud" => matches!(v.as_str(), Some("quiet" | "medium" | "loud")),
            "integer 0..6" => v.as_u64().is_some_and(|n| n <= 6),
            "integer 0..42" => v.as_u64().is_some_and(|n| n <= 42),
            "number -1..360" => v.as_f64().is_some_and(|n| (-1.0..=360.0).contains(&n)),
            "(0,1]" => v.as_f64().is_some_and(|n| n > 0.0 && n <= 1.0),
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
    if !crate::gateway::ws::auth_ok(headers, query, &engines.config.server) {
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

async fn device_delete(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    if registry::global(&engines.config).remove(&id) {
        super::bump_devices_version();
        json(
            StatusCode::OK,
            serde_json::json!({ "removed": true, "device_id": id }),
        )
    } else {
        json(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "device not found" }),
        )
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
        Some(v) => {
            // 批准/改名对列表可见。
            super::bump_devices_version();
            json(StatusCode::OK, v)
        }
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
    let (code, reply, _) = settings_plan(true, online, known, &payload);
    if code == StatusCode::BAD_REQUEST || code == StatusCode::NOT_FOUND {
        return json(code, reply);
    }
    // 合法性已验证：持久化（离线也保存，设备下次接入时由 control_session 推送）。
    let (settings, _) = filter_settings(&payload);
    let Some(record) = reg.set_settings(&id, settings) else {
        return json(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "device not found" }),
        );
    };
    // Push the merged snapshot: pushing only a delta would reapply baseline
    // owwOnDevice=on and silently undo a previously stored off/shadow mode.
    let outs: Vec<ControlOut> = if online {
        super::config_push_with(&serde_json::Value::Object(
            record.settings.into_iter().collect(),
        ))
        .into_iter()
        .map(ControlOut::Json)
        .collect()
    } else {
        Vec::new()
    };
    // 设置对列表可见（settings 快照随行）。
    super::bump_devices_version();
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
