//! Private, fixed-operation shell bridge. Not an interactive/admin shell API.
//! EchoMuse main 858987c/controller/em_shell.py: shell_open without pty, device
//! dials /shell/{device_id}; stdin Text, stdout Binary or Text. Unlike upstream,
//! a timeout/EOF/nonzero exit is an explicit failure, never an empty success.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use tokio::sync::{mpsc::UnboundedSender, oneshot};
use tokio::time::{timeout_at, Instant};

use super::{registry::Registry, runtime::LiveControl, ControlOut};
use crate::runtime::Engines;

pub(super) const MAX_OUTPUT: usize = 64 * 1024;
const OP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ShellError {
    NotFound,
    Unapproved,
    Unsupported,
    MissingSpeaker,
    Offline,
    Busy,
    MissingToken,
    PeerUnavailable,
    Unauthorized,
    Unsolicited,
    Timeout,
    Transport,
    OutputLimit,
    InvalidMarker,
    CommandFailed(u16),
}

impl ShellError {
    pub(super) fn response(&self) -> Response {
        let (status, code, error) = match self {
            Self::NotFound => (404, "not_found", "device not found"),
            Self::Unapproved => (403, "unapproved", "device not approved"),
            Self::Unsupported => (422, "unsupported_os", "system management requires emOS"),
            Self::MissingSpeaker => (
                422,
                "unsupported_capability",
                "device does not declare speaker capability",
            ),
            Self::Offline => (409, "offline", "device control is offline or replaced"),
            Self::Busy => (409, "busy", "another system operation is active"),
            Self::MissingToken => (
                503,
                "token_required",
                "configure both server.expected_token and echomuse.token",
            ),
            Self::PeerUnavailable => (
                503,
                "peer_unavailable",
                "authenticated control peer unavailable",
            ),
            Self::Unauthorized => (401, "unauthorized", "invalid device token or control peer"),
            Self::Unsolicited => (409, "no_pending_shell", "no matching pending shell request"),
            Self::Timeout => (
                504,
                "shell_timeout",
                "shell deadline exceeded; outcome unknown",
            ),
            Self::Transport => (
                502,
                "shell_transport",
                "shell disconnected or transport failed; outcome unknown",
            ),
            Self::OutputLimit => (502, "output_limit", "shell output exceeded limit"),
            Self::InvalidMarker => (502, "invalid_marker", "invalid shell completion marker"),
            Self::CommandFailed(_) => (
                502,
                "command_failed",
                "device command returned nonzero status",
            ),
        };
        let mut body = serde_json::json!({"error": error, "code": code});
        if let Self::CommandFailed(exit) = self {
            body["exit_code"] = (*exit).into();
        }
        super::admin::json(StatusCode::from_u16(status).expect("constant status"), body)
    }
}

pub(super) fn approved_emos(reg: &Registry, id: &str) -> Result<(), ShellError> {
    let record = reg.get(id).ok_or(ShellError::NotFound)?;
    if !record.approved {
        return Err(ShellError::Unapproved);
    }
    if record.base_os.as_deref() != Some("emos") {
        return Err(ShellError::Unsupported);
    }
    Ok(())
}

struct Pending {
    owner: uuid::Uuid,
    ctl: UnboundedSender<ControlOut>,
    peer: IpAddr,
    deadline: Instant,
    socket: Option<oneshot::Sender<WebSocket>>,
}

type PendingMap = Mutex<HashMap<String, Pending>>;
fn pending() -> &'static PendingMap {
    static PENDING: OnceLock<PendingMap> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The HTTP operation alone owns this slot. Any cancellation/timeout/error/drop
/// clears only its own entry and shell_close goes ONLY to its captured channel.
struct Operation {
    id: String,
    owner: uuid::Uuid,
    ctl: UnboundedSender<ControlOut>,
    deadline: Instant,
}

impl Drop for Operation {
    fn drop(&mut self) {
        let mut map = pending().lock().expect("echomuse shell lock");
        if map.get(&self.id).is_some_and(|p| p.owner == self.owner) {
            // Enqueue close BEFORE releasing the slot. Otherwise a fresh owner
            // could enqueue shell_open between removal and this old close.
            let _ = self
                .ctl
                .send(ControlOut::Json(r#"{"type":"shell_close"}"#.into()));
            map.remove(&self.id);
        }
    }
}

impl Operation {
    fn reserve(
        id: &str,
        ctl: UnboundedSender<ControlOut>,
        live: &LiveControl,
        timeout: Duration,
    ) -> Result<(Self, oneshot::Receiver<WebSocket>), ShellError> {
        if ctl.is_closed() {
            return Err(ShellError::Offline);
        }
        if !live.authenticated {
            return Err(ShellError::Unauthorized);
        }
        let peer = live.peer.ok_or(ShellError::PeerUnavailable)?;
        let owner = uuid::Uuid::new_v4();
        let deadline = Instant::now() + timeout;
        let (tx, rx) = oneshot::channel();
        let mut map = pending().lock().expect("echomuse shell lock");
        // No stealing expired slots: the owner may still hold a connected socket.
        // Its own deadline/drop releases it, so a waiter cannot close its shell.
        if map.contains_key(id) {
            return Err(ShellError::Busy);
        }
        map.insert(
            id.into(),
            Pending {
                owner,
                ctl: ctl.clone(),
                peer,
                deadline,
                socket: Some(tx),
            },
        );
        Ok((
            Self {
                id: id.into(),
                owner,
                ctl,
                deadline,
            },
            rx,
        ))
    }
}

fn live_matches(id: &str, ctl: &UnboundedSender<ControlOut>) -> bool {
    super::hub_live(id)
        .is_some_and(|(current, _)| current.same_channel(ctl) && !current.is_closed())
}

/// Claim happens before HTTP upgrade, so unsolicited/duplicate sockets get a
/// real rejection. Sender handoff after upgrade fails closed if owner expired.
fn claim(id: &str, peer: IpAddr) -> Result<oneshot::Sender<WebSocket>, ShellError> {
    let mut map = pending().lock().expect("echomuse shell lock");
    let p = map.get_mut(id).ok_or(ShellError::Unsolicited)?;
    if Instant::now() >= p.deadline {
        return Err(ShellError::Unsolicited);
    }
    if p.peer != peer {
        return Err(ShellError::Unauthorized);
    }
    if !live_matches(id, &p.ctl) {
        return Err(ShellError::Offline);
    }
    p.socket.take().ok_or(ShellError::Unsolicited)
}

pub(super) fn device_auth(headers: &HeaderMap, expected: &str) -> bool {
    !expected.is_empty()
        && headers.get("X-Em-Token").and_then(|v| v.to_str().ok()) == Some(expected)
}

pub(super) async fn upgrade(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Path(id): Path<String>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    let cfg = &engines.config;
    if !cfg.echomuse.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    admit(
        ws,
        &headers,
        &id,
        peer.ip(),
        super::registry::global(cfg),
        (&cfg.server.expected_token, &cfg.echomuse.token),
    )
}

fn admit(
    ws: WebSocketUpgrade,
    headers: &HeaderMap,
    id: &str,
    peer: IpAddr,
    reg: &Registry,
    tokens: (&str, &str),
) -> Response {
    if tokens.0.is_empty() || tokens.1.is_empty() {
        return ShellError::MissingToken.response();
    }
    if !device_auth(headers, tokens.1) {
        return ShellError::Unauthorized.response();
    }
    if let Err(err) = approved_emos(reg, id) {
        return err.response();
    }
    match claim(id, peer) {
        Ok(tx) => ws
            .max_message_size(MAX_OUTPUT)
            .max_frame_size(MAX_OUTPUT)
            .on_upgrade(move |socket| async move {
                let _ = tx.send(socket);
            }),
        Err(err) => err.response(),
    }
}

/// Text command is built exclusively from server-owned static allowlisted scripts.
fn wrapped_command(command: &'static str, marker: &str) -> String {
    format!("( {command} ) 2>&1; __em_rc=$?; printf '\\n{marker}%s\\n' \"$__em_rc\"\n")
}

pub(super) struct Output {
    marker: Vec<u8>,
    buffer: Vec<u8>,
}

impl Output {
    pub(super) fn new(marker: &str) -> Self {
        Self {
            marker: format!("\n{marker}").into_bytes(),
            buffer: Vec::new(),
        }
    }
    pub(super) fn push(&mut self, bytes: &[u8]) -> Result<Option<String>, ShellError> {
        if self.buffer.len().saturating_add(bytes.len()) > MAX_OUTPUT {
            return Err(ShellError::OutputLimit);
        }
        self.buffer.extend_from_slice(bytes);
        let Some(pos) = self
            .buffer
            .windows(self.marker.len())
            .position(|w| w == self.marker)
        else {
            return Ok(None);
        };
        let tail = &self.buffer[pos + self.marker.len()..];
        let Some(end) = tail.iter().position(|b| *b == b'\n') else {
            if tail.len() > 3 {
                return Err(ShellError::InvalidMarker);
            }
            return Ok(None);
        };
        let exit = std::str::from_utf8(&tail[..end])
            .ok()
            .filter(|v| !v.is_empty() && v.bytes().all(|c| c.is_ascii_digit()))
            .and_then(|v| v.parse::<u16>().ok())
            .filter(|v| *v <= 255)
            .ok_or(ShellError::InvalidMarker)?;
        if exit != 0 {
            return Err(ShellError::CommandFailed(exit));
        }
        Ok(Some(
            String::from_utf8_lossy(&self.buffer[..pos]).into_owned(),
        ))
    }
}

async fn exchange(
    socket: &mut WebSocket,
    command: &'static str,
    marker: &str,
) -> Result<String, ShellError> {
    socket
        .send(Message::Text(wrapped_command(command, marker).into()))
        .await
        .map_err(|_| ShellError::Transport)?;
    let mut output = Output::new(marker);
    loop {
        let bytes = match socket.next().await {
            Some(Ok(Message::Binary(b))) => b.to_vec(),
            Some(Ok(Message::Text(t))) => t.as_bytes().to_vec(),
            Some(Ok(Message::Ping(p))) => {
                socket
                    .send(Message::Pong(p))
                    .await
                    .map_err(|_| ShellError::Transport)?;
                continue;
            }
            Some(Ok(Message::Pong(_))) => continue,
            _ => return Err(ShellError::Transport),
        };
        if let Some(text) = output.push(&bytes)? {
            return Ok(text);
        }
    }
}

pub(super) async fn run(id: &str, command: &'static str) -> Result<String, ShellError> {
    run_timed(id, command, OP_TIMEOUT).await
}

async fn run_timed(
    id: &str,
    command: &'static str,
    timeout: Duration,
) -> Result<String, ShellError> {
    let (ctl, live) = super::hub_live(id).ok_or(ShellError::Offline)?;
    let (op, rx) = Operation::reserve(id, ctl, &live, timeout)?;
    let marker = format!("__EM_DONE_{}__:", op.owner.simple());
    let work = async {
        op.ctl
            .send(ControlOut::Json(r#"{"type":"shell_open"}"#.into()))
            .map_err(|_| ShellError::Offline)?;
        let mut socket = rx.await.map_err(|_| ShellError::Transport)?;
        if !live_matches(id, &op.ctl) {
            return Err(ShellError::Offline);
        }
        let result = exchange(&mut socket, command, &marker).await;
        // Drop closes the transport even if graceful close cannot flush.
        let _ = socket.send(Message::Close(None)).await;
        if !live_matches(id, &op.ctl) {
            return Err(ShellError::Offline);
        }
        result
    };
    tokio::select! {
        _ = op.ctl.closed() => Err(ShellError::Offline),
        result = timeout_at(op.deadline, work) => result.unwrap_or(Err(ShellError::Timeout)),
    }
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
