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

pub(super) fn log_command(source: &str) -> Option<&'static str> {
    match source {
        "messages" => Some("tail -c 49152 /run/messages"),
        "messages_last" => Some("tail -c 49152 /data/emos/messages.last"),
        "last_kmsg_prev" => Some("tail -c 49152 /data/emos/last_kmsg.prev"),
        "supervisor" => Some("tail -c 49152 /data/local/etc/echomuse/supervisor.log"),
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

/// All system endpoints are opt-in: an empty default admin token must not make
/// root operations accessible to arbitrary LAN clients, even with device auth.
fn system_gate(
    engines: &Engines,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
    id: &str,
) -> Result<(), Response> {
    if let Some(r) = admin::gate(engines, headers, query) {
        return Err(r);
    }
    if engines.config.server.expected_token.is_empty() || engines.config.echomuse.token.is_empty() {
        return Err(ShellError::MissingToken.response());
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
    let body = super::hub_live(&id)
        .filter(|(c, _)| !c.is_closed())
        .map(|(_, live)| live.payload(&id))
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
            "source must be messages / messages_last / last_kmsg_prev / supervisor",
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
