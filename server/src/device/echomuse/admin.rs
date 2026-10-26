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
        .route("/api/echomuse/devices/{id}/music", post(device_music))
        .merge(super::system::routes())
        .merge(super::oww::routes())
        .merge(super::console::routes())
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

/// `{"error": …}` 单行错误响应：handler 里 `device not found` / `device offline`
/// 反复出现的同一形状（4 行 vs 1 行，且错误体只有一处定义）。
fn err_json(code: StatusCode, error: &str) -> Response {
    json(code, serde_json::json!({ "error": error }))
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
        // 配网闭环（官方三步：wifi_change → 设备重连回 wifi_result → wifi_commit）。
        // commit 前设备处于回滚窗口（连不上会自动退回旧网）；管理台在确认新网
        // 可用（/runtime 的 wifi_result.ok）后调用。
        Some("wifi_commit") => Ok(r#"{"type":"wifi_commit"}"#.into()),
        // 扫描周边网络；结果经 /runtime 的 wifi_scan 读取（wifi_scan_result 上行）。
        Some("wifi_scan") => Ok(r#"{"type":"wifi_scan"}"#.into()),
        _ => Err("action 只支持 wifi/wifi_scan/wifi_commit；重启请使用确认 system/reboot 接口"),
    }
}

/// settings 白名单：**设备端 `ConfigMessage` 实际消费的 40 个键**（`device/internal/
/// config/config.go` 的线缆结构）。controller 侧键（`ledScene`/`meter*`/`nsAsr`/
/// `saveUtterances`/`button*`…）在设备 `Device` 结构体里不存在、`vadChannel` 只由设备
/// 环境变量注入，都**不收**；`consolePassword`/`consoleTimeoutMin` 也不收（emOS **USB
/// 串口 console** 的 init，与网页控制台无关）。边界与理由见 `docs/echomuse.md`，并在
/// `admin_tests.rs::settings_keys_are_exactly_the_device_wire_contract` 钉死。
/// 协议 camelCase、"零值忽略"部分更新；`0`/`false` 是合法设置，设备按指针类型
/// 区分"未设置"。
///
/// 设备端官方初始默认值（排障基准），**逐字对齐**官方固件
/// `device/internal/config/config.go::loadDefaults()`（设备首次 `Get()` 的初始化值）。
/// 本服务器不强制推送这些键——设备本地即为官方默认；但当某台设备的存储
/// settings 被污染（如本仓库遇到的 `beamformingEnabled=false`、
/// `vadThreshold=0.06340`）时，需经 `POST /settings` 显式发回每个键（部分更新、
/// 零值忽略语义）才能覆盖旧值。完整官方真值：
/// vadThreshold=0.004, vadSpeechMs=80, vadSilenceMs=600, owwThreshold=0.5,
/// owwModel=hey_jarvis_v0.1, owwOnDevice=off, bargeInThreshold=0.05,
/// wakeWordEnabled=true, duckDb=-18, responseLevel=low, wakeSound=false,
/// wakeSoundLevel=medium, remoteVolumeArc=false, adcDigitalGain=88, adcMicpga=40,
/// micGainDb=24, beamAngle=-1, beamformingEnabled=true, wakeMic=0, agcEnabled=true,
/// aecEnabled=true, aecDelayMs=0, aecTailMs=300, aecRefSource=auto,
/// bleProxyEnabled=false, sendspinEnabled=false, sendspinUnpaired=false,
/// startupVolume=85, volumeButtonSound=true。
///
/// kind 语言（[`filter_settings`] 解析）：`bool` / `string` / `json`（任意 JSON）/
/// `float list`（数字数组）/ 含 `|` 的枚举 / `(0,1]` / `integer A..B` / `number A..B`。
pub(super) const SETTINGS_KEYS: &[(&str, &str)] = &[
    // 唤醒词
    ("wakeWordEnabled", "bool"),
    ("owwOnDevice", "off|shadow|on"),
    ("owwModel", "string"),
    ("owwThreshold", "(0,1]"),
    ("wakeSound", "bool"),
    ("wakeSoundLevel", "quiet|medium|loud"),
    ("wakeMic", "integer 0..6"),
    // VAD 时序（"多久算说完"的现场调参）
    ("vadThreshold", "(0,1]"),
    ("vadSpeechMs", "integer 0..60000"),
    ("vadSilenceMs", "integer 0..60000"),
    // 采集增益链（模拟前端 → 数字增益 → 处理增益）
    ("adcMicpga", "integer 0..127"),
    ("adcDigitalGain", "integer 0..127"),
    ("micGainDb", "integer 0..42"),
    ("startupVolume", "integer 0..127"),
    // AEC / AGC（aecRefSource: auto 检测硬件环回，hw/sw 钉死做 A/B）
    ("aecEnabled", "bool"),
    ("aecDelayMs", "integer 0..1000"),
    ("aecTailMs", "integer 0..500"),
    ("aecRefSource", "auto|hw|sw"),
    ("agcEnabled", "bool"),
    // 波束
    ("beamformingEnabled", "bool"),
    ("beamAngle", "number -1..360"),
    // 打断（barge-in）
    ("bargeInEnabled", "bool"),
    ("bargeInThreshold", "number 0..1"),
    // 音量交互与播报电平
    ("responseLevel", "low|medium|high"),
    ("volumeButtonSound", "bool"),
    ("remoteVolumeArc", "bool"),
    // 输出链（ack 声明 output_chain 后设备在 ALSA 写入处执行）
    ("eqBands", "float list"),
    ("eqLoudness", "bool"),
    ("bassGuardEnabled", "bool"),
    ("bassGuardDb", "number -60..24"),
    ("limiterEnabled", "bool"),
    ("limiterThreshold", "number -60..0"),
    ("limiterRelease", "number 10..2000"),
    // 音乐压低（duck）深度
    ("duckDb", "number -60..0"),
    // 蓝牙代理（每设备开关：接入时 config_baseline 打基线，单台可覆盖）
    ("bleProxyEnabled", "bool"),
    ("bleProxyConnections", "bool"),
    // Sendspin（Music Assistant 直连设备，不经本服务器；仅开关与名字）
    ("sendspinEnabled", "bool"),
    ("sendspinName", "string"),
    // 官方 Stage07 的「已配对另一台 MA」：设备线缆真有（`…config.go:601` 的 `*bool`）。
    ("sendspinUnpaired", "bool"),
    // 灯环：私听态默认动画 spec（设备本地渲染）
    ("listeningAnim", "json"),
];

fn range_ok(kind: &str, prefix: &str, v: &serde_json::Value) -> bool {
    let Some(rest) = kind.strip_prefix(prefix) else {
        return false;
    };
    let Some((lo, hi)) = rest.split_once("..") else {
        return false;
    };
    let (Ok(lo), Ok(hi)) = (lo.trim().parse::<f64>(), hi.trim().parse::<f64>()) else {
        return false;
    };
    if prefix == "integer " {
        v.as_i64().is_some_and(|n| n as f64 >= lo && n as f64 <= hi)
    } else {
        v.as_f64().is_some_and(|n| n >= lo && n <= hi)
    }
}

/// 决策失败的三元组：`(状态码, {"error": …}, 空下发)`。
///
/// [settings_plan]/[command_plan] 里六处失败分支原本各摊 6 行完全同构的元组字面量
/// （只有状态码/文案/空值类型不同），压成一处后所有失败响应共用同一形状，
/// 也避免"改一处忘了另一处"导致两个端点错误体不一致。
fn reject<T>(code: StatusCode, error: &str, empty: T) -> (StatusCode, serde_json::Value, T) {
    (code, serde_json::json!({ "error": error }), empty)
}

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
            "json" => v.is_object() || v.is_array(),
            "float list" => v
                .as_array()
                .is_some_and(|a| !a.is_empty() && a.iter().all(|x| x.is_number())),
            "(0,1]" => v.as_f64().is_some_and(|n| n > 0.0 && n <= 1.0),
            k if k.contains('|') => v
                .as_str()
                .is_some_and(|s| k.split('|').any(|option| option == s)),
            k if k.starts_with("integer ") => range_ok(k, "integer ", v),
            k if k.starts_with("number ") => range_ok(k, "number ", v),
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
        return reject(StatusCode::NOT_FOUND, "echomuse disabled", Vec::new());
    }
    if !known {
        return reject(StatusCode::NOT_FOUND, "device not found", Vec::new());
    }
    let (settings, err) = filter_settings(body);
    if let Some(err) = err {
        return reject(StatusCode::BAD_REQUEST, &err, Vec::new());
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
        return reject(StatusCode::NOT_FOUND, "echomuse disabled", None);
    }
    if !online {
        return reject(StatusCode::CONFLICT, "device offline", None);
    }
    match command_out(body) {
        Ok(text) => (
            StatusCode::ACCEPTED,
            serde_json::json!({ "queued": true }),
            Some(ControlOut::Json(text)),
        ),
        Err(err) => reject(StatusCode::BAD_REQUEST, err, None),
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
        None => err_json(StatusCode::NOT_FOUND, "device not found"),
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
        err_json(StatusCode::NOT_FOUND, "device not found")
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
        None => err_json(StatusCode::NOT_FOUND, "device not found"),
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
        return err_json(StatusCode::NOT_FOUND, "device not found");
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
                return err_json(StatusCode::CONFLICT, "device offline");
            }
        }
    }
    json(code, reply)
}

/// `POST /api/echomuse/devices/{id}/music`（音乐面，音源仅 DLNA）：
/// - `{"action":"scan"}` → 200 `{servers:[{name,location}]}`（SSDP 发现局域网
///   MediaServer，不依赖设备在线）；
/// - `{"action":"browse","location":...}` → 200 `{tracks:[{title,artist,album,url,duration}]}`
///   （SOAP 递归列出音频）；
/// - `{"action":"play_url","url":...}` → 202 `{queued}`（服务端拉流→解码 48k mono
///   →复用 [`super::music`] 通道逐帧推流，与语音混音）；
/// - `{"action":"stop"}` → 200（epoch 作废 + `music_flush`，不依赖 `music_dir`）。
async fn device_music(
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
    match payload.get("action").and_then(|a| a.as_str()) {
        // ── DLNA 音源：扫描/浏览不依赖设备在线与 music_dir ──
        Some("scan") => match super::dlna::discover(std::time::Duration::from_secs(4)).await {
            servers if !servers.is_empty() => {
                json(StatusCode::OK, serde_json::json!({ "servers": servers }))
            }
            _ => json(
                StatusCode::OK,
                serde_json::json!({
                    "servers": [],
                    "hint": "局域网未发现 DLNA 媒体服务器（确认服务器已开机且同网段；容器部署需 host 网络才能收到 SSDP 组播）"
                }),
            ),
        },
        Some("browse") => {
            let Some(location) = payload.get("location").and_then(|l| l.as_str()) else {
                return json(
                    StatusCode::BAD_REQUEST,
                    serde_json::json!({"error": "browse 需要 location"}),
                );
            };
            match super::dlna::browse(location).await {
                Ok(tracks) => json(StatusCode::OK, serde_json::json!({ "tracks": tracks })),
                Err(e) => json(StatusCode::BAD_REQUEST, serde_json::json!({ "error": e })),
            }
        }
        Some("play_url") => {
            let Some(url) = payload.get("url").and_then(|u| u.as_str()) else {
                return json(
                    StatusCode::BAD_REQUEST,
                    serde_json::json!({"error": "play_url 需要 url"}),
                );
            };
            let Some((_, live)) = super::hub_live(&id) else {
                return json(
                    StatusCode::CONFLICT,
                    serde_json::json!({ "error": "device offline" }),
                );
            };
            let Some(data_tx) = super::hub_data_tx(&id) else {
                return json(
                    StatusCode::CONFLICT,
                    serde_json::json!({ "error": "data channel offline" }),
                );
            };
            let plane = super::music::MusicPlane::new(live.fx.clone(), live.media.clone(), data_tx);
            match plane.play_url(url).await {
                Ok(()) => json(StatusCode::ACCEPTED, serde_json::json!({ "queued": true })),
                Err(e) => json(StatusCode::BAD_REQUEST, serde_json::json!({ "error": e })),
            }
        }
        // ── 停止：epoch 作废 + music_flush，只依赖设备在线（音乐面音源仅 DLNA） ──
        Some("stop") => {
            let Some((_, live)) = super::hub_live(&id) else {
                return json(
                    StatusCode::CONFLICT,
                    serde_json::json!({ "error": "device offline" }),
                );
            };
            let Some(data_tx) = super::hub_data_tx(&id) else {
                return json(
                    StatusCode::CONFLICT,
                    serde_json::json!({ "error": "data channel offline" }),
                );
            };
            super::music::MusicPlane::new(live.fx.clone(), live.media.clone(), data_tx).stop();
            json(StatusCode::OK, serde_json::json!({ "stopped": true }))
        }
        _ => json(
            StatusCode::BAD_REQUEST,
            serde_json::json!({ "error": "action 支持 scan/browse/play_url/stop" }),
        ),
    }
}
