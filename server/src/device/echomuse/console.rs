//! EchoMuse 网页控制台：设备 `sh` 的长驻 WebSocket 通道（设计 §5 全章）。
//!
//! ## 边界
//!
//! - 端点 `GET /api/echomuse/devices/{id}/console`（WS upgrade）。**upgrade 前**用
//!   [`super::system::system_gate`] 完成全部准入（enabled=false → 404、未批准/非 emOS/
//!   离线/未认证/peer 缺失 → 对应 HTTP 码 + `{"error","code"}`）；**upgrade 之后**才占槽，
//!   槽被占则首帧 `error{code:"busy"}` 再关——浏览器 `WebSocket` API 在握手被拒时拿不到
//!   HTTP 状态码（`onerror` 无信息、`onclose` 恒 1006），而 `busy` 是唯一必须让用户看到
//!   具体 code 的失败（设计 §5.6）。
//! - 链路：`shell_open{pty:true}`（**真终端**，官方 `em_routes_shell.py` 同形）→ 设备
//!   回拨 `/shell/{id}` → `shell::claim` 认领 → 双向泵（浏览器 JSON 文本帧 ↔ 设备原始
//!   Binary）。设备若在回拨后先发 `shell_meta{pty}`，原样转给浏览器（控制帧不进画面）。
//! - 与一次性操作（`/system/*`、`/oww/install-vad`、文件传输）**共用同一把 per-device 槽**：
//!   本模块不复制槽逻辑，直接把 [`Operation`] 当长驻租约持有到会话结束，收尾走同一段
//!   `Drop`（先 enqueue `shell_close` 到**捕获的** control 通道，再按 owner 删槽）。
//! - 一次性操作仍走 pipe（`shell::SHELL_OPEN`，不带 `pty`）：那边要干净管道做 md5 校验，
//!   pty 的回显会污染输出；控制台要的正好相反，没有提示符就不是终端。
//! - pty 模式下 `sh` 自己回显、自己打提示符，所以**前端不再伪造提示符**、服务端也不做
//!   行编辑；老固件忽略 `pty` 字段仍开 pipe（无提示符无回显），此时前端只标「已连接」，
//!   功能（写 stdin、读输出）不受影响。

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::Instant;

use super::console_frames as frames;
use super::console_frames::{Budget, CloseReason, ConsoleFrame, ErrorCode, TokenBucket, Utf8Carry};
use super::shell::{self, Operation, ShellError};
use super::{hub_live, ControlOut};
use crate::runtime::Engines;

/// 发 `shell_open` 的唯一出处：**带 `pty:true`**，即官方 dashboard 的 Shell 终端模式
/// （`em_routes_shell.py` 的 `{"type": "shell_open", "pty": True}`）。
///
/// 与 `shell::` 里一次性操作用的 `{"type":"shell_open"}` **刻意分成两个常量**：
/// 系统信息/日志/文件传输要的是干净管道（pty 会把设备 stty 的回显混进输出，md5 校验
/// 与哨兵匹配都会失真）；控制台要的正好相反——没有提示符、没有回显、没有行编辑，
/// 就不是终端，用户看到的是"一个能打字的日志框"。
///
/// 老固件忽略该字段仍开 pipe，此时设备不会给提示符；前端按 `ready.mode` 决定提示符
/// 与回显口径，功能（写 stdin、读输出）不受影响。
const SHELL_OPEN: &str = r#"{"type":"shell_open","pty":true}"#;

const DETAIL_CLIENT_CLOSED: &str = "浏览器关闭连接";
const DETAIL_DEVICE_OFFLINE: &str = "设备控制通道已断开";
const DETAIL_SUPERSEDED: &str = "设备已重连，旧 control 会话已失效";
const DETAIL_SHELL_LOST: &str = "设备关闭了 shell 通道（sh 退出或重启）";
const DETAIL_IDLE: &str = "会话因 600 秒无输入断开";
const DETAIL_SESSION_LIMIT: &str = "会话已达 7200 秒上限";
const DETAIL_DIAL_TIMEOUT: &str = "设备未在 15 秒内回拨 shell 通道";
const DETAIL_DIAL_CLOSED: &str = "设备回拨通道已关闭（占槽在回拨前被撤销）";

/// 挂载控制台路由（由 [`super::admin::admin_routes`] merge，与系统/资产路由并列）。
pub(super) fn routes() -> axum::Router<Arc<Engines>> {
    use axum::routing::get;
    axum::Router::new().route("/api/echomuse/devices/{id}/console", get(console_ws))
}

/// `GET /api/echomuse/devices/{id}/console`：准入 → upgrade → 会话。
///
/// 准入（`system_gate`）必须在 `on_upgrade` **之前**跑完：此时还能返回真 HTTP 状态码。
async fn console_ws(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(id): Path<String>,
    State(engines): State<Arc<Engines>>,
) -> Response {
    if let Err(r) = super::system::system_gate(&engines, &headers, &query, &id) {
        return r;
    }
    // 设备未设 token = 同网段任何人可连入 root 命令通道，前端要顶一条琥珀警告。
    let insecure = engines.config.echomuse.token.trim().is_empty();
    ws.max_message_size(frames::INBOUND_MAX)
        .max_frame_size(frames::INBOUND_MAX)
        .on_upgrade(move |socket| async move { serve(socket, id, insecure).await })
}

/// 一个控制台会话的全部可变状态。**清尾只有 [`ConsoleSession::teardown`] 一个出口**。
pub(super) struct ConsoleSession {
    /// 设备号（帧、审计、`live_matches` 复核都用它）。
    device_id: String,
    /// 设备 TCP peer（准入已保证存在；`Option` 只为审计文案不假填）。
    peer: Option<IpAddr>,
    /// 设备 control 链路是否已认证（准入已保证为 true；审计行按实值记录，不假填）。
    authenticated: bool,
    /// `[echomuse].token` 为空 → 明文 + 无设备鉴权。
    insecure: bool,
    /// 占槽租约：`take()` 后 drop 即"先 enqueue `shell_close` 再按 owner 删槽"。
    lease: Option<Operation>,
    started: Instant,
    /// 最近一次**输入**时刻（输出流动不重置空闲计时）。
    last_input: Instant,
    inputs: u32,
    rate: TokenBucket,
    carry: Utf8Carry,
    budget: Budget,
    closed: bool,
}

impl ConsoleSession {
    fn new(
        device_id: String,
        peer: Option<IpAddr>,
        authenticated: bool,
        insecure: bool,
        lease: Operation,
    ) -> Self {
        let now = Instant::now();
        Self {
            device_id,
            peer,
            authenticated,
            insecure,
            lease: Some(lease),
            started: now,
            last_input: now,
            inputs: 0,
            rate: TokenBucket::new(frames::INPUT_PER_SEC),
            carry: Utf8Carry::new(),
            budget: Budget::new(frames::SESSION_BYTES_CAP),
            closed: false,
        }
    }

    /// 占槽时刻捕获的 control 出站口（**绝不用 `hub_ctl(id)` 现取**：那是重拨后的新会话）。
    fn control(&self) -> &UnboundedSender<ControlOut> {
        &self.lease.as_ref().expect("会话生命周期内租约必在").ctl
    }

    fn peer_label(&self) -> String {
        self.peer
            .map(|p| p.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    }

    fn secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    // ---- 审计（三行，全部 `tracing::info!`；含命令全文，按敏感数据对待） ----

    fn audit_open(&self) {
        tracing::info!(
            "[echomuse] 控制台会话开启 device={} peer={} auth={} insecure={}",
            self.device_id,
            self.peer_label(),
            self.authenticated,
            self.insecure
        );
    }

    fn audit_input(&self, line: &str) {
        tracing::info!(
            "[echomuse] 控制台命令 device={} peer={} cmd={}",
            self.device_id,
            self.peer_label(),
            line
        );
    }

    fn audit_close(&self, reason: CloseReason) {
        tracing::info!(
            "[echomuse] 控制台会话结束 device={} peer={} reason={} secs={} inputs={} bytes_out={}",
            self.device_id,
            self.peer_label(),
            reason.code(),
            self.secs(),
            self.inputs,
            self.budget.used()
        );
    }

    // ---- 出站 ----

    async fn send(&self, browser: &mut WebSocket, frame: &ConsoleFrame) {
        let _ = browser
            .send(Message::Text(frames::encode(frame).into()))
            .await;
    }

    async fn send_ready(&self, browser: &mut WebSocket) {
        let frame = ConsoleFrame::Ready {
            device_id: self.device_id.clone(),
            mode: frames::MODE_PTY,
            insecure: self.insecure,
            notice: frames::COPY_READY_NOTICE.to_string(),
        };
        self.send(browser, &frame).await;
    }

    async fn reject(&self, browser: &mut WebSocket, code: ErrorCode) {
        let frame = ConsoleFrame::Error {
            code,
            error: code.message(""),
        };
        self.send(browser, &frame).await;
    }

    // ---- 入站：浏览器 → 设备 ----

    /// 处理一条浏览器文本帧。被拒（限流/超长）时**一个字节都不写设备**：
    /// 决策由 `frames::plan_line`/`plan_ctrl_c` 给出，`Reject` 不携带字节。
    async fn on_browser_text(
        &mut self,
        text: &str,
        browser: &mut WebSocket,
        device: &mut WebSocket,
    ) {
        match frames::parse_incoming(text) {
            frames::Incoming::Ping => self.send(browser, &ConsoleFrame::Pong).await,
            frames::Incoming::CtrlC => match frames::plan_ctrl_c(&mut self.rate, Instant::now()) {
                frames::InputAction::Write(bytes) => {
                    self.write_device(device, bytes, "^C").await;
                }
                frames::InputAction::Reject(code) => self.reject(browser, code).await,
            },
            frames::Incoming::Input(data) => {
                match frames::plan_line(&mut self.rate, Instant::now(), &data) {
                    frames::InputAction::Write(bytes) => {
                        self.write_device(device, bytes, &data).await;
                    }
                    frames::InputAction::Reject(code) => self.reject(browser, code).await,
                }
            }
            frames::Incoming::Unknown => {}
        }
    }

    /// 真正写设备 stdin（pipe 模式 raw 字节）。计时与审计只在"写成功路径"上发生。
    async fn write_device(&mut self, device: &mut WebSocket, bytes: Vec<u8>, audit: &str) {
        self.last_input = Instant::now();
        self.inputs = self.inputs.saturating_add(1);
        self.audit_input(audit);
        // 写失败不在这里收尾：设备读端下一轮就会给出 EOF/Err，由统一出口处理。
        let _ = device.send(Message::Binary(bytes.into())).await;
    }

    // ---- 出站：设备 → 浏览器 ----

    /// 一块设备输出：按 8 KiB 再切分 → UTF-8 对齐 → 预算 → `output` 帧。
    async fn on_device_bytes(&mut self, bytes: &[u8], browser: &mut WebSocket) {
        for chunk in bytes.chunks(frames::FRAME_MAX) {
            let text = self.carry.push(chunk);
            if text.is_empty() {
                continue;
            }
            match self.budget.admit(text.len()) {
                frames::Admit::Forward => self.send(browser, &ConsoleFrame::Output { text }).await,
                frames::Admit::Cap => {
                    let frame = ConsoleFrame::Notice {
                        code: frames::NoticeCode::OutputCapped,
                        text: frames::COPY_CAPPED.to_string(),
                    };
                    self.send(browser, &frame).await;
                }
                frames::Admit::Drop => {}
            }
        }
    }

    /// 每轮复核 control 会话身份：先判通道关闭（`device-offline`），再判被替换
    /// （`superseded`）。两者必须分开——前者是掉线，后者是设备重拨后的**新**会话。
    fn supervise(&self) -> Option<(CloseReason, &'static str)> {
        let ctl = self.control();
        if ctl.is_closed() {
            return Some((CloseReason::DeviceOffline, DETAIL_DEVICE_OFFLINE));
        }
        if !shell::live_matches(&self.device_id, ctl) {
            return Some((CloseReason::Superseded, DETAIL_SUPERSEDED));
        }
        None
    }

    // ---- 唯一收尾出口（设计 §5.3 五步，顺序不可颠倒） ----

    async fn teardown(
        &mut self,
        reason: CloseReason,
        detail: &str,
        browser: &mut WebSocket,
        device: &mut Option<WebSocket>,
    ) {
        if self.closed {
            return;
        }
        // 1) 置 closed：此后到达的 input 一律丢弃，不再写设备。
        self.closed = true;

        // 2) 向浏览器发 closed 帧并关 WS（尽力而为，失败忽略）。浏览器先关时不发帧。
        if reason != CloseReason::ClientClosed {
            let frame = ConsoleFrame::Closed {
                reason,
                detail: detail.to_string(),
                inputs: self.inputs,
                bytes_out: self.budget.used(),
                secs: self.secs(),
            };
            self.send(browser, &frame).await;
            let _ = browser.send(Message::Close(None)).await;
        }

        // 3) 关设备 shell socket。设备侧 `ShellActive()` 可能还没清，所以紧接着仍要发
        //    `shell_close`（第 4 步），不能因为 socket 已关就省掉。
        if let Some(device) = device.as_mut() {
            let _ = device.send(Message::Close(None)).await;
        }

        // 残余不足一个字符的字节：会话已结束，按 lossy 丢弃（不留在缓冲里）。
        let _ = self.carry.flush();

        // 4) drop 租约 → 内部先向**捕获的旧 ctl** enqueue `shell_close`，再按 owner 删槽。
        if let Some(lease) = self.lease.take() {
            drop(lease);
        }

        // 5) 审计。
        self.audit_close(reason);
    }
}

impl Drop for ConsoleSession {
    /// 兜底：pump 被取消/panic 时 [`ConsoleSession::teardown`] 可能没跑完，但租约随字段
    /// 一起 drop，`Operation::Drop` 的 owner 校验保证"只清自己的槽"。这里补一条审计，
    /// 让"没走正常出口"在日志里可见（幂等：正常收尾已置 `closed`）。
    fn drop(&mut self) {
        if !self.closed {
            tracing::info!(
                "[echomuse] 控制台会话异常结束 device={} peer={} reason={} secs={} inputs={} bytes_out={}",
                self.device_id,
                self.peer_label(),
                CloseReason::Internal.code(),
                self.secs(),
                self.inputs,
                self.budget.used()
            );
        }
    }
}

/// 占槽失败/准入竞态时的失败帧 + 关连接（浏览器只能从这里拿到 code）。
async fn fail(browser: &mut WebSocket, code: ErrorCode, detail: &str) {
    let frame = ConsoleFrame::Error {
        code,
        error: code.message(detail),
    };
    let _ = browser
        .send(Message::Text(frames::encode(&frame).into()))
        .await;
    let _ = browser.send(Message::Close(None)).await;
}

/// 占槽错误 → 帧错误码（一次性 HTTP 路径继续用 `ShellError::response()`，互不影响）。
fn reserve_code(err: &ShellError) -> (ErrorCode, String) {
    match err {
        ShellError::Busy => (ErrorCode::Busy, String::new()),
        ShellError::Offline | ShellError::Unauthorized | ShellError::PeerUnavailable => {
            (ErrorCode::DeviceOffline, String::new())
        }
        other => (ErrorCode::Internal, format!("占槽失败：{other:?}")),
    }
}

/// 一次控制台会话的完整生命周期：占槽 → dial → pump → teardown。
///
/// `pub(super)` 是为了让外置测试用真实 loopback WS 直驱（不需要 `Engines`）。
pub(super) async fn serve(mut browser: WebSocket, device_id: String, insecure: bool) {
    // 准入刚过，这里只是补一次原子捕获（control 身份 + peer 元数据同锁读取）。
    let Some((ctl, live)) = hub_live(&device_id) else {
        fail(&mut browser, ErrorCode::DeviceOffline, "").await;
        return;
    };
    let (lease, mut dial) = match Operation::reserve(&device_id, ctl, &live, frames::DIAL_TIMEOUT) {
        Ok(pair) => pair,
        Err(err) => {
            let (code, detail) = reserve_code(&err);
            fail(&mut browser, code, &detail).await;
            return;
        }
    };
    let mut session =
        ConsoleSession::new(device_id, live.peer, live.authenticated, insecure, lease);
    session.audit_open();

    // 拿到槽才发 `shell_open`：失败方一个字节都不会写到设备上。
    if session
        .control()
        .send(ControlOut::Json(SHELL_OPEN.to_string()))
        .is_err()
    {
        let mut device: Option<WebSocket> = None;
        session
            .teardown(
                CloseReason::DeviceOffline,
                DETAIL_DEVICE_OFFLINE,
                &mut browser,
                &mut device,
            )
            .await;
        return;
    }

    let mut device: Option<WebSocket> = None;
    let dial_deadline = Instant::now() + frames::DIAL_TIMEOUT;
    loop {
        tokio::select! {
            // control 通道关闭 / 被新会话替换：设备不会回拨了，立刻收尾而不是干等 15 s。
            _ = session.control().closed() => {
                teardown(&mut session, &mut browser, &mut device, CloseReason::DeviceOffline, DETAIL_DEVICE_OFFLINE).await;
                return;
            }
            got = &mut dial => {
                match got {
                    Ok(socket) => {
                        device = Some(socket);
                        break;
                    }
                    Err(_) => {
                        teardown(&mut session, &mut browser, &mut device, CloseReason::Internal, DETAIL_DIAL_CLOSED).await;
                        return;
                    }
                }
            }
            // 回拨等待期间必须同时听浏览器读端：否则用户关了 tab，服务端还白占槽 15 s。
            message = browser.next() => {
                match message {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => {
                        teardown(&mut session, &mut browser, &mut device, CloseReason::ClientClosed, DETAIL_CLIENT_CLOSED).await;
                        return;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = browser.send(Message::Pong(payload)).await;
                    }
                    // ready 之前的 input/ctrl-c 一律丢弃（前端此时输入框置灰）。
                    _ => {}
                }
            }
            _ = tokio::time::sleep_until(dial_deadline) => {
                teardown(&mut session, &mut browser, &mut device, CloseReason::Internal, DETAIL_DIAL_TIMEOUT).await;
                return;
            }
        }
    }

    // 认领后复核：设备可能在这几毫秒里重拨，旧 socket 已不该被当成当前会话。
    if let Some((reason, detail)) = session.supervise() {
        teardown(&mut session, &mut browser, &mut device, reason, detail).await;
        return;
    }
    session.send_ready(&mut browser).await;

    let Some(device_socket) = device.as_mut() else {
        return;
    };
    let (reason, detail) = pump(&mut session, &mut browser, device_socket).await;
    teardown(&mut session, &mut browser, &mut device, reason, detail).await;
}

/// 收尾包装：把"会话 + 两侧 socket"一起传进去，保证四条路径都走同一个出口。
async fn teardown(
    session: &mut ConsoleSession,
    browser: &mut WebSocket,
    device: &mut Option<WebSocket>,
    reason: CloseReason,
    detail: &str,
) {
    session.teardown(reason, detail, browser, device).await;
}

/// 双向泵。返回收尾原因（由 [`ConsoleSession::teardown`] 统一执行五步）。
async fn pump(
    session: &mut ConsoleSession,
    browser: &mut WebSocket,
    device: &mut WebSocket,
) -> (CloseReason, &'static str) {
    loop {
        // 每轮复核：绝不 `hub_live(id)` 现取 ctl（那是重拨后的新会话）。
        if let Some(hit) = session.supervise() {
            return hit;
        }
        // 两个截止时刻取早者：空闲 600 s（每次输入刷新）与硬上限 7200 s。
        let wake =
            (session.last_input + frames::IDLE_TIMEOUT).min(session.started + frames::SESSION_MAX);

        tokio::select! {
            _ = session.control().closed() => return (CloseReason::DeviceOffline, DETAIL_DEVICE_OFFLINE),
            message = browser.next() => {
                match message {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => {
                        return (CloseReason::ClientClosed, DETAIL_CLIENT_CLOSED);
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = browser.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    // 浏览器侧只允许 JSON 文本帧；二进制帧忽略（不是协议的一部分）。
                    Some(Ok(Message::Binary(_))) => {}
                    Some(Ok(Message::Text(text))) => {
                        session.on_browser_text(&text, browser, device).await;
                    }
                }
            }
            message = device.next() => {
                match message {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => {
                        return (CloseReason::DeviceShellLost, DETAIL_SHELL_LOST);
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = device.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Binary(bytes))) => {
                        session.on_device_bytes(&bytes, browser).await;
                    }
                    // 设备侧正常只发 Binary；Text 也照收（宽容，不丢输出）。
                    // 例外是 `shell_meta`：它是控制帧（设备声明自己有没有 pty），
                    // 必须原样转给浏览器——写进终端画面会在提示符前多出一行 JSON。
                    Some(Ok(Message::Text(text))) => {
                        if frames::is_shell_meta(&text) {
                            let _ = browser.send(Message::Text(text)).await;
                        } else {
                            session.on_device_bytes(text.as_bytes(), browser).await;
                        }
                    }
                }
            }
            _ = tokio::time::sleep_until(wake) => {
                if session.started + frames::SESSION_MAX
                    <= session.last_input + frames::IDLE_TIMEOUT
                {
                    return (CloseReason::SessionLimit, DETAIL_SESSION_LIMIT);
                }
                return (CloseReason::IdleTimeout, DETAIL_IDLE);
            }
        }
    }
}

#[cfg(test)]
#[path = "console_tests.rs"]
mod tests;

/// QA 补测：dial 阶段关 tab 的槽释放、反向 `busy`、输入字节边界。
/// 单独成文件是因为 `console_tests.rs` 已接近 600 行硬上限。
#[cfg(test)]
#[path = "console_path_tests.rs"]
mod path_tests;
