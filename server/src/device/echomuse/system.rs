//! Restricted emOS management HTTP. No command/path supplied by a client is ever
//! interpolated into a shell. Scripts follow EchoMuse main 858987c/emos/README.md.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};

use super::{admin, registry, runtime, shell, ControlOut};
use crate::runtime::Engines;
use shell::ShellError;

// Fixed commands only. && preserves errors; no failed read becomes empty success.
pub(super) const INFO: &str =
    "uname -a && cat /etc/os-release && cat /proc/uptime && cat /proc/meminfo && df -k /data /run";
// Schedule through the shell, confirming scheduling only. PID1 owns emOS clean
// reboot. Redirect all inherited fds so closing this short shell won't kill it.
pub(super) const REBOOT: &str = "(sleep 1; kill -TERM 1) </dev/null >/dev/null 2>&1 &";

/// 固定日志源白名单（客户端下拉的 id 必须与这里逐一对应，见 `EchoMuseManagementState`
/// 的 `EM_LOG_SOURCES`）。`server` 是设备上**本机服务进程**的日志（EchoMuse 控制器把
/// 自己的运行日志写到 `/tmp/server.log`）——它就是原先必须插数据线、开串口 console 才能
/// `cat` 到的那份，远程读它就不必再接线；`net` 是网络协商日志。
pub(super) fn log_command(source: &str) -> Option<&'static str> {
    match source {
        "server" => Some("tail -c 49152 /tmp/server.log"),
        "messages" => Some("tail -c 49152 /run/messages"),
        "messages_last" => Some("tail -c 49152 /data/emos/messages.last"),
        "last_kmsg_prev" => Some("tail -c 49152 /data/emos/last_kmsg.prev"),
        "supervisor" => Some("tail -c 49152 /data/local/etc/echomuse/supervisor.log"),
        "net" => Some("tail -c 49152 /run/net.log"),
        _ => None,
    }
}

pub(super) fn routes() -> axum::Router<Arc<Engines>> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/api/echomuse/devices/{id}/runtime", get(get_runtime))
        .route("/api/echomuse/devices/{id}/volume", post(set_volume))
        .route("/api/echomuse/devices/{id}/system/info", get(get_info))
        .route("/api/echomuse/devices/{id}/system/logs", get(get_logs))
        .route("/api/echomuse/devices/{id}/system/reboot", post(reboot))
}

/// 系统操作鉴权（产品决策：**局域网默认可管理**）：
///
/// - `server.expected_token` 为空（默认）→ 管理台免 token 直接可用（[`admin::gate`] 放行）；
///   配置了 token 即**收紧**为必须鉴权（`?token=` / Bearer）——这就是"关闭默认可管理"的开关，
///   不再要求双 token 都非空才允许系统操作（旧口径会 503 并提示配置双 token，已按局域网
///   默认可管理的语义移除）。
/// - 设备侧继续看 `echomuse.token`（空 = 局域网信任设备；非空 = 设备必须带 `X-Em-Token`），
///   与 shell/BLE 闸门同一口径。
///
/// 也供 `oww.rs`（OWW 资产下发）复用：同样是"管理面鉴权 + 已批准 emOS + 在线且已认证"。
pub(super) fn system_gate(
    engines: &Engines,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
    id: &str,
) -> Result<(), Response> {
    if let Some(r) = admin::gate(engines, headers, query) {
        return Err(r);
    }
    shell::approved_emos(registry::global(&engines.config), id).map_err(|e| e.response())?;
    let Some((ctl, live)) = super::hub_live(id).filter(|(c, _)| !c.is_closed()) else {
        return Err(ShellError::Offline.response());
    };
    if !live.authenticated {
        return Err(ShellError::Unauthorized.response());
    }
    if live.peer.is_none() {
        return Err(ShellError::PeerUnavailable.response());
    }
    drop(ctl);
    Ok(())
}

async fn get_runtime(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if let Some(r) = admin::gate(&engines, &headers, &query) {
        return r;
    }
    if registry::global(&engines.config).get(&id).is_none() {
        return ShellError::NotFound.response();
    }
    // `link`：设备是否已认证 + 服务端是否要求设备 token（明网信任边界，设计 §3.2）。
    let token_required = !engines.config.echomuse.token.trim().is_empty();
    let body = super::hub_live(&id)
        .filter(|(c, _)| !c.is_closed())
        .map(|(_, live)| live.payload_with_link(&id, token_required))
        .unwrap_or_else(|| runtime::offline_payload(&id));
    admin::json(StatusCode::OK, body)
}

pub(super) fn volume_command(body: &Value) -> Result<String, &'static str> {
    if !body.as_object().is_some_and(|o| o.len() == 1) {
        return Err("body must contain only level");
    }
    let level = runtime::volume_level(body).ok_or("level must be an integer in 0..127")?;
    Ok(json!({"type":"volume_set", "level":level}).to_string())
}

async fn set_volume(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
    body: axum::Json<Value>,
) -> Response {
    if let Some(r) = admin::gate(&engines, &headers, &query) {
        return r;
    }
    let reg = registry::global(&engines.config);
    if let Err(e) = shell::approved_emos(reg, &id) {
        return e.response();
    }
    if !reg
        .get(&id)
        .is_some_and(|r| r.capabilities.iter().any(|c| c == "speaker"))
    {
        return ShellError::MissingSpeaker.response();
    }
    let text = match volume_command(&body.0) {
        Ok(text) => text,
        Err(error) => return bad_request(error),
    };
    let Some((ctl, live)) = super::hub_live(&id).filter(|(c, _)| !c.is_closed()) else {
        return ShellError::Offline.response();
    };
    if !live.authenticated {
        return ShellError::Unauthorized.response();
    }
    if ctl.send(ControlOut::Json(text)).is_err() {
        return ShellError::Offline.response();
    }
    admin::json(
        StatusCode::ACCEPTED,
        json!({"queued":true, "applied":false, "level":body.0["level"]}),
    )
}

async fn get_info(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if let Err(r) = system_gate(&engines, &headers, &query, &id) {
        return r;
    }
    match shell::run(&id, INFO).await {
        Ok(output) => admin::json(
            StatusCode::OK,
            json!({"device_id":id,"output":output,"exit_code":0}),
        ),
        Err(e) => e.response(),
    }
}

async fn get_logs(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if let Err(r) = system_gate(&engines, &headers, &query, &id) {
        return r;
    }
    let source = query
        .get("source")
        .map(String::as_str)
        .unwrap_or("messages");
    let Some(command) = log_command(source) else {
        return bad_request(
            "source must be server / messages / messages_last / last_kmsg_prev / supervisor / net",
        );
    };
    match shell::run(&id, command).await {
        Ok(output) => admin::json(
            StatusCode::OK,
            json!({"device_id":id,"source":source,"output":output,"exit_code":0}),
        ),
        Err(e) => e.response(),
    }
}

pub(super) fn confirmed(body: &Value) -> bool {
    body.as_object()
        .is_some_and(|o| o.len() == 1 && o.get("confirm") == Some(&Value::Bool(true)))
}

async fn reboot(
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
    body: axum::Json<Value>,
) -> Response {
    if let Err(r) = system_gate(&engines, &headers, &query, &id) {
        return r;
    }
    if !confirmed(&body.0) {
        return bad_request("reboot requires exactly {\"confirm\":true}");
    }
    match shell::run(&id, REBOOT).await {
        Ok(_) => admin::json(
            StatusCode::ACCEPTED,
            json!({"device_id":id,"dispatched":true,"rebooted":false}),
        ),
        Err(e) => e.response(),
    }
}

fn bad_request(error: &str) -> Response {
    admin::json(
        StatusCode::BAD_REQUEST,
        json!({"error":error,"code":"bad_request"}),
    )
}

#[cfg(test)]
#[path = "system_tests.rs"]
mod tests;
