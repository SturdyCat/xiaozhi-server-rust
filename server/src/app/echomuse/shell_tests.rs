//! Shell safety and actual loopback WS wire tests. All commands are fake bytes;
//! these tests never spawn a shell or connect to real hardware.
use super::*;
use crate::app::echomuse as em;
use axum::routing::get;
use tungstenite::client::IntoClientRequest;

fn online(
    id: &str,
    peer: IpAddr,
    authenticated: bool,
) -> (
    UnboundedSender<ControlOut>,
    tokio::sync::mpsc::UnboundedReceiver<ControlOut>,
) {
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

fn localhost() -> IpAddr {
    "127.0.0.1".parse().unwrap()
}

#[test]
fn sentinels_split_at_every_byte_and_utf8_are_aggregated() {
    let wire = "你好\n__DONE__:0\n".as_bytes();
    for split in 0..wire.len() {
        let mut o = Output::new("__DONE__:");
        let a = o.push(&wire[..split]).unwrap();
        let b = o.push(&wire[split..]).unwrap();
        assert_eq!(a.or(b), Some("你好".into()));
    }
}

#[test]
fn missing_nonzero_invalid_and_overflow_never_report_success() {
    let mut o = Output::new("MARK:");
    assert_eq!(o.push(b"partial output").unwrap(), None);
    let mut o = Output::new("MARK:");
    assert_eq!(
        o.push(b"bad\nMARK:42\n"),
        Err(ShellError::CommandFailed(42))
    );
    let mut o = Output::new("MARK:");
    assert_eq!(o.push(b"\nMARK:wat\n"), Err(ShellError::InvalidMarker));
    let mut o = Output::new("MARK:");
    assert_eq!(
        o.push(&vec![b'x'; MAX_OUTPUT + 1]),
        Err(ShellError::OutputLimit)
    );
    assert!(o.buffer.is_empty(), "overflow never retained");
}

#[tokio::test]
async fn only_owner_cleans_busy_slot_and_duplicate_claim_is_rejected() {
    let _g = em::hub_tests::HUB_LOCK.lock().unwrap();
    let id = "shell-owner";
    let (ctl, mut rx) = online(id, localhost(), true);
    let live = em::hub_live(id).unwrap().1;
    let (op, _socket_rx) = Operation::reserve(id, ctl.clone(), &live, OP_TIMEOUT).unwrap();
    assert!(matches!(
        Operation::reserve(id, ctl.clone(), &live, OP_TIMEOUT),
        Err(ShellError::Busy)
    ));
    assert!(rx.try_recv().is_err(), "busy waiter sent no close");
    assert!(matches!(
        claim(id, "192.0.2.1".parse().unwrap()),
        Err(ShellError::Unauthorized)
    ));
    let _sender = claim(id, localhost()).unwrap();
    assert!(matches!(
        claim(id, localhost()),
        Err(ShellError::Unsolicited)
    ));
    drop(op);
    assert!(!pending().lock().unwrap().contains_key(id));
    assert!(matches!(rx.try_recv(), Ok(ControlOut::Json(s)) if s.contains("shell_close")));
    em::hub_remove(id);
}

#[tokio::test]
async fn expired_and_replaced_control_cannot_claim_and_old_cleanup_spares_new() {
    let _g = em::hub_tests::HUB_LOCK.lock().unwrap();
    let id = "shell-stale";
    let (ctl, mut rx) = online(id, localhost(), true);
    let live = em::hub_live(id).unwrap().1;
    let (op, _) = Operation::reserve(id, ctl.clone(), &live, Duration::ZERO).unwrap();
    assert!(matches!(
        claim(id, localhost()),
        Err(ShellError::Unsolicited)
    ));
    drop(op);
    let (op, _) = Operation::reserve(id, ctl.clone(), &live, OP_TIMEOUT).unwrap();
    let (new_ctl, mut new_rx) = online(id, localhost(), true);
    assert!(matches!(claim(id, localhost()), Err(ShellError::Offline)));
    em::hub_remove_session(id, &ctl);
    assert!(em::hub_ctl(id).unwrap().same_channel(&new_ctl));
    drop(op);
    assert!(new_rx.try_recv().is_err());
    assert!(rx.try_recv().is_ok());
    em::hub_remove(id);
}

#[tokio::test]
async fn unauthenticated_pairing_missing_peer_and_offline_fail_closed() {
    let _g = em::hub_tests::HUB_LOCK.lock().unwrap();
    let (ctl, rx) = online("shell-auth", localhost(), false);
    assert!(matches!(
        Operation::reserve(
            "shell-auth",
            ctl.clone(),
            &LiveControl::new(Some(localhost()), false),
            OP_TIMEOUT
        ),
        Err(ShellError::Unauthorized)
    ));
    assert!(matches!(
        Operation::reserve(
            "shell-auth",
            ctl.clone(),
            &LiveControl::new(None, true),
            OP_TIMEOUT
        ),
        Err(ShellError::PeerUnavailable)
    ));
    drop(rx);
    assert!(matches!(
        Operation::reserve(
            "shell-auth",
            ctl,
            &LiveControl::new(Some(localhost()), true),
            OP_TIMEOUT
        ),
        Err(ShellError::Offline)
    ));
    em::hub_remove("shell-auth");
}

#[tokio::test]
async fn deadline_and_abort_cleanup_pending_and_send_close() {
    let _g = em::hub_tests::HUB_LOCK.lock().unwrap();
    let id = "shell-deadline";
    let (_ctl, mut rx) = online(id, localhost(), true);
    let task = tokio::spawn(run_timed(id, em::system::INFO, Duration::from_millis(1)));
    assert!(matches!(rx.recv().await, Some(ControlOut::Json(s)) if s.contains("shell_open")));
    assert_eq!(task.await.unwrap(), Err(ShellError::Timeout));
    assert!(!pending().lock().unwrap().contains_key(id));
    assert!(matches!(rx.try_recv(), Ok(ControlOut::Json(s)) if s.contains("shell_close")));
    let task = tokio::spawn(run(id, em::system::INFO));
    assert!(
        matches!(rx.recv().await, Some(ControlOut::Json(s)) if s == r#"{"type":"shell_open"}"#)
    );
    task.abort();
    let _ = task.await;
    assert!(!pending().lock().unwrap().contains_key(id));
    assert!(matches!(rx.recv().await, Some(ControlOut::Json(s)) if s.contains("shell_close")));
    em::hub_remove(id);
}

#[derive(Clone)]
struct TestState {
    reg: Arc<Registry>,
    admin_token: &'static str,
}

async fn handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Path(id): Path<String>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<TestState>,
) -> Response {
    admit(
        ws,
        &headers,
        &id,
        peer.ip(),
        &state.reg,
        (state.admin_token, "device-secret"),
    )
}

async fn server(
    id: &str,
    admin_token: &'static str,
) -> (String, tokio::task::JoinHandle<()>, std::path::PathBuf) {
    let (reg, path) = em::tests::temp_registry(id);
    let mut record = em::tests::reg_load(id, "test");
    record.base_os = Some("emos".into());
    reg.upsert(&record, 0);
    reg.approve(id, None);
    let router = axum::Router::new()
        .route("/shell/{id}", get(handler))
        .with_state(TestState {
            reg: Arc::new(reg),
            admin_token,
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/shell/{id}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (url, task, path)
}

fn request(url: &str, token: &str) -> tungstenite::http::Request<()> {
    let mut req = url.into_client_request().unwrap();
    req.headers_mut()
        .insert("X-Em-Token", token.parse().unwrap());
    req
}

#[tokio::test]
async fn loopback_upgrade_rejects_tokens_peer_unsolicited_and_unsecured_admin() {
    let _g = em::hub_tests::HUB_LOCK.lock().unwrap();
    let id = "shell-wire-auth";
    let (url, server, path) = server(id, "admin-secret").await;
    let (ctl, _rx) = online(id, localhost(), true);
    for (token, code) in [("wrong", 401), ("device-secret", 409)] {
        let url = url.clone();
        let status =
            tokio::task::spawn_blocking(move || match tungstenite::connect(request(&url, token)) {
                Err(tungstenite::Error::Http(r)) => r.status().as_u16(),
                other => panic!("expected HTTP denial, got {other:?}"),
            })
            .await
            .unwrap();
        assert_eq!(status, code);
    }
    let live = LiveControl::new(Some("192.0.2.1".parse().unwrap()), true);
    let (op, _) = Operation::reserve(id, ctl, &live, OP_TIMEOUT).unwrap();
    let status = tokio::task::spawn_blocking(move || {
        match tungstenite::connect(request(&url, "device-secret")) {
            Err(tungstenite::Error::Http(r)) => r.status().as_u16(),
            other => panic!("expected peer denial, got {other:?}"),
        }
    })
    .await
    .unwrap();
    assert_eq!(status, 401);
    drop(op);
    server.abort();
    em::hub_remove(id);
    let _ = std::fs::remove_file(path);
    let (url, server, path) = self::server("shell-no-admin", "").await;
    let status = tokio::task::spawn_blocking(move || {
        match tungstenite::connect(request(&url, "device-secret")) {
            Err(tungstenite::Error::Http(r)) => r.status().as_u16(),
            other => panic!("expected config denial, got {other:?}"),
        }
    })
    .await
    .unwrap();
    assert_eq!(status, 503);
    server.abort();
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn loopback_fixed_info_accepts_text_input_and_mixed_split_output() {
    let _g = em::hub_tests::HUB_LOCK.lock().unwrap();
    let id = "shell-wire-info";
    let (url, server, path) = server(id, "admin-secret").await;
    let (_ctl, mut rx) = online(id, localhost(), true);
    let operation = tokio::spawn(run(id, em::system::INFO));
    match rx.recv().await.unwrap() {
        ControlOut::Json(t) => assert_eq!(t, r#"{"type":"shell_open"}"#, "must not request pty"),
        other => panic!("unexpected {other:?}"),
    }
    let device = tokio::task::spawn_blocking(move || {
        let (mut ws, _) = tungstenite::connect(request(&url, "device-secret")).unwrap();
        let tungstenite::Message::Text(text) = ws.read().unwrap() else {
            panic!("stdin must be text")
        };
        assert!(text.contains(em::system::INFO));
        let (_, tail) = text.split_once("__EM_DONE_").unwrap();
        let uuid = tail.split_once("__:").unwrap().0;
        let output = format!("fake system info\n__EM_DONE_{uuid}__:0\n");
        let split = output.len() - 4;
        ws.send(tungstenite::Message::Binary(
            output.as_bytes()[..split].to_vec().into(),
        ))
        .unwrap();
        ws.send(tungstenite::Message::Text(
            output[split..].to_string().into(),
        ))
        .unwrap();
        let _ = ws.read();
    });
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), operation)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        "fake system info"
    );
    device.await.unwrap();
    assert!(!pending().lock().unwrap().contains_key(id));
    assert!(matches!(rx.recv().await, Some(ControlOut::Json(s)) if s.contains("shell_close")));
    server.abort();
    em::hub_remove(id);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn loopback_disconnect_without_marker_is_error_and_cleans_slot() {
    let _g = em::hub_tests::HUB_LOCK.lock().unwrap();
    let id = "shell-wire-eof";
    let (url, server, path) = server(id, "admin-secret").await;
    let (_ctl, mut rx) = online(id, localhost(), true);
    let operation = tokio::spawn(run(id, em::system::INFO));
    rx.recv().await.unwrap();
    let device = tokio::task::spawn_blocking(move || {
        let (mut ws, _) = tungstenite::connect(request(&url, "device-secret")).unwrap();
        ws.read().unwrap();
        ws.send(tungstenite::Message::Binary(b"partial".to_vec().into()))
            .unwrap();
        let _ = ws.close(None);
    });
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), operation)
            .await
            .unwrap()
            .unwrap(),
        Err(ShellError::Transport)
    );
    device.await.unwrap();
    assert!(!pending().lock().unwrap().contains_key(id));
    server.abort();
    em::hub_remove(id);
    let _ = std::fs::remove_file(path);
}
