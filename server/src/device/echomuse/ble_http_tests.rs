//! Actual loopback HTTP routes and data-channel tests. Never touch Bluetooth or
//! device hardware; firmware frames are simulated bytes on captured channels.
use super::*;
use crate::device::echomuse::hub_tests::HUB_LOCK;
use tokio::sync::mpsc::{self, UnboundedReceiver};

#[derive(Clone)]
struct TestContext {
    config: Arc<Config>,
    registry: Arc<registry::Registry>,
}
impl Context for TestContext {
    fn config(&self) -> &Config {
        &self.config
    }
    fn registry(&self) -> &registry::Registry {
        &self.registry
    }
}

struct TestServer {
    url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn context(id: &str) -> (TestContext, std::path::PathBuf) {
    let (registry, path) = em::tests::temp_registry(id);
    let mut register = em::tests::reg_load(id, "BLE test");
    register.capabilities = vec!["ble_connect".into()];
    registry.upsert(&register, 0);
    registry.approve(id, None);
    let mut config = Config::default();
    config.echomuse.enabled = true;
    config.echomuse.ble_proxy = true;
    config.echomuse.token = "device-secret".into();
    config.server.expected_token = "admin-secret".into();
    (
        TestContext {
            config: Arc::new(config),
            registry: Arc::new(registry),
        },
        path,
    )
}

fn online(
    id: &str,
    authenticated: bool,
    ready: bool,
) -> (UnboundedReceiver<ControlOut>, UnboundedReceiver<DataOut>) {
    let (ctl, ctl_rx) = mpsc::unbounded_channel();
    let (data, data_rx) = mpsc::unbounded_channel();
    let (up, _) = mpsc::unbounded_channel();
    let (_, unclaimed) = mpsc::unbounded_channel();
    em::hub_insert(
        id,
        em::PendingDevice {
            up_tx: up,
            data_rx: if ready { None } else { Some(unclaimed) },
            since: std::time::Instant::now(),
            ctl_tx: Some(ctl),
            data_tx: Some(data),
            ble: Arc::new(Mutex::new(BleLink::default())),
            management: Arc::new(em::runtime::LiveControl::new(None, authenticated)),
        },
    );
    (ctl_rx, data_rx)
}

async fn server(context: TestContext) -> TestServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router::<TestContext>().with_state(context))
            .await
            .unwrap();
    });
    TestServer { url, task }
}

async fn call(
    server: &TestServer,
    id: &str,
    endpoint: &str,
    body: Option<Value>,
    token: &str,
) -> (u16, Value) {
    let client = reqwest::Client::new();
    let url = format!("{}/api/echomuse/devices/{id}/ble/{endpoint}", server.url);
    let request = if let Some(body) = body {
        client.post(url).json(&body)
    } else {
        client.get(url)
    };
    let response = request
        .bearer_auth(token)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let text = response.text().await.unwrap();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

#[tokio::test]
async fn http_auth_flags_approval_capability_and_control_gate_all_ble_reads() {
    let _g = HUB_LOCK.lock().unwrap();
    let id = "ble-http-gates";
    let (base, path) = context(id);
    let (ctl_rx, data_rx) = online(id, true, true);
    let srv = server(base.clone()).await;
    assert_eq!(call(&srv, id, "adverts", None, "wrong").await.0, 401);
    assert_eq!(
        call(&srv, "missing", "state", None, "admin-secret").await.0,
        404
    );
    drop(srv);
    for (field, expected) in [("enabled", 404), ("ble_proxy", 404)] {
        let mut ctx = base.clone();
        let config = Arc::make_mut(&mut ctx.config);
        match field {
            "enabled" => config.echomuse.enabled = false,
            _ => config.echomuse.ble_proxy = false,
        }
        let srv = server(ctx).await;
        for endpoint in ["adverts", "state"] {
            assert_eq!(
                call(&srv, id, endpoint, None, "admin-secret").await.0,
                expected
            );
        }
    }
    // 产品决策（局域网默认可管理）：清空任一 token 不再 503——请求越过 token 闸门继续
    // 走业务链。用「设备不存在 → 404」证明闸门已放行（旧口径这里会被拦成 503）。
    for field in ["device_token", "admin_token"] {
        let mut ctx = base.clone();
        let config = Arc::make_mut(&mut ctx.config);
        match field {
            "device_token" => config.echomuse.token.clear(),
            _ => config.server.expected_token.clear(),
        }
        let srv = server(ctx).await;
        for endpoint in ["adverts", "state"] {
            assert_eq!(
                call(&srv, "missing", endpoint, None, "admin-secret")
                    .await
                    .0,
                404,
                "{field} 清空后不得再被 token 闸门拦成 503"
            );
        }
    }
    base.registry.remove(id);
    let record = em::tests::reg_load(id, "unapproved");
    base.registry.upsert(&record, 0);
    let srv = server(base.clone()).await;
    assert_eq!(call(&srv, id, "state", None, "admin-secret").await.0, 403);
    base.registry.approve(id, None);
    assert_eq!(call(&srv, id, "state", None, "admin-secret").await.0, 422);
    let mut record = record;
    record.capabilities = vec!["ble_connect".into()];
    base.registry.upsert(&record, 0);
    let (pair_ctl, pair_data) = online(id, false, true);
    assert_eq!(call(&srv, id, "state", None, "admin-secret").await.0, 401);
    em::hub_remove(id);
    assert_eq!(call(&srv, id, "state", None, "admin-secret").await.0, 409);
    drop((ctl_rx, data_rx, pair_ctl, pair_data));
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn http_snapshots_have_identity_and_remain_readable_when_single_device_disabled() {
    let _g = HUB_LOCK.lock().unwrap();
    let id = "ble-http-snapshot";
    let (ctx, path) = context(id);
    let (_ctl, _data) = online(id, true, true);
    let capture = Capture::new(&ctx.registry, id, None, false).unwrap();
    capture.link.lock().unwrap().merge_adverts(
        &[json!({"addr":"aa:bb:cc:dd:ee:ff","data":"AQ==","rssi":-50})],
        42,
    );
    capture
        .link
        .lock()
        .unwrap()
        .on_gatt(br#"{"t":"slots","free":3,"limit":3,"addrs":[]}"#);
    ctx.registry.set_settings(
        id,
        [
            ("bleProxyEnabled".into(), json!(false)),
            ("bleProxyConnections".into(), json!(false)),
        ]
        .into_iter()
        .collect(),
    );
    let srv = server(ctx).await;
    for endpoint in ["state", "adverts"] {
        let (status, body) = call(
            &srv,
            id,
            &format!("{endpoint}?session_id={}", capture.session),
            None,
            "admin-secret",
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["device_id"], id);
        assert_eq!(body["session_id"], capture.session.to_string());
        if endpoint == "state" {
            assert_eq!(body["slots"]["free"], 3);
        } else {
            assert_eq!(body["count"], 1);
            assert_eq!(body["updatedAt"], 42);
        }
    }
    assert_eq!(
        call(
            &srv,
            id,
            &format!("state?session_id={}", uuid::Uuid::new_v4()),
            None,
            "admin-secret"
        )
        .await
        .1["code"],
        "session_changed"
    );
    assert_eq!(
        call(&srv, id, "state?session_id=invalid", None, "admin-secret")
            .await
            .0,
        400
    );
    let (status, body) = call(
        &srv,
        id,
        "gatt",
        Some(json!({"t":"slots","session_id":capture.session.to_string()})),
        "admin-secret",
    )
    .await;
    assert_eq!(status, 409);
    assert_eq!(body["code"], "ble_disabled");
    em::hub_remove(id);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn http_gatt_six_operations_and_device_failures_preserve_wire_shapes() {
    let _g = HUB_LOCK.lock().unwrap();
    let id = "ble-http-wire";
    let (ctx, path) = context(id);
    let (_ctl, mut data) = online(id, true, true);
    let capture = Capture::new(&ctx.registry, id, None, true).unwrap();
    let link = capture.link.clone();
    let sid = capture.session.to_string();
    let srv = server(ctx).await;
    let simulator = tokio::spawn(async move {
        let mut kinds = Vec::new();
        for _ in 0..7 {
            let frame = data.recv().await.unwrap();
            assert_eq!(frame[0], frames::FRAME_BLE_GATT);
            let request: Value = serde_json::from_slice(&frame[1..]).unwrap();
            assert!(request.get("session_id").is_none());
            let kind = request["t"].as_str().unwrap();
            kinds.push(kind.to_string());
            let req = &request["req"];
            let result = match kind {
                "slots" => {
                    json!({"t":"slots","req":req,"free":2,"limit":3,"addrs":["aa:bb:cc:dd:ee:ff"]})
                }
                "services" => {
                    json!({"t":"result","req":req,"ok":true,"services":[{"uuid":"0000180f-0000-1000-8000-00805f9b34fb","start":1,"end":9,"chars":[{"uuid":"00002a19-0000-1000-8000-00805f9b34fb","handle":2,"value_handle":3,"props":18,"descs":[{"uuid":"00002902-0000-1000-8000-00805f9b34fb","handle":4}]}]}]})
                }
                "read" => json!({"t":"result","req":req,"ok":true,"value":"AQI="}),
                "write" if request["value"] == "AA==" => {
                    json!({"t":"result","req":req,"ok":false,"error":"att","att":5})
                }
                _ => json!({"t":"result","req":req,"ok":true,"mtu":247}),
            };
            link.lock()
                .unwrap()
                .on_gatt(&serde_json::to_vec(&result).unwrap());
        }
        // Keep the simulated data connection alive in the JoinHandle result
        // until every HTTP response has passed its final live-link check.
        (kinds, data)
    });
    let mut writes = 0;
    for kind in [
        "connect",
        "services",
        "read",
        "write",
        "write",
        "disconnect",
        "slots",
    ] {
        let mut body = json!({"t":kind,"session_id":sid});
        if kind != "slots" {
            body["addr"] = json!("aa:bb:cc:dd:ee:ff");
        }
        if matches!(kind, "read" | "write") {
            body["handle"] = json!(3);
        }
        if kind == "write" {
            writes += 1;
            body["value"] = json!(if writes == 1 { "AQ==" } else { "AA==" });
            body["response"] = json!(writes != 1);
        }
        let (status, result) = call(&srv, id, "gatt", Some(body), "admin-secret").await;
        assert_eq!(status, 200);
        assert_eq!(result["device_id"], id);
        assert_eq!(result["session_id"], sid);
        if kind == "slots" {
            assert_eq!(result["t"], "slots");
            assert!(result.get("ok").is_none());
        } else if kind == "write" && writes == 2 {
            assert_eq!(result["ok"], false);
            assert_eq!(result["att"], 5);
        } else {
            assert_eq!(result["ok"], true);
        }
        if kind == "services" {
            assert_eq!(result["services"][0]["chars"][0]["value_handle"], 3);
        }
        if kind == "read" {
            assert_eq!(result["value"], "AQI=");
        }
    }
    let (kinds, _simulator_data) = simulator.await.unwrap();
    assert_eq!(kinds.len(), 7);
    assert!(capture.link.lock().unwrap().pending.is_empty());
    em::hub_remove(id);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn http_gatt_rejects_stale_session_unclaimed_data_invalid_types_and_large_body() {
    let _g = HUB_LOCK.lock().unwrap();
    let id = "ble-http-rejections";
    let (ctx, path) = context(id);
    let (_ctl, _data) = online(id, true, false);
    let srv = server(ctx.clone()).await;
    let (status, body) = call(&srv, id, "gatt", Some(json!({"t":"slots"})), "admin-secret").await;
    assert_eq!(status, 409);
    assert_eq!(body["code"], "data_offline");
    let (_new_ctl, mut data) = online(id, true, true);
    let (status, body) = call(
        &srv,
        id,
        "gatt",
        Some(json!({"t":"slots","session_id":uuid::Uuid::new_v4().to_string()})),
        "admin-secret",
    )
    .await;
    assert_eq!(status, 409);
    assert_eq!(body["code"], "session_changed");
    let (status, _) = call(
        &srv,
        id,
        "gatt",
        Some(json!({"t":"connect","addr":"aa:bb:cc:dd:ee:ff","addr_type":"0"})),
        "admin-secret",
    )
    .await;
    assert_eq!(status, 400);
    let (status, _) = call(
        &srv,
        id,
        "gatt",
        Some(json!({"t":"write","addr":"aa:bb:cc:dd:ee:ff","handle":1,"value":"a".repeat(5000)})),
        "admin-secret",
    )
    .await;
    assert_eq!(status, 413);
    assert!(data.try_recv().is_err());
    em::hub_remove(id);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn timeout_abort_and_closed_data_clean_only_owned_request() {
    let _g = HUB_LOCK.lock().unwrap();
    let id = "ble-http-cancel";
    let (ctx, path) = context(id);
    let (_ctl, mut data) = online(id, true, true);
    let capture = Arc::new(Capture::new(&ctx.registry, id, None, true).unwrap());
    assert_eq!(
        execute(&capture, json!({"t":"slots"}), Duration::ZERO).await,
        Err(BleError::Timeout)
    );
    data.recv().await.unwrap();
    assert!(capture.link.lock().unwrap().pending.is_empty());
    let aborted_capture = capture.clone();
    let task =
        tokio::spawn(
            async move { execute(&aborted_capture, json!({"t":"slots"}), GATT_WAIT).await },
        );
    data.recv().await.unwrap();
    assert_eq!(capture.link.lock().unwrap().pending.len(), 1);
    task.abort();
    let _ = task.await;
    assert!(capture.link.lock().unwrap().pending.is_empty());
    drop(data);
    assert_eq!(
        execute(&capture, json!({"t":"slots"}), GATT_WAIT).await,
        Err(BleError::DataOffline)
    );
    assert!(capture.link.lock().unwrap().pending.is_empty());
    em::hub_remove(id);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn reconnect_does_not_send_to_or_clean_new_session_with_same_request_id() {
    let _g = HUB_LOCK.lock().unwrap();
    let id = "ble-http-reconnect";
    let (ctx, path) = context(id);
    let (_old_ctl, mut old_data) = online(id, true, true);
    let old_capture = Arc::new(Capture::new(&ctx.registry, id, None, true).unwrap());
    let old = old_capture.clone();
    let old_task =
        tokio::spawn(async move { execute(&old, json!({"t":"slots"}), GATT_WAIT).await });
    let old_frame = old_data.recv().await.unwrap();
    let (_new_ctl, mut new_data) = online(id, true, true);
    let new_capture = Arc::new(Capture::new(&ctx.registry, id, None, true).unwrap());
    let new = new_capture.clone();
    let new_task =
        tokio::spawn(async move { execute(&new, json!({"t":"slots"}), GATT_WAIT).await });
    let new_frame = new_data.recv().await.unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&old_frame[1..]).unwrap()["req"],
        serde_json::from_slice::<Value>(&new_frame[1..]).unwrap()["req"]
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), old_task)
            .await
            .unwrap()
            .unwrap(),
        Err(BleError::SessionChanged)
    );
    assert!(old_capture.link.lock().unwrap().pending.is_empty());
    assert_eq!(new_capture.link.lock().unwrap().pending.len(), 1);
    assert_eq!(
        execute(&old_capture, json!({"t":"slots"}), GATT_WAIT).await,
        Err(BleError::SessionChanged)
    );
    let req = serde_json::from_slice::<Value>(&new_frame[1..]).unwrap()["req"].clone();
    new_capture.link.lock().unwrap().on_gatt(
        &serde_json::to_vec(&json!({"t":"slots","req":req,"free":3,"limit":3,"addrs":[]})).unwrap(),
    );
    assert_eq!(new_task.await.unwrap().unwrap()["free"], 3);
    assert!(new_capture.link.lock().unwrap().pending.is_empty());
    em::hub_remove(id);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn same_channel_data_restore_epoch_blocks_old_dispatch_and_cleanup() {
    let _g = HUB_LOCK.lock().unwrap();
    let id = "ble-data-epoch";
    let (ctx, path) = context(id);
    let (_ctl, data) = online(id, true, true);
    let old = Capture::new(&ctx.registry, id, None, true).unwrap();
    let (tx, rx) = oneshot::channel();
    let req = old.link.lock().unwrap().alloc_req();
    let owner = old.link.lock().unwrap().register(req, true, tx).unwrap();
    let old_lease = RequestLease {
        link: old.link.clone(),
        req,
        owner,
    };
    // Queue an old write while the simulated data loop is not consuming it.
    old.data
        .as_ref()
        .unwrap()
        .send(frames::frame_ble_gatt("old write"))
        .unwrap();
    em::hub_restore_data(id, data, old.data.as_ref().unwrap());
    let mut claim = em::hub_claim_data(id).unwrap();
    assert!(
        claim.data_rx.try_recv().is_err(),
        "restoration drains old outbound frames"
    );
    assert!(rx.await.is_err(), "reset cancels old requests");
    let new = Arc::new(Capture::new(&ctx.registry, id, None, true).unwrap());
    assert_eq!(old.session, new.session);
    assert!(old.ctl.same_channel(&new.ctl));
    assert!(old
        .data
        .as_ref()
        .unwrap()
        .same_channel(new.data.as_ref().unwrap()));
    assert!(Arc::ptr_eq(&old.link, &new.link));
    assert_eq!(new.data_epoch, old.data_epoch.wrapping_add(1));
    // Deliberate req reuse proves old owner cleanup cannot remove new pending,
    // even on the same BleLink after reset (normal numbering still advances).
    let (tx, mut new_rx) = oneshot::channel();
    let owner = new.link.lock().unwrap().register(req, true, tx).unwrap();
    let new_lease = RequestLease {
        link: new.link.clone(),
        req,
        owner,
    };
    drop(old_lease);
    assert_eq!(new.link.lock().unwrap().pending.len(), 1);
    assert_eq!(
        execute(&old, json!({"t":"slots"}), GATT_WAIT).await,
        Err(BleError::DataOffline)
    );
    assert!(
        claim.data_rx.try_recv().is_err(),
        "old capture must not dispatch after fast reclaim"
    );
    assert!(new_rx.try_recv().is_err());
    drop(new_lease);
    let fresh = new.clone();
    let task = tokio::spawn(async move { execute(&fresh, json!({"t":"slots"}), GATT_WAIT).await });
    let frame = claim.data_rx.recv().await.unwrap();
    let sent: Value = serde_json::from_slice(&frame[1..]).unwrap();
    assert!(
        sent["req"].as_u64().unwrap() > req as u64,
        "reset preserves request numbering"
    );
    new.link.lock().unwrap().on_gatt(
        &serde_json::to_vec(&json!({"t":"slots","req":sent["req"],"free":3,"limit":3,"addrs":[]}))
            .unwrap(),
    );
    assert_eq!(task.await.unwrap().unwrap()["free"], 3);
    em::hub_remove(id);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn pending_limit_owner_token_and_reset_do_not_leak_or_alias() {
    let _g = HUB_LOCK.lock().unwrap();
    let id = "ble-http-limit";
    let (ctx, path) = context(id);
    let (_ctl, mut data) = online(id, true, true);
    let capture = Capture::new(&ctx.registry, id, None, true).unwrap();
    let mut receivers = Vec::new();
    for req in 1..=super::super::MAX_PENDING as u32 {
        let (tx, rx) = oneshot::channel();
        capture
            .link
            .lock()
            .unwrap()
            .register(req, true, tx)
            .unwrap();
        receivers.push(rx);
    }
    assert_eq!(
        execute(&capture, json!({"t":"slots"}), GATT_WAIT).await,
        Err(BleError::Busy)
    );
    assert!(data.try_recv().is_err());
    capture.link.lock().unwrap().reset();
    for receiver in receivers {
        assert!(receiver.await.is_err());
    }
    let (tx, _rx) = oneshot::channel();
    let owner = capture.link.lock().unwrap().register(1, true, tx).unwrap();
    let wrong = RequestLease {
        link: capture.link.clone(),
        req: 1,
        owner: uuid::Uuid::new_v4(),
    };
    drop(wrong);
    assert_eq!(capture.link.lock().unwrap().pending.len(), 1);
    let owned = RequestLease {
        link: capture.link.clone(),
        req: 1,
        owner,
    };
    drop(owned);
    assert!(capture.link.lock().unwrap().pending.is_empty());
    em::hub_remove(id);
    let _ = std::fs::remove_file(path);
}
