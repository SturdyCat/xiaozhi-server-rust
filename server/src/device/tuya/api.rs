//! `/api/tuya/*` 管理面（axum，经管理鉴权，风格对齐 `echomuse::admin`）。
//!
//! ## 端点（架构 §5）
//!
//! | 方法/路径 | 说明 |
//! |---|---|
//! | GET  /api/tuya/status | 凭证/同步状态（secret 只出"已配置 + 掩码"） |
//! | POST /api/tuya/credentials | 录入/更新凭证（secret 留空 = 保留旧值） |
//! | DELETE /api/tuya/credentials（+ POST /delete 别名） | 清除凭证 |
//! | POST /api/tuya/sync | 立即同步（60s 闸门） |
//! | GET  /api/tuya/devices | 设备列表（?online=true/false 过滤） |
//! | GET  /api/tuya/devices/{id} | 详情 + DP 快照 |
//! | POST /api/tuya/devices/{id}/dp | 控制 {dp, value} |
//! | POST /api/tuya/devices/{id}/alias | 本地重命名 |
//! | POST /api/tuya/cloud/proxy | **云调用调试代理**：`{method, path, query?, body?}`
//!   → 服务端按官方签名转发涂鸦 OpenAPI，回包原样透传（联调免写临时脚本） |
//!
//! ## 安全铁律
//!
//! - `access_secret` **只写不读**：任何响应/日志永不回显（[`status_payload`] 只出
//!   `configured` + 掩码，见 `api_tests` 的"不回显"断言）；
//! - `[tuya].enabled=false` 时全部 404（与 echomuse 管理面同一口径）；
//! - 凭证写入走 `config_store`（配置库真源），保存后即时丢弃 token 缓存；
//! - 调试代理只放行 `/v1.` 前缀路径 + GET/POST/DELETE（`cloud_proxy_call` 校验），
//!   回包含 access_token 等临时凭据属调试用途，端点经管理鉴权、与其它 /api 同权。
//!
//! 响应体纯函数（[`status_payload`] / [`devices_payload`] / [`credentials_apply`]）
//! 外置测试直驱（`api_tests.rs`），handler 只做鉴权、取参、组装。

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

use crate::config::TuyaConfig;
use crate::gateway::ws::auth_ok;
use crate::runtime::Engines;

use super::lan::ControlFailure;
use super::store::DeviceRow;
use super::{SyncRecordView, UnregisteredDevice};

/// 挂载管理面路由（由 [`crate::gateway::ws::router`] merge）。
pub(crate) fn routes() -> axum::Router<Arc<Engines>> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/api/tuya/status", get(status))
        .route("/api/tuya/credentials", post(credentials_put))
        .route("/api/tuya/credentials/delete", post(credentials_delete))
        .route(
            "/api/tuya/credentials",
            axum::routing::delete(credentials_delete),
        )
        .route("/api/tuya/sync", post(sync_now))
        .route("/api/tuya/devices", get(devices_list))
        .route("/api/tuya/devices/{id}", get(device_get))
        .route("/api/tuya/devices/{id}/dp", post(device_dp))
        .route("/api/tuya/devices/{id}/alias", post(device_alias))
        .route("/api/tuya/cloud/proxy", post(cloud_proxy))
}

// ---- 响应体纯函数（api_tests 直驱）----

/// access_id 掩码：`ab****ef`；≤4 位或空 → `****`。
pub(crate) fn mask_access_id(id: &str) -> String {
    let chars: Vec<char> = id.trim().chars().collect();
    if chars.len() <= 4 {
        "****".to_string()
    } else {
        format!(
            "{}****{}",
            chars[..2].iter().collect::<String>(),
            chars[chars.len() - 2..].iter().collect::<String>()
        )
    }
}

/// 凭证写入决策（纯函数）：region 必须在白名单；access_id 非空必填；
/// access_secret 留空且已配置 → 保留旧值（管理员只想改区域/ID 的场景）。
pub(crate) fn credentials_apply(cfg: &mut TuyaConfig, body: &Value) -> Result<(), String> {
    let region = body
        .get("region")
        .and_then(Value::as_str)
        .unwrap_or(&cfg.region)
        .trim()
        .to_ascii_lowercase();
    if !crate::device::tuya::config::REGIONS.contains(&region.as_str()) {
        return Err(format!(
            "region 只支持 {:?}",
            crate::device::tuya::config::REGIONS
        ));
    }
    let access_id = body
        .get("access_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if access_id.is_empty() {
        return Err("access_id 不能为空".into());
    }
    let access_secret = body
        .get("access_secret")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    cfg.region = region;
    cfg.access_id = access_id;
    if !access_secret.is_empty() {
        cfg.access_secret = access_secret;
    } else if cfg.access_secret.is_empty() {
        return Err("access_secret 不能为空（已配置过可留空保留旧值）".into());
    }
    cfg.normalize();
    Ok(())
}

/// 清除凭证（纯函数）：三字段全清，enabled/周期不动。
pub(crate) fn credentials_clear(cfg: &mut TuyaConfig) {
    cfg.access_id = String::new();
    cfg.access_secret = String::new();
    cfg.normalize();
}

/// status 端点响应体。**永不包含 access_secret**（测试钉死）。
///
/// `lan_unregistered` 是 LAN 扫描发现的未登记设备快照（不纳管，目录以云同步为准）；
/// `last_sync.message` 带同步失败的具体原因（如 code 1108），UI 状态条据此透出。
pub(crate) fn status_payload(
    enabled: bool,
    cfg: &TuyaConfig,
    last_sync: Option<&SyncRecordView>,
    syncing: bool,
    unregistered: &[UnregisteredDevice],
) -> Value {
    json!({
        "enabled": enabled,
        "configured": cfg.credentials_ready(),
        "access_id_masked": if cfg.access_id.is_empty() { String::new() } else { mask_access_id(&cfg.access_id) },
        "region": cfg.region,
        "sync_interval_secs": cfg.sync_interval_secs,
        "syncing": syncing,
        "last_sync": last_sync.map(|s| json!({
            "finished_at": s.finished_at,
            "age_ms": s.age_ms,
            "result": s.result,
            "device_count": s.device_count,
            "message": s.message,
        })),
        "lan_unregistered": unregistered
            .iter()
            .map(|u| json!({
                "device_id": u.device_id,
                "ip": u.ip,
                "age_ms": (super::store::now_ms() - u.discovered_at_ms).max(0),
            }))
            .collect::<Vec<_>>(),
        "store_path": String::new(), // handler 填（涉及落点，纯函数不管路径）
    })
}

/// 设备列表响应体（不回显 local_key；展示名 alias 优先于 name）。
pub(crate) fn devices_payload(rows: &[DeviceRow]) -> Value {
    let devices: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "device_id": row.device_id,
                "name": if row.alias.is_empty() { row.name.clone() } else { row.alias.clone() },
                "model": row.model,
                "online": row.online,
                "ip": row.ip,
                "updated_at": row.updated_at,
            })
        })
        .collect();
    let online = rows.iter().filter(|r| r.online).count();
    json!({ "devices": devices, "total": rows.len(), "online": online })
}

/// 设备详情响应体：设备信息 + DP 快照（writable = 控制成功过）。
pub(crate) fn device_detail_payload(
    row: &DeviceRow,
    dps: Option<&HashMap<String, Value>>,
    writable_of: &dyn Fn(&str) -> bool,
) -> Value {
    let mut dps_json: Vec<Value> = Vec::new();
    if let Some(map) = dps {
        let mut ids: Vec<&String> = map.keys().collect();
        ids.sort();
        for dp in ids {
            let value = &map[dp];
            dps_json.push(json!({
                "dp": dp,
                "value": value,
                "value_text": value_text(value),
                "writable": writable_of(dp),
                "control": control_shape(value),
            }));
        }
    }
    json!({
        "device_id": row.device_id,
        "name": if row.alias.is_empty() { row.name.clone() } else { row.alias.clone() },
        "model": row.model,
        "online": row.online,
        "ip": row.ip,
        "dps": dps_json,
    })
}

/// DP 值 → 展示文本（bool→开/关；空对象/数组 → JSON 文本）。
fn value_text(value: &Value) -> String {
    match value {
        Value::Bool(true) => "开".into(),
        Value::Bool(false) => "关".into(),
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// 控件形态（UI 据此渲染）：boolean → 开关按钮；number → 数值输入；其余只读。
fn control_shape(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "switch",
        Value::Number(_) => "number",
        _ => "readonly",
    }
}

/// 控制失败响应体（三归类 + 建议动作；不回显任何凭据/报文）。
pub(crate) fn failure_payload(failure: &ControlFailure) -> Value {
    json!({
        "error": failure.message,
        "kind": failure.kind,
        "advice": failure.advice,
    })
}

// ---- 云调用调试代理（`POST /api/tuya/cloud/proxy`）----

/// 代理调用参数（`cloud_proxy_call` 校验后的产物，handler 直发 [`super::cloud`]）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CloudProxyCall {
    pub method: &'static str,
    /// path?query 一体（query 键已按字典序排好，与签名共用同一串）。
    pub path_query: String,
    /// 原始 JSON 文本；非 POST 为空串。
    pub body_json: String,
}

/// 代理请求体校验（纯函数）：method 白名单 + path 前缀闸 + query 字典序 + body 限 POST。
///
/// 限制理由：代理以**用户凭证**签名转发，必须收敛为"涂鸦 OpenAPI 调试器"而非
/// 通用代理——`/v1.` 前缀挡住任意路径，方法白名单挡住非常规调用。
pub(crate) fn cloud_proxy_call(v: &Value) -> Result<CloudProxyCall, String> {
    let method = match v
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_ascii_uppercase()
        .as_str()
    {
        "GET" => "GET",
        "POST" => "POST",
        "DELETE" => "DELETE",
        other => return Err(format!("method 只支持 GET/POST/DELETE（收到 {other}）")),
    };
    let path = v
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if !path.starts_with("/v1.") {
        return Err("path 必须以 /v1. 开头（涂鸦 OpenAPI 路径，如 /v1.0/devices）".into());
    }
    if path.contains(' ') || path.contains('?') || path.contains('#') {
        return Err("path 不能含空格、? 或 #（query 参数用 query 对象传）".into());
    }
    let mut path_query = path;
    if let Some(q) = v.get("query").filter(|q| q.is_object()) {
        let map = q.as_object().expect("is_object 已判定");
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        let pairs: Vec<String> = keys
            .iter()
            .map(|k| {
                let val = &map[*k];
                let rendered = val.to_string();
                let text = val.as_str().unwrap_or(&rendered);
                format!(
                    "{}={}",
                    encode_query_component(k),
                    encode_query_component(text)
                )
            })
            .collect();
        path_query = format!("{}?{}", path_query, pairs.join("&"));
    }
    let body_json = match v.get("body") {
        Some(b) if !b.is_null() => {
            if method != "POST" {
                return Err("只有 POST 请求允许带 body".into());
            }
            serde_json::to_string(b).map_err(|e| format!("body 序列化失败: {e}"))?
        }
        _ => String::new(),
    };
    Ok(CloudProxyCall {
        method,
        path_query,
        body_json,
    })
}

/// query 键/值的最小 URL 编码（涂鸦 query 值实践中是 id/游标/分页数，普通字符直通）。
fn encode_query_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 代理回包响应体：云回包原样透传（调试所见即所得），外层带 HTTP 状态。
pub(crate) fn cloud_proxy_payload(status: u16, raw_body: &str) -> Value {
    let body: Value = serde_json::from_str(raw_body).unwrap_or(Value::String(raw_body.to_string()));
    json!({ "status": status, "body": body })
}

/// 代理失败响应体（网络/鉴权/云业务三归类，消息不含 secret）。
pub(crate) fn cloud_proxy_error(kind: &'static str, message: &str) -> Value {
    json!({ "kind": kind, "error": message })
}

// ---- axum handlers（鉴权 → enabled 闸门 → 组装）----

/// 管理面共用前置：鉴权 + `[tuya].enabled` 闸门；Some = 已定型的错误响应。
fn gate(
    engines: &Engines,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Option<Response> {
    if !auth_ok(headers, query, &engines.config.server) {
        return Some((StatusCode::UNAUTHORIZED, "unauthorized").into_response());
    }
    if !engines.config.tuya.enabled {
        return Some(json_404("tuya disabled"));
    }
    None
}

fn json_404(error: &str) -> Response {
    (StatusCode::NOT_FOUND, axum::Json(json!({ "error": error }))).into_response()
}

fn json_ok(body: Value) -> Response {
    (StatusCode::OK, axum::Json(body)).into_response()
}

fn json_error(code: StatusCode, body: Value) -> Response {
    (code, axum::Json(body)).into_response()
}

/// 读最新配置（热生效：一律经 config_store，spawn_blocking）。
async fn load_config(engines: &Engines) -> Result<TuyaConfig, String> {
    let store = engines.config_store.clone();
    let cfg = tokio::task::spawn_blocking(move || store.load())
        .await
        .map_err(|e| format!("配置读取任务失败: {e}"))?
        .map_err(|e| format!("配置读取失败: {e}"))?;
    Ok(cfg.tuya)
}

/// 写回配置（凭证端点用）：整份 load → 改 [tuya]（apply 校验失败即中止）→ persist。
async fn persist_config(
    engines: &Engines,
    apply: impl FnOnce(&mut TuyaConfig) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let store = engines.config_store.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut cfg = store.load().map_err(|e| format!("配置读取失败: {e}"))?;
        apply(&mut cfg.tuya)?;
        crate::config::Config::normalize(&mut cfg);
        store
            .persist(&cfg)
            .map_err(|e| format!("配置写回失败: {e}"))
    })
    .await
    .map_err(|e| format!("配置写回任务失败: {e}"))?
}

async fn status(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    let cfg = match load_config(&engines).await {
        Ok(c) => c,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e })),
    };
    let last_sync = engines
        .tuya
        .last_sync()
        .await
        .ok()
        .flatten()
        .as_ref()
        .map(SyncRecordView::from_record);
    let mut body = status_payload(
        cfg.enabled,
        &cfg,
        last_sync.as_ref(),
        engines.tuya.syncing(),
        &engines.tuya.unregistered_snapshot(),
    );
    body["store_path"] = json!(engines.tuya.store_path().display().to_string());
    json_ok(body)
}

async fn credentials_put(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    State(engines): State<Arc<Engines>>,
    body: Option<axum::Json<Value>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    let payload = body.map(|axum::Json(v)| v).unwrap_or(json!({}));
    // 先在当前配置副本上校验（纯函数），通过才写库
    let mut probe = match load_config(&engines).await {
        Ok(c) => c,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e })),
    };
    if let Err(err) = credentials_apply(&mut probe, &payload) {
        return json_error(StatusCode::BAD_REQUEST, json!({ "error": err }));
    }
    if let Err(e) = persist_config(&engines, move |cfg| credentials_apply(cfg, &payload)).await {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e }));
    }
    engines.tuya.invalidate_credentials();
    json_ok(json!({ "ok": true, "message": "凭证已保存（secret 不再回显）" }))
}

async fn credentials_delete(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    if let Err(e) = persist_config(&engines, |cfg| {
        credentials_clear(cfg);
        Ok(())
    })
    .await
    {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e }));
    }
    engines.tuya.invalidate_credentials();
    json_ok(json!({ "ok": true, "message": "凭证已清除" }))
}

async fn sync_now(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    let cfg = match load_config(&engines).await {
        Ok(c) => c,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e })),
    };
    let outcome = engines.tuya.sync_now(&cfg).await;
    json_ok(json!({ "result": outcome.result, "message": outcome.message }))
}

async fn devices_list(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    let rows = match engines.tuya.list_devices().await {
        Ok(rows) => rows,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e })),
    };
    // ?online=true/false 过滤（列表页"只看在线"开关）
    let rows: Vec<DeviceRow> = match query.get("online").map(|v| v.to_ascii_lowercase()) {
        Some(v) if v == "true" || v == "1" => rows.into_iter().filter(|r| r.online).collect(),
        Some(v) if v == "false" || v == "0" => rows.into_iter().filter(|r| !r.online).collect(),
        _ => rows,
    };
    json_ok(devices_payload(&rows))
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
    let row = match engines.tuya.get_device(&id).await {
        Ok(Some(row)) => row,
        Ok(None) => return json_404("device not found"),
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e })),
    };
    let engine = &engines.tuya;
    let dps = engine.cached_dps(&id);
    let writable_of = |dp: &str| engine.dp_writable(&id, dp);
    json_ok(device_detail_payload(&row, dps.as_ref(), &writable_of))
}

async fn device_dp(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
    body: Option<axum::Json<Value>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    let payload = body.map(|axum::Json(v)| v).unwrap_or(json!({}));
    let Some(dp) = payload
        .get("dp")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return json_error(
            StatusCode::BAD_REQUEST,
            json!({ "error": "需要 dp（数据点 ID）" }),
        );
    };
    let Some(value) = payload.get("value").cloned() else {
        return json_error(
            StatusCode::BAD_REQUEST,
            json!({ "error": "需要 value（目标值）" }),
        );
    };
    match engines.tuya.set_dp(&id, &dp, value).await {
        Ok(dps) => {
            let dps_json: serde_json::Map<String, Value> = dps.into_iter().collect();
            json_ok(json!({ "dps": dps_json, "message": "已下发" }))
        }
        Err(failure) => json_error(
            StatusCode::from_u16(failure.status_code()).unwrap_or(StatusCode::CONFLICT),
            failure_payload(&failure),
        ),
    }
}

async fn device_alias(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
    body: Option<axum::Json<Value>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    let payload = body.map(|axum::Json(v)| v).unwrap_or(json!({}));
    let Some(alias) = payload.get("alias").and_then(Value::as_str) else {
        return json_error(
            StatusCode::BAD_REQUEST,
            json!({ "error": "需要 alias（本地展示名，可为空串清除）" }),
        );
    };
    match engines.tuya.set_alias(&id, alias).await {
        Ok(()) => json_ok(json!({ "ok": true })),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e })),
    }
}

/// 云调用调试代理：校验 → 读凭证 → 签名转发 → 原样透传。
async fn cloud_proxy(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    State(engines): State<Arc<Engines>>,
    body: Option<axum::Json<Value>>,
) -> Response {
    if let Some(r) = gate(&engines, &headers, &query) {
        return r;
    }
    let payload = body.map(|axum::Json(v)| v).unwrap_or(json!({}));
    let call = match cloud_proxy_call(&payload) {
        Ok(c) => c,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, json!({ "error": e })),
    };
    let cfg = match load_config(&engines).await {
        Ok(c) => c,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e })),
    };
    if !cfg.credentials_ready() {
        return json_error(
            StatusCode::BAD_REQUEST,
            json!({ "error": "涂鸦凭证未配置：先 POST /api/tuya/credentials 录入" }),
        );
    }
    let mut client = engines.tuya.cloud.lock().await;
    match client
        .proxy_request(
            &cfg.region,
            &cfg.access_id,
            &cfg.access_secret,
            call.method,
            &call.path_query,
            &call.body_json,
        )
        .await
    {
        Ok(resp) => json_ok(cloud_proxy_payload(resp.status, &resp.body)),
        Err(e) => {
            let kind = match &e {
                super::cloud::CloudError::Auth(_) => "auth",
                super::cloud::CloudError::Network(_) => "network",
                super::cloud::CloudError::Api(_) => "api",
            };
            json_error(
                StatusCode::BAD_GATEWAY,
                cloud_proxy_error(kind, &e.message()),
            )
        }
    }
}
