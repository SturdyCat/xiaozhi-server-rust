//! 控制台的**收尾路径 / 反向互斥 / 输入字节边界**回归（QA 补测，设计 §5.3、§5.5）。
//!
//! 为什么单独成文件而不是并进 `console_tests.rs`：那个文件已经 584 行（仓规 ≤600 硬上限），
//! 再塞 90 行就越限。这里只补**工程师原测试未覆盖**的三条：
//!
//! 1. 浏览器在**设备回拨之前**关掉 → 槽必须在秒级释放（不是白占 `DIAL_TIMEOUT` 15 s）；
//! 2. 反向互斥：一次性操作占槽时，控制台首帧必须是 `error{busy}`；
//! 3. `LINE_MAX` 按 **UTF-8 字节**算（多字节命令），空 `data` 不 panic。
//!
//! 全部命令都是假字节：**不 spawn shell、不连真机**。涉及 hub 的用例持 `HUB_LOCK` 串行。

use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;

use axum::extract::ConnectInfo;
use axum::response::Response;
use axum::routing::get;
use tokio::net::TcpListener;
use tokio::sync::mpsc::UnboundedReceiver;

use super::*;
use crate::device::echomuse as em;
use crate::device::echomuse::runtime::LiveControl;

// ---- 助手（与 console_tests.rs 同构，但本文件自持一份以免跨模块共享私有项） ----

fn localhost() -> IpAddr {
    "127.0.0.1".parse().unwrap()
}

/// 登记一台"已认证在线"的假设备；返回 control 出站口与接收端（接收端即"设备收到什么"）。
fn online(id: &str, peer: IpAddr) -> (UnboundedSender<ControlOut>, UnboundedReceiver<ControlOut>) {
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
            management: Arc::new(LiveControl::new(Some(peer), true)),
        },
    );
    (ctl, rx)
}

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

async fn device_route(
    ws: WebSocketUpgrade,
    Path(id): Path<String>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    match shell::claim(&id, peer.ip(), false) {
        Ok(tx) => ws.on_upgrade(move |socket| async move {
            let _ = tx.send(socket);
        }),
        Err(err) => err.response(),
    }
}

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

fn next_json<S: std::io::Read + std::io::Write>(
    ws: &mut tungstenite::WebSocket<S>,
) -> serde_json::Value {
    match ws.read().expect("必须收到一帧") {
        tungstenite::Message::Text(text) => serde_json::from_str(&text).expect("帧必须是 JSON"),
        other => panic!("意外的帧：{other:?}"),
    }
}

// ---- 1. dial 阶段浏览器关闭 ----

/// 设计 §5.3 表格里点名"最容易漏"的一条：浏览器在**设备回拨之前**关掉。
///
/// dial 的 `select!` 必须同时听浏览器读端，否则用户关掉 tab 后服务端还要白占槽一整个
/// `DIAL_TIMEOUT`（15 s）——这段时间该设备所有 `/system/*` 都只能拿到 409。
#[tokio::test]
async fn browser_close_during_dial_releases_slot_without_waiting_for_dial_timeout() {
    let _g = em::hub_tests::HUB_LOCK.lock().unwrap();
    let id = "qa-console-dial-close";
    let (_shell_url, console_url, server) = loopback_server(id).await;
    let (ctl, mut rx) = online(id, localhost());
    let _ = ctl;

    let client = tokio::task::spawn_blocking(move || {
        let (mut ws, _) = tungstenite::connect(console_url.as_str()).unwrap();
        // 设备侧故意不回拨：此刻服务端正停在 dial 的 `select!` 里。
        ws.close(None).unwrap();
        let _ = ws.read();
    });

    // 先确认真的占到了槽（`shell_open` 已入队），否则下面的"释放"没有意义。
    match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
        Ok(Some(ControlOut::Json(text))) => assert_eq!(text, SHELL_OPEN),
        other => panic!("未收到 shell_open：{other:?}"),
    }
    assert!(slot_owner(id).is_some(), "dial 期间槽必须处于占用态");

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) && slot_owner(id).is_some() {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let elapsed = start.elapsed();
    assert!(
        slot_owner(id).is_none(),
        "浏览器在 dial 阶段关闭后槽必须释放"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "槽释放耗时 {elapsed:?} 接近 dial 超时，说明 dial 没听浏览器读端"
    );
    assert_eq!(
        drain(&mut rx),
        vec![r#"{"type":"shell_close"}"#],
        "dial 阶段收尾同样要发 shell_close"
    );
    client.await.expect("浏览器侧 panic");
    server.abort();
    em::hub_remove(id);
}

// ---- 2. 反向互斥：一次性操作占槽 → 控制台首帧 busy ----

/// §5.5 反向：一次性操作（系统信息/日志/资产下发）占着槽时，控制台 upgrade 成功后
/// **第一帧**必须是 `error{code:"busy"}`——浏览器拿不到 HTTP 状态码，`busy` 只能靠帧承载。
#[tokio::test]
async fn console_gets_busy_frame_when_one_shot_holds_slot() {
    let _g = em::hub_tests::HUB_LOCK.lock().unwrap();
    let id = "qa-console-rev-busy";
    let (_shell_url, console_url, server) = loopback_server(id).await;
    let (ctl, _rx) = online(id, localhost());
    let live = hub_live(id).unwrap().1;
    // 模拟"一次性操作在跑"：占住同一把槽。
    let (holder, _dial) = Operation::reserve(id, ctl.clone(), &live, frames::DIAL_TIMEOUT).unwrap();

    let first = tokio::task::spawn_blocking(move || {
        let (mut ws, _) = tungstenite::connect(console_url.as_str()).unwrap();
        let frame = next_json(&mut ws);
        let _ = ws.close(None);
        frame
    })
    .await
    .expect("浏览器侧 panic");

    assert_eq!(first["type"], "error", "占槽失败的首帧必须是 error");
    assert_eq!(first["code"], "busy");
    assert_eq!(
        first["error"],
        frames::COPY_BUSY,
        "文案必须与服务端定值一致"
    );

    // 占槽失败方不得影响持有者：它自己的租约根本没拿到，drop 不了别人的槽。
    assert!(slot_owner(id).is_some(), "Busy 方不许释放别人的槽");
    drop(holder);
    assert_eq!(slot_owner(id), None);
    server.abort();
    em::hub_remove(id);
}

// ---- 3. 输入字节边界（多字节 / 空行） ----

/// `LINE_MAX` 是**字节**不是字符：中文 3 字节/字，用字符数判会放过 3 倍超长命令。
/// 空 `data` 不 panic，按协议只写一个换行（`sh` 空命令无副作用；前端负责不发）。
#[test]
fn plan_line_counts_utf8_bytes_and_tolerates_empty_line() {
    let now = Instant::now();

    // 1366 个汉字 = 4098 字节 → 必须拒；1365 个 = 4095 字节 → 放行（含换行 4096）。
    let mut bucket = TokenBucket::new(frames::INPUT_PER_SEC);
    let too_long_cjk = "一".repeat(1366);
    assert_eq!(too_long_cjk.len(), 4098);
    assert_eq!(
        frames::plan_line(&mut bucket, now, &too_long_cjk, false),
        frames::InputAction::Reject(ErrorCode::InputTooLong),
        "4098 字节的中文必须拒（按字节算，不是按字符）"
    );

    let mut bucket = TokenBucket::new(frames::INPUT_PER_SEC);
    match frames::plan_line(&mut bucket, now, &"一".repeat(1365), false) {
        frames::InputAction::Write(bytes) => assert_eq!(bytes.len(), 4096),
        other => panic!("4095 字节必须放行，实际 {other:?}"),
    }

    let mut bucket = TokenBucket::new(frames::INPUT_PER_SEC);
    assert_eq!(
        frames::plan_line(&mut bucket, now, "", false),
        frames::InputAction::Write(vec![b'\n']),
        "空行只写换行，不 panic、不写别的字节"
    );
}
