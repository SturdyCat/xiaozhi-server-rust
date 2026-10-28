//! 控制台回归：槽互斥/租约 owner/收尾顺序/帧字面量/限流/UTF-8 边界 + 真实 loopback WS。
//!
//! 全部命令都是假字节：**不 spawn shell、不连真机**。涉及 hub 的用例持 `HUB_LOCK` 串行。

use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;

use axum::extract::ConnectInfo;
use axum::http::StatusCode;
use axum::routing::get;
use tokio::net::TcpListener;
use tokio::sync::mpsc::UnboundedReceiver;

use super::*;
use crate::device::echomuse as em;
use crate::device::echomuse::runtime::LiveControl;

// ---- 共享助手 ----

fn localhost() -> IpAddr {
    "127.0.0.1".parse().unwrap()
}

/// 登记一台"已认证在线"的假设备，返回 control 出站口与接收端（接收端即"设备收到
/// 什么"的记录序列，用于断言 `shell_open`/`shell_close` 的次数与顺序）。
fn online(
    id: &str,
    peer: IpAddr,
    authenticated: bool,
) -> (UnboundedSender<ControlOut>, UnboundedReceiver<ControlOut>) {
    let (ctl, rx) = tokio::sync::mpsc::unbounded_channel();
    let (up_tx, _) = tokio::sync::mpsc::unbounded_channel();
    em::hub_insert(
        id,
        em::PendingDevice {
            up_tx,
            data_rx: None,
            since: std::time::Instant::now(),
            ctl_tx: Some(ctl.clone()),
            data_tx: None,
            ble: Arc::new(Mutex::new(em::ble::BleLink::default())),
            management: Arc::new(LiveControl::new(Some(peer), authenticated)),
        },
    );
    (ctl, rx)
}

/// 排空 ctl 接收端，返回已入队的 JSON 文本（顺序即入队顺序）。
fn drain(rx: &mut UnboundedReceiver<ControlOut>) -> Vec<String> {
    let mut out = Vec::new();
    while let Ok(item) = rx.try_recv() {
        match item {
            ControlOut::Json(text) => out.push(text),
            ControlOut::Shutdown => out.push("<shutdown>".to_string()),
        }
    }
    out
}

fn slot_owner(id: &str) -> Option<uuid::Uuid> {
    shell::pending().lock().unwrap().get(id).map(|p| p.owner)
}

// ---- 槽互斥与租约 owner ----

/// 设计 §5.1 不变式 1/2：没拿到 `Operation` 的调用方结构上不可能释放别人的槽。
#[tokio::test]
async fn busy_contest_never_touches_holder_slot_or_sends_close() {
    let _g = em::hub_tests::HUB_LOCK.lock().unwrap();
    let id = "console-busy";
    let (ctl, mut rx) = online(id, localhost(), true);
    let live = hub_live(id).unwrap().1;
    let (holder, _dial) = Operation::reserve(id, ctl.clone(), &live, frames::DIAL_TIMEOUT).unwrap();
    let owner = holder.owner;
    assert_eq!(slot_owner(id), Some(owner));

    for _ in 0..3 {
        // `Operation` 故意不实现 `Debug`（内含 channel 与 UUID，无诊断价值），故用 `.err()`。
        assert_eq!(
            Operation::reserve(id, ctl.clone(), &live, frames::DIAL_TIMEOUT).err(),
            Some(ShellError::Busy)
        );
        // 等待者持槽失败，owner 必须是持有者（不抢槽、不改写）。
        assert_eq!(slot_owner(id), Some(owner));
    }
    assert!(
        drain(&mut rx).is_empty(),
        "Busy 申请者一次 shell_close/shell_open 都不许发"
    );

    drop(holder);
    assert_eq!(slot_owner(id), None, "持有者 drop 才释槽");
    assert_eq!(drain(&mut rx), vec![r#"{"type":"shell_close"}"#]);
    em::hub_remove(id);
}

/// 既有行为不许变：回拨超期 → `Unsolicited`；15 s 内二次回拨 → `Unsolicited`。
#[tokio::test]
async fn stale_and_duplicate_dial_are_unsolicited() {
    let _g = em::hub_tests::HUB_LOCK.lock().unwrap();
    let id = "console-dial";
    let (ctl, _rx) = online(id, localhost(), true);
    let live = hub_live(id).unwrap().1;

    let (expired, _dial) = Operation::reserve(id, ctl.clone(), &live, Duration::ZERO).unwrap();
    assert_eq!(
        shell::claim(id, localhost(), false).err(),
        Some(ShellError::Unsolicited),
        "回拨晚于 deadline 必须判 Unsolicited（服务端已放弃）"
    );
    drop(expired);

    let (live_lease, _dial) =
        Operation::reserve(id, ctl.clone(), &live, frames::DIAL_TIMEOUT).unwrap();
    let _first = shell::claim(id, localhost(), false).unwrap();
    assert_eq!(
        shell::claim(id, localhost(), false).err(),
        Some(ShellError::Unsolicited),
        "15 s 内二次回拨必须判 Unsolicited（socket 已被取走）"
    );
    drop(live_lease);
    em::hub_remove(id);
}

/// 收尾顺序：`shell_close` **先入队**、槽后释放——这样下一个 owner 的 `shell_open`
/// 不可能插到旧 close 前面（`shell.rs::Drop` 单锁临界区的语义，本任务未改其逻辑）。
#[tokio::test]
async fn teardown_enqueues_close_before_new_owner_opens() {
    let _g = em::hub_tests::HUB_LOCK.lock().unwrap();
    let id = "console-order";
    let (ctl, mut rx) = online(id, localhost(), true);
    let live = hub_live(id).unwrap().1;

    let (old, _dial) = Operation::reserve(id, ctl.clone(), &live, frames::DIAL_TIMEOUT).unwrap();
    drop(old);
    // 旧租约已释放：新 owner 立刻能占槽并下发 shell_open。
    let (fresh, _dial) = Operation::reserve(id, ctl.clone(), &live, frames::DIAL_TIMEOUT).unwrap();
    ctl.send(ControlOut::Json(SHELL_OPEN.to_string())).unwrap();

    assert_eq!(
        drain(&mut rx),
        vec![
            r#"{"type":"shell_close"}"#.to_string(),
            SHELL_OPEN.to_string()
        ],
        "旧 close 必须排在新 open 之前（先 enqueue 再删槽）"
    );
    drop(fresh);
    em::hub_remove(id);
}

/// 一次性路径回归：控制台占槽期间 `/system/*` 照旧 409 `busy`；放开后一次性路径可用；
/// 日志白名单与固定命令一个字节都没变。
#[tokio::test]
async fn console_slot_keeps_one_shot_paths_and_whitelist_intact() {
    let _g = em::hub_tests::HUB_LOCK.lock().unwrap();
    let id = "console-oneshot";
    let (ctl, _rx) = online(id, localhost(), true);
    let live = hub_live(id).unwrap().1;
    let (console_lease, _dial) =
        Operation::reserve(id, ctl.clone(), &live, frames::DIAL_TIMEOUT).unwrap();

    // 系统接口 → `Operation::reserve` → Busy（不需要设备回拨即可判定）。
    assert_eq!(
        shell::run(id, em::system::INFO).await.unwrap_err(),
        ShellError::Busy
    );
    let response = ShellError::Busy.response();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(response.into_body(), 8 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], "busy");
    assert_eq!(body["error"], "another system operation is active");

    // 固定命令与日志白名单原样：只做字面量断言，不跑真机命令。
    assert!(em::system::INFO.starts_with("uname -a && cat /etc/os-release"));
    assert!(em::system::REBOOT.contains("kill -TERM 1"));
    for source in [
        "server",
        "messages",
        "messages_last",
        "last_kmsg_prev",
        "supervisor",
        "net",
    ] {
        assert!(
            em::system::log_command(source).is_some(),
            "日志源 {source} 必须仍在白名单"
        );
    }
    assert!(em::system::log_command("ledScene").is_none());
    assert!(em::system::confirmed(&serde_json::json!({"confirm": true})));

    // 控制台放开槽后一次性路径立刻可占（同一把槽，同一段 Drop）。
    drop(console_lease);
    assert_eq!(slot_owner(id), None);
    let (again, _dial) = Operation::reserve(id, ctl.clone(), &live, frames::DIAL_TIMEOUT).unwrap();
    drop(again);
    em::hub_remove(id);
}

// ---- 帧字面量（前后端逐字一致，防漂移） ----

fn frame_json(frame: &ConsoleFrame) -> serde_json::Value {
    serde_json::from_str(&frames::encode(frame)).expect("帧必须是合法 JSON")
}

#[test]
fn frame_types_and_codes_match_shared_contract() {
    // ready：任何 output 之前的第一帧，mode 一期恒 "pipe"。
    let ready = frame_json(&ConsoleFrame::Ready {
        device_id: "d1".into(),
        mode: frames::MODE_PIPE,
        insecure: true,
        notice: "n".into(),
    });
    assert_eq!(ready["type"], "ready");
    assert_eq!(ready["device_id"], "d1");
    assert_eq!(ready["mode"], "pipe");
    assert_eq!(ready["insecure"], true);
    assert_eq!(ready["notice"], "n");

    let output = frame_json(&ConsoleFrame::Output {
        text: "hi\n".into(),
    });
    assert_eq!(output["type"], "output");
    assert_eq!(output["text"], "hi\n");

    let notice = frame_json(&ConsoleFrame::Notice {
        code: frames::NoticeCode::OutputCapped,
        text: frames::COPY_CAPPED.into(),
    });
    assert_eq!(notice["type"], "notice");
    assert_eq!(notice["code"], "output-capped");
    assert_eq!(notice["text"], frames::COPY_CAPPED);

    // error.code 五个字面量 + 定值文案（§5.2 / §8.3）。
    for (code, literal, copy) in [
        (ErrorCode::Busy, "busy", frames::COPY_BUSY),
        (
            ErrorCode::InputTooLong,
            "input-too-long",
            frames::COPY_INPUT_TOO_LONG,
        ),
        (
            ErrorCode::RateLimited,
            "rate-limited",
            frames::COPY_RATE_LIMITED,
        ),
        (
            ErrorCode::DeviceOffline,
            "device-offline",
            frames::COPY_DEVICE_OFFLINE,
        ),
    ] {
        let json = frame_json(&ConsoleFrame::Error {
            code,
            error: code.message(""),
        });
        assert_eq!(json["type"], "error");
        assert_eq!(json["code"], literal);
        assert_eq!(json["error"], copy);
    }
    let internal = frame_json(&ConsoleFrame::Error {
        code: ErrorCode::Internal,
        error: ErrorCode::Internal.message("占槽失败"),
    });
    assert_eq!(internal["code"], "internal");
    assert_eq!(
        internal["error"],
        format!("{}占槽失败", frames::COPY_INTERNAL)
    );

    // closed.reason 八个共享字面量 + 仅审计用的 client-closed（`code()` 与 serde 同步）。
    let all = [
        (CloseReason::IdleTimeout, "idle-timeout"),
        (CloseReason::SessionLimit, "session-limit"),
        (CloseReason::DeviceOffline, "device-offline"),
        (CloseReason::Superseded, "superseded"),
        (CloseReason::DeviceShellLost, "device-shell-lost"),
        (CloseReason::Replaced, "replaced"),
        (CloseReason::ServerShutdown, "server-shutdown"),
        (CloseReason::Internal, "internal"),
        (CloseReason::ClientClosed, "client-closed"),
    ];
    for (reason, literal) in all {
        assert_eq!(reason.code(), literal);
        assert_eq!(
            serde_json::to_value(reason).unwrap().as_str(),
            Some(literal),
            "serde 与 code() 必须同字面量"
        );
        let json = frame_json(&ConsoleFrame::Closed {
            reason,
            detail: "d".into(),
            inputs: 3,
            bytes_out: 4096,
            secs: 7,
        });
        assert_eq!(json["type"], "closed");
        assert_eq!(json["reason"], literal);
        assert_eq!(json["detail"], "d");
        assert_eq!(json["inputs"], 3);
        assert_eq!(json["bytes_out"], 4096);
        assert_eq!(json["secs"], 7);
    }

    let pong = frame_json(&ConsoleFrame::Pong);
    assert_eq!(pong["type"], "pong");

    // shell_meta：与官方 dashboard 帧逐字一致（type 必须是 "shell_meta"，不是 kebab）。
    let meta = frame_json(&ConsoleFrame::ShellMeta { pty: true });
    assert_eq!(meta["type"], "shell_meta");
    assert_eq!(meta["pty"], true);
    let meta_pipe = frame_json(&ConsoleFrame::ShellMeta { pty: false });
    assert_eq!(meta_pipe["type"], "shell_meta");
    assert_eq!(meta_pipe["pty"], false);
}

#[test]
fn inbound_frames_parse_by_type_and_reject_junk() {
    assert_eq!(
        frames::parse_incoming(r#"{"type":"input","data":"uname -a"}"#),
        frames::Incoming::Input("uname -a".into())
    );
    // 尾部换行被剥掉（协议规定 data 不含 \n，前端可能带）。
    assert_eq!(
        frames::parse_incoming("{\"type\":\"input\",\"data\":\"ls\\n\"}"),
        frames::Incoming::Input("ls".into())
    );
    assert_eq!(
        frames::parse_incoming(r#"{"type":"ctrl-c"}"#),
        frames::Incoming::CtrlC
    );
    assert_eq!(
        frames::parse_incoming(r#"{"type":"ping"}"#),
        frames::Incoming::Ping
    );
    // `clear` 不存在（清屏是纯 UI 动作）；未知/残缺一律 Unknown，绝不当空命令写设备。
    for junk in [
        r#"{"type":"clear"}"#,
        r#"{"type":"input"}"#,
        r#"{"type":"input","data":123}"#,
        "not json",
        "",
    ] {
        assert_eq!(frames::parse_incoming(junk), frames::Incoming::Unknown);
    }
}

// ---- 限流 / 长度（被拒时零字节写设备） ----

#[test]
fn rate_limit_and_long_line_write_nothing() {
    let now = Instant::now();
    let mut bucket = TokenBucket::new(frames::INPUT_PER_SEC);
    for _ in 0..frames::INPUT_PER_SEC {
        assert!(matches!(
            frames::plan_line(&mut bucket, now, "ls", false),
            frames::InputAction::Write(bytes) if bytes == b"ls\n"
        ));
    }
    assert_eq!(
        frames::plan_line(&mut bucket, now, "ps", false),
        frames::InputAction::Reject(ErrorCode::RateLimited),
        "第 11 帧必须 rate-limited"
    );
    // 拒绝不消耗令牌：同一时刻仍然限流。
    assert_eq!(
        frames::plan_line(&mut bucket, now, "ps", false),
        frames::InputAction::Reject(ErrorCode::RateLimited)
    );

    // 4097 字节 → input-too-long，且 `Reject` 不携带字节（设备侧零字节）。
    let mut bucket = TokenBucket::new(frames::INPUT_PER_SEC);
    let long = "x".repeat(frames::LINE_MAX + 1);
    assert_eq!(
        frames::plan_line(&mut bucket, now, &long, false),
        frames::InputAction::Reject(ErrorCode::InputTooLong)
    );
    // 4096 字节恰好通过，且补一个换行（pipe 模式 sh 靠换行触发执行）。
    match frames::plan_line(&mut bucket, now, &"y".repeat(frames::LINE_MAX), false) {
        frames::InputAction::Write(bytes) => assert_eq!(bytes.len(), frames::LINE_MAX + 1),
        other => panic!("4096 字节必须放行，实际 {other:?}"),
    }

    // ctrl-c：pipe 模式写单字节 0x03。
    let mut bucket = TokenBucket::new(frames::INPUT_PER_SEC);
    assert_eq!(
        frames::plan_ctrl_c(&mut bucket, now, false),
        frames::InputAction::Write(vec![0x03])
    );
}

/// PTY 帧格式（根因修复）：pty 模式 stdin 必须带 `0x00` 头，否则设备
/// `runShellSession` 的 `switch data[0]` 把命令首字符当"其它"静默丢弃；pipe 模式裸字节。
#[test]
fn pty_framing_prefixes_stdin_byte_and_ctrl_c() {
    let now = Instant::now();

    // pty：stdin 帧头 0x00 + 命令 + 换行。
    let mut bucket = TokenBucket::new(frames::INPUT_PER_SEC);
    assert_eq!(
        frames::plan_line(&mut bucket, now, "ls", true),
        frames::InputAction::Write(b"\x00ls\n".to_vec())
    );
    // 多字节命令同样加首字节。
    let mut bucket = TokenBucket::new(frames::INPUT_PER_SEC);
    assert_eq!(
        frames::plan_line(&mut bucket, now, "重启", true),
        frames::InputAction::Write(b"\x00\xe9\x87\x8d\xe5\x90\xaf\n".to_vec())
    );
    // pty：ctrl-c 走 stdin 帧（0x00 + 0x03）。
    let mut bucket = TokenBucket::new(frames::INPUT_PER_SEC);
    assert_eq!(
        frames::plan_ctrl_c(&mut bucket, now, true),
        frames::InputAction::Write(vec![0x00, 0x03])
    );

    // pipe：裸字节，无首字节（一次性操作与老固件）。
    let mut bucket = TokenBucket::new(frames::INPUT_PER_SEC);
    assert_eq!(
        frames::plan_line(&mut bucket, now, "ls", false),
        frames::InputAction::Write(b"ls\n".to_vec())
    );
    let mut bucket = TokenBucket::new(frames::INPUT_PER_SEC);
    assert_eq!(
        frames::plan_ctrl_c(&mut bucket, now, false),
        frames::InputAction::Write(vec![0x03])
    );
}

/// 会话累计 16 MiB：`Forward` → 一次 `Cap` → 之后 `Drop`，会话本身不被断（预算之外无状态）。
#[test]
fn budget_caps_once_without_ending_session() {
    let mut budget = Budget::new(frames::SESSION_BYTES_CAP);
    let chunk = 1024 * 1024;
    for _ in 0..(frames::SESSION_BYTES_CAP / chunk as u64) {
        assert_eq!(budget.admit(chunk), frames::Admit::Forward);
    }
    assert_eq!(budget.used(), frames::SESSION_BYTES_CAP);
    assert_eq!(budget.admit(1), frames::Admit::Cap, "越界那一块只报一次");
    assert_eq!(budget.admit(1), frames::Admit::Drop, "封顶后静默丢弃");
    assert_eq!(budget.admit(4096), frames::Admit::Drop);
    assert_eq!(budget.used(), frames::SESSION_BYTES_CAP);
}

// ---- UTF-8 增量对齐 ----

/// 把 3 字节汉字切在 4 KiB 帧边界上：拼接后无 `�`、字节数不丢。
#[test]
fn utf8_carry_joins_char_split_at_frame_boundary() {
    let mut wire = vec![b'a'; 4094];
    wire.extend_from_slice("一".as_bytes()); // 4094 + 3 = 4097 字节
    wire.push(b'z');
    assert_eq!(wire.len(), 4098);

    let mut carry = Utf8Carry::new();
    let mut out = String::new();
    let mut chunks = 0;
    // 按设备侧 4 KiB 块喂（`device_control.go:1170`），而不是 `FRAME_MAX`：
    // 要测的正是「块边界切断多字节字符」。
    for chunk in wire.chunks(4096) {
        chunks += 1;
        out.push_str(&carry.push(chunk));
    }
    assert_eq!(chunks, 2, "必须真的跨帧（第一块在汉字中间断开）");
    assert_eq!(out, format!("{}一z", "a".repeat(4094)));
    assert!(!out.contains('\u{fffd}'), "合法字符不得变成替换符：{out:?}");
    assert_eq!(out.len(), wire.len(), "字节数不丢");
    assert_eq!(carry.flush(), "");
}

/// 真非法字节：按 `from_utf8_lossy` 语义替换，不清缓冲、不断会话。
#[test]
fn utf8_carry_replaces_invalid_bytes_and_keeps_going() {
    let mut carry = Utf8Carry::new();
    let first = carry.push(&[0xE4, 0xB8]); // 半个「一」
    assert_eq!(first, "");
    let second = carry.push(&[0xFF, b'o', b'k']);
    assert!(
        second.contains('\u{fffd}'),
        "非法字节必须变替换符：{second:?}"
    );
    assert!(second.ends_with("ok"), "后续合法字节照常输出：{second:?}");
    // 残余（若有）flush 出来，不残留。
    let _ = carry.flush();
    assert_eq!(carry.push(b"tail"), "tail");
}

// ---- 真实 loopback 全链路 ----

/// 设备侧回拨路由：只做 `claim`（鉴权部分由 `shell_tests.rs` 覆盖，这里不重复）。
async fn device_route(
    ws: WebSocketUpgrade,
    Path(id): Path<String>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    match shell::claim(&id, peer.ip(), true) {
        Ok(tx) => ws.on_upgrade(move |socket| async move {
            let _ = tx.send(socket);
        }),
        Err(err) => err.response(),
    }
}

/// 浏览器侧路由：跳过管理面鉴权（`Engines` 太重），直接进会话。
async fn client_route(ws: WebSocketUpgrade, Path(id): Path<String>) -> Response {
    ws.on_upgrade(move |socket| async move { serve(socket, id, false).await })
}

async fn loopback_server(id: &str) -> (String, String, tokio::task::JoinHandle<()>) {
    let router = axum::Router::new()
        .route("/shell/{id}", get(device_route))
        .route("/console/{id}", get(client_route));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("ws://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (
        format!("{base}/shell/{id}"),
        format!("{base}/console/{id}"),
        task,
    )
}

/// 读一帧 JSON（浏览器侧断言用；阻塞客户端跑在 blocking 线程上）。
fn next_json<S: std::io::Read + std::io::Write>(
    ws: &mut tungstenite::WebSocket<S>,
) -> serde_json::Value {
    match ws.read().expect("必须收到一帧") {
        tungstenite::Message::Text(text) => serde_json::from_str(&text).expect("帧必须是 JSON"),
        other => panic!("意外的帧：{other:?}"),
    }
}

/// 全链路：`shell_open`（不带 pty）→ 设备回拨 → `ready`（先于任何 output）→
/// 输入直达设备 stdin → 设备 Binary 输出变 `output` 帧 → 浏览器关闭后收尾（
/// 设备 socket 被关、槽释放、`shell_close` 入队）。
#[tokio::test]
async fn loopback_ready_output_and_client_close_teardown() {
    let _g = em::hub_tests::HUB_LOCK.lock().unwrap();
    let id = "console-wire";
    let (shell_url, console_url, server) = loopback_server(id).await;
    let (ctl, mut rx) = online(id, localhost(), true);
    let _ = ctl;

    let client_id = id.to_string();
    let client = tokio::task::spawn_blocking(move || {
        let (mut ws, _) = tungstenite::connect(console_url.as_str()).unwrap();
        // 1) 第一帧必须是 ready，且 mode=pty（终端模式：提示符与回显由设备打）。
        let ready = next_json(&mut ws);
        assert_eq!(ready["type"], "ready");
        assert_eq!(ready["mode"], "pty");
        assert_eq!(ready["device_id"], client_id.as_str());
        assert_eq!(ready["insecure"], false);
        assert!(ready["notice"].as_str().is_some_and(|n| !n.is_empty()));

        // 1b) ready 之后紧跟 shell_meta：告知浏览器设备真实 shell 模式（pty=true，
        // 真终端）。该帧由本任务新增（console.rs send_shell_meta），漏读会让下一帧
        // output 被误判为 shell_meta，引发断言失败并毒化 HUB_LOCK。
        let meta = next_json(&mut ws);
        assert_eq!(meta["type"], "shell_meta");
        assert_eq!(meta["pty"], true);

        // 2) 输入一行 → 服务端写设备 stdin（raw 字节 + 换行）。
        ws.send(tungstenite::Message::Text(
            r#"{"type":"input","data":"uname -a"}"#.into(),
        ))
        .unwrap();

        // 3) 设备回的 Binary 变成 output 帧。
        let output = next_json(&mut ws);
        assert_eq!(output["type"], "output");
        assert_eq!(output["text"], "fake-kernel\n");

        // 4) 浏览器先关：服务端不发 closed 帧，直接走收尾。
        ws.close(None).unwrap();
        let _ = ws.read();
    });

    // 服务端占槽后立刻下发 shell_open，**带 pty**：控制台是终端，不是日志框。
    // 这条断言原来是反的（"不得请求 pty"），也正是网页上一直没有提示符/回显的来源。
    match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
        Ok(Some(ControlOut::Json(text))) => {
            assert_eq!(
                text, r#"{"type":"shell_open","pty":true}"#,
                "控制台必须请求 pty（一次性操作才走 pipe）"
            )
        }
        other => panic!("未收到 shell_open：{other:?}"),
    }

    // 设备侧回拨（真实固件行为：收到 shell_open 才拨 /shell/{id}）。
    let device = tokio::task::spawn_blocking(move || {
        let (mut ws, _) = tungstenite::connect(shell_url.as_str()).unwrap();
        let stdin = match ws.read().unwrap() {
            tungstenite::Message::Binary(bytes) => bytes.to_vec(),
            tungstenite::Message::Text(text) => text.as_bytes().to_vec(),
            other => panic!("意外的 stdin 帧：{other:?}"),
        };
        assert_eq!(stdin, b"\x00uname -a\n", "PTY 模式输入必须带 0x00 帧头直达设备 stdin");
        ws.send(tungstenite::Message::Binary(
            b"fake-kernel\n".to_vec().into(),
        ))
        .unwrap();
        // 收尾第 3 步：服务端关设备 shell socket。
        match ws.read() {
            Ok(tungstenite::Message::Close(_)) | Err(_) => {}
            other => panic!("收尾必须关设备 socket：{other:?}"),
        }
    });

    tokio::time::timeout(Duration::from_secs(5), client)
        .await
        .expect("浏览器侧超时")
        .expect("浏览器侧 panic");
    tokio::time::timeout(Duration::from_secs(5), device)
        .await
        .expect("设备侧超时")
        .expect("设备侧 panic");

    // 收尾第 4/5 步：槽已释放、shell_close 已入队（发往捕获的旧 ctl）。
    assert_eq!(slot_owner(id), None, "会话结束必须释槽");
    assert_eq!(
        drain(&mut rx),
        vec![r#"{"type":"shell_close"}"#],
        "收尾必须发 shell_close"
    );
    server.abort();
    em::hub_remove(id);
}
