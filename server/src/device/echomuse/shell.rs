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

use base64::Engine;
use sha2::{Digest, Sha256};

use super::{registry::Registry, runtime::LiveControl, ControlOut};
use crate::runtime::Engines;

pub(super) const MAX_OUTPUT: usize = 64 * 1024;
const OP_TIMEOUT: Duration = Duration::from_secs(15);

/// 文件传输整体上限：1.8MB 模型在局域网实测 2–5s，120s 覆盖 5–7% 丢包链路
/// （上游同值）。超时不是"再等等"，而是**结果未知**——调用方不得当成已装。
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(120);
/// heredoc 定界符：base64 字母表不含 `_`，正文不可能撞上，故可安全独占一行。
const B64_DELIM: &str = "__END_B64_42__";
const PROBE_MARKER: &str = "__EM_PROBE__";
/// 单帧文本块大小。设备 shell 是非 pty 的普通管道，分块只影响往返次数：
/// 逐行（76B）发 1.8MB 要三万多帧，32KB 一块只需约 75 帧。
const WRITE_CHUNK: usize = 32 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ShellError {
    NotFound,
    Unapproved,
    Unsupported,
    MissingSpeaker,
    Offline,
    Busy,
    PeerUnavailable,
    Unauthorized,
    Unsolicited,
    Timeout,
    Transport,
    OutputLimit,
    InvalidMarker,
    CommandFailed(u16),
    /// 设备上连 `busybox base64 -d` / `python` 都没有：一个字节都没发出去。
    NoDecoder,
    /// 设备上没有 `busybox sha256sum`：拒绝发无校验的字节（坏文件只在加载时才
    /// 炸，且报错不指向文件本身）。
    NoHashTool,
    /// 设备侧校验和与发送端不一致：`.part` 已被删除，目标路径保持原样。
    VerifyFailed,
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
            Self::NoDecoder => (
                502,
                "no_base64_decoder",
                "device has no base64 decoder — no data was sent",
            ),
            Self::NoHashTool => (
                502,
                "no_hash_tool",
                "device has no sha256 tool — refusing to send unverified bytes",
            ),
            Self::VerifyFailed => (
                502,
                "verify_failed",
                "device checksum did not match — target file left untouched",
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
        &cfg.echomuse.token,
    )
}

/// 设备 shell 通道准入（产品决策：**局域网默认可管理**）：`echomuse.token` 为空（默认）=
/// 信任局域网设备、不做设备鉴权；非空 = 设备必须带正确 `X-Em-Token`。
/// （`server.expected_token` 只管管理面鉴权，不再参与设备通道准入——旧口径要求双 token
/// 都非空才准入并 503，已按局域网默认语义移除。）
fn admit(
    ws: WebSocketUpgrade,
    headers: &HeaderMap,
    id: &str,
    peer: IpAddr,
    reg: &Registry,
    device_token: &str,
) -> Response {
    if !device_token.is_empty() && !device_auth(headers, device_token) {
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

/// 一次成功传输的凭证：只有设备回算的 sha256 与发送端一致才算数（退出码和
/// 字节数都不足以证明内容正确——上游实测 shell 传输产过"长度对、内容错"的文件）。
pub(super) struct Transfer {
    pub bytes: usize,
    pub sha256: String,
}

/// `data` 的 sha256 十六进制摘要（传输校验与 E2E 报告共用同一口径）。
pub(super) fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 把一个文件写到设备的 `dest` 上（**只接受服务端常量路径**：`&'static str`
/// 让"不把请求参数拼进 shell 命令"成为类型约束）。
///
/// 协议与上游 `controller/em_shell.py::stream_file` 同构：
/// 1. 一次往返探测 `busybox base64 -d` 与 `busybox sha256sum`，缺任一都不发字节；
/// 2. base64 heredoc 解码直落 `{dest}.part`；
/// 3. 设备侧算 sha256 与发送端比对，一致才 `mv` 覆盖目标。
///
/// 为什么不是 curl/wget：设备固件不保证带，而 busybox 是 emOS 既有工具（`/system`
/// 的 df/md5sum 都走它）；为什么要 `.part` + 校验：模型/so 文件损坏只在加载那一刻
/// 炸，报错不指向文件本身，中途断链绝不能污染目标路径。
pub(super) async fn stream_file(
    id: &str,
    data: &[u8],
    dest: &'static str,
    mode: u32,
) -> Result<Transfer, ShellError> {
    let (ctl, live) = super::hub_live(id).ok_or(ShellError::Offline)?;
    let (op, rx) = Operation::reserve(id, ctl, &live, TRANSFER_TIMEOUT)?;
    let digest = sha256_hex(data);
    let work = async {
        op.ctl
            .send(ControlOut::Json(r#"{"type":"shell_open"}"#.into()))
            .map_err(|_| ShellError::Offline)?;
        let mut socket = rx.await.map_err(|_| ShellError::Transport)?;
        if !live_matches(id, &op.ctl) {
            return Err(ShellError::Offline);
        }
        let result = push(&mut socket, data, dest, mode, &digest, op.deadline).await;
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

async fn push(
    socket: &mut WebSocket,
    data: &[u8],
    dest: &str,
    mode: u32,
    digest: &str,
    deadline: Instant,
) -> Result<Transfer, ShellError> {
    // 1) 探测：同一次往返问完解码器与哈希工具，先发完再发字节。
    //    末尾的 `\n` 不是排版：设备 shell 是**非 pty 的管道 stdin**，sh 只在读到
    //    换行时才执行一条完整命令行。少这个换行，命令会一直攥在行缓冲里不跑，
    //    调用方只能等到 deadline（实测挂满 120s、设备侧 shell 会话空转，一个字节
    //    都没发出去）。同理见下面第 4) 步的校验命令。
    let probe = format!(
        "if echo dGVzdA== | busybox base64 -d >/dev/null 2>&1; then echo DECODER:ok; \
         else echo DECODER:none; fi; \
         if echo x | busybox sha256sum >/dev/null 2>&1; then echo HASH:ok; \
         else echo HASH:none; fi; echo {PROBE_MARKER}\n"
    );
    send_text(socket, probe).await?;
    let probe_out = read_until(socket, PROBE_MARKER, deadline).await?;
    if !probe_out.contains("DECODER:ok") {
        return Err(ShellError::NoDecoder);
    }
    if !probe_out.contains("HASH:ok") {
        return Err(ShellError::NoHashTool);
    }

    // 2) heredoc 解码直落 `.part`（`>` 截断上次残留，目标路径保持原样）。
    let part = format!("{dest}.part");
    send_text(
        socket,
        format!(
            "busybox base64 -d << '{B64_DELIM}' > {part} && chmod {mode:o} {part} && echo TRANSFER_OK\n"
        ),
    )
    .await?;

    // 3) 正文按块发：块边界切在 base64 字母中间不影响流式解码；末尾补换行让
    //    定界符独占一行（heredoc 结束条件就是"整行等于定界符"）。
    let encoded = base64::engine::general_purpose::STANDARD.encode(data);
    for chunk in encoded.as_bytes().chunks(WRITE_CHUNK) {
        send_text(socket, String::from_utf8_lossy(chunk).into_owned()).await?;
    }
    send_text(socket, format!("\n{B64_DELIM}\n")).await?;
    read_until(socket, "TRANSFER_OK", deadline).await?;

    // 4) 校验 + 原子替换：设备算出来的摘要才是唯一判据，不一致就删掉 `.part`。
    send_text(
        socket,
        format!(
            "GOT=$(busybox sha256sum {part} 2>/dev/null); GOT=${{GOT%% *}}; \
             if [ \"$GOT\" = \"{digest}\" ]; then mv {part} {dest} && chmod {mode:o} {dest} \
             && echo VERIFY_OK; else rm -f {part}; echo \"VERIFY_BAD:$GOT\"; fi\n"
        ),
    )
    .await?;
    let verify_out = read_until(socket, "VERIFY_", deadline).await?;
    if !verify_out.contains("VERIFY_OK") {
        return Err(ShellError::VerifyFailed);
    }
    Ok(Transfer {
        bytes: data.len(),
        sha256: digest.to_string(),
    })
}

async fn send_text(socket: &mut WebSocket, text: String) -> Result<(), ShellError> {
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(|_| ShellError::Transport)
}

/// 读到 `needle` 出现为止（跨帧聚合，容忍 Binary/Text 混排与 Ping）。
async fn read_until(
    socket: &mut WebSocket,
    needle: &str,
    deadline: Instant,
) -> Result<String, ShellError> {
    let mut buffer = String::new();
    loop {
        let message = match timeout_at(deadline, socket.next()).await {
            Err(_) => {
                // 超时几乎总是"设备没执行"而不是"链路慢"：把已经收到的输出打出来，
                // 否则设备侧的 `sh: syntax error` 之类只有插数据线才看得到。
                tracing::warn!(
                    "[echomuse] shell 等待 `{needle}` 超时：已收 {} 字节，尾部输出={:?}",
                    buffer.len(),
                    buffer
                        .chars()
                        .rev()
                        .take(400)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect::<String>()
                );
                return Err(ShellError::Timeout);
            }
            Ok(None) | Ok(Some(Err(_))) => return Err(ShellError::Transport),
            Ok(Some(Ok(message))) => message,
        };
        match message {
            Message::Text(text) => buffer.push_str(&text),
            Message::Binary(bytes) => buffer.push_str(&String::from_utf8_lossy(&bytes)),
            Message::Ping(payload) => {
                socket
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|_| ShellError::Transport)?;
            }
            Message::Pong(_) => {}
            Message::Close(_) => return Err(ShellError::Transport),
        }
        if buffer.contains(needle) {
            return Ok(buffer);
        }
        // 固定命令的输出只有几行；超限说明设备回了非预期内容，立刻收手。
        if buffer.len() > MAX_OUTPUT * 4 {
            return Err(ShellError::OutputLimit);
        }
    }
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
