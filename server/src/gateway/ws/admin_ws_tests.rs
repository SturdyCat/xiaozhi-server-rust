//! 管理台实时通道（`GET /api/admin/ws`）的**外置测试**：帧形状与
//! [`admin_ws_loop`] 纯逻辑 mpsc/watch 直驱，不起真实 HTTP/WS
//! （挂法照 `echomuse::admin_tests` 的外置风格）。

use std::time::Duration;

use super::admin_ws::{admin_ws_loop, event_frame, hello_frame, reply_for, WsIn};

/// 读一帧（带超时防挂死），返回解析后的 JSON。
async fn recv_frame(rx: &mut tokio::sync::mpsc::Receiver<String>) -> serde_json::Value {
    let frame = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("等帧超时")
        .expect("out 通道被对端关闭");
    serde_json::from_str(&frame).expect("帧必须是 JSON 文本")
}

// ---- 帧形状 ----

#[test]
fn hello_and_event_frames_carry_type_and_version() {
    let hello: serde_json::Value = serde_json::from_str(&hello_frame(7)).unwrap();
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["version"], 7);
    let event: serde_json::Value = serde_json::from_str(&event_frame(9)).unwrap();
    assert_eq!(event["type"], "echomuse_devices_changed");
    assert_eq!(event["version"], 9);
}

// ---- reply_for：ping / 未知 / 非法 ----

#[test]
fn reply_for_answers_ping_with_pong() {
    let reply: serde_json::Value =
        serde_json::from_str(&reply_for("{\"type\":\"ping\"}").unwrap()).unwrap();
    assert_eq!(reply["type"], "pong");
}

#[test]
fn reply_for_rejects_unknown_type_invalid_json_and_non_object() {
    // 未知 type → error 帧（连接保持，由循环语义保证，这里只看回执形状）。
    let reply: serde_json::Value =
        serde_json::from_str(&reply_for("{\"type\":\"nope\"}").unwrap()).unwrap();
    assert_eq!(reply["type"], "error");
    assert!(reply["error"].is_string());
    // 非 JSON / 非对象 / 缺 type → 同样 error，且都说得出原因。
    for bad in ["not json", "[1,2]", "{\"x\":1}"] {
        let reply: serde_json::Value = serde_json::from_str(&reply_for(bad).unwrap()).unwrap();
        assert_eq!(reply["type"], "error", "bad={bad}");
        assert!(reply["error"].is_string(), "bad={bad}");
    }
}

// ---- admin_ws_loop（mpsc/watch 直驱） ----

#[tokio::test]
async fn loop_sends_hello_then_event_on_version_bump() {
    let (ver_tx, ver_rx) = tokio::sync::watch::channel(5u64);
    let (_in_tx, in_rx) = tokio::sync::mpsc::channel::<WsIn>(16);
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<String>(16);
    tokio::spawn(admin_ws_loop(in_rx, out_tx, ver_rx, None));

    // 连接建立后立刻 hello（当前版本）。
    let hello = recv_frame(&mut out_rx).await;
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["version"], 5);

    // 版本 bump → 推事件帧（新版本）。
    ver_tx.send_replace(6);
    let event = recv_frame(&mut out_rx).await;
    assert_eq!(event["type"], "echomuse_devices_changed");
    assert_eq!(event["version"], 6);
}

#[tokio::test]
async fn loop_replies_ping_with_pong_and_exits_on_closed() {
    let (_ver_tx, ver_rx) = tokio::sync::watch::channel(0u64);
    let (in_tx, in_rx) = tokio::sync::mpsc::channel::<WsIn>(16);
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<String>(16);
    tokio::spawn(admin_ws_loop(in_rx, out_tx, ver_rx, None));
    assert_eq!(recv_frame(&mut out_rx).await["type"], "hello");

    // 应用层心跳：ping → pong。
    in_tx
        .send(WsIn::Text("{\"type\":\"ping\"}".into()))
        .await
        .unwrap();
    assert_eq!(recv_frame(&mut out_rx).await["type"], "pong");

    // Closed → 循环退出（其持有的 out 发送端随任务结束被丢弃，out 通道取尽关闭）。
    in_tx.send(WsIn::Closed).await.unwrap();
    let gone = tokio::time::timeout(Duration::from_secs(2), out_rx.recv()).await;
    assert!(
        matches!(gone, Ok(None)),
        "循环退出后 out 应关闭，实际 {gone:?}"
    );
}

#[tokio::test]
async fn loop_stays_idle_without_changes() {
    let (ver_tx, ver_rx) = tokio::sync::watch::channel(3u64);
    let (_in_tx, in_rx) = tokio::sync::mpsc::channel::<WsIn>(16);
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<String>(16);
    tokio::spawn(admin_ws_loop(in_rx, out_tx, ver_rx, None));
    assert_eq!(recv_frame(&mut out_rx).await["type"], "hello");

    // 无变化时挂住不发帧（短超时探空，hello 之后不应有任何帧）。
    let idle = tokio::time::timeout(Duration::from_millis(50), out_rx.recv()).await;
    assert!(idle.is_err(), "无变化不应有帧，实际 {idle:?}");

    // 之后的变化照常推送（证明只是空闲而非挂死）。
    ver_tx.send_replace(4);
    assert_eq!(recv_frame(&mut out_rx).await["version"], 4);
}

#[tokio::test]
async fn loop_exits_when_subscriber_goes_away() {
    let (_ver_tx, ver_rx) = tokio::sync::watch::channel(0u64);
    let (in_tx, in_rx) = tokio::sync::mpsc::channel::<WsIn>(16);
    let (out_tx, out_rx) = tokio::sync::mpsc::channel::<String>(16);
    tokio::spawn(admin_ws_loop(in_rx, out_tx, ver_rx, None));
    // 丢弃订阅方：循环下一次回执（ping → pong）发送失败即退出，
    // in_rx 随任务结束被丢弃 → in_tx.send 报错即循环已退出的可观测信号。
    drop(out_rx);
    let mut exited = false;
    for _ in 0..100 {
        if in_tx
            .send(WsIn::Text("{\"type\":\"ping\"}".into()))
            .await
            .is_err()
        {
            exited = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(exited, "out 订阅方消失后循环应退出");
}

// ---- RPC 帧：parse_rpc_frame（纯函数校验） ----

use super::admin_ws::{execute_rpc, parse_rpc_frame};
use axum::routing::get as axum_get;
use serde_json::json;

#[test]
fn parse_rpc_frame_accepts_get_and_post_with_body() {
    let (_, timeout, req) = parse_rpc_frame(&json!({
        "type": "rpc", "id": 7, "method": "GET", "path": "/api/stats"
    }))
    .expect("GET 帧应合法");
    assert!(timeout.is_none(), "未带 timeout 字段应回退默认");
    assert_eq!(req.method(), "GET");
    assert_eq!(req.uri().path(), "/api/stats");

    let (_, timeout, req) = parse_rpc_frame(&json!({
        "type": "rpc", "id": 8, "method": "POST", "path": "/api/config",
        "body": {"asr": {"enabled": true}}
    }))
    .expect("POST 帧应合法");
    assert!(timeout.is_none());
    assert_eq!(req.method(), "POST");
}

#[test]
fn parse_rpc_frame_timeout_clamped() {
    // 45s（NLU 模型测试场景）原样通过。
    let (_, timeout, _) = parse_rpc_frame(&json!({
        "type": "rpc", "id": 1, "method": "GET", "path": "/api/x", "timeout": 45
    }))
    .expect("45s 应合法");
    assert_eq!(timeout, Some(45));
    // 超上限钳到 120s；0 钳到 1s；非整数拒绝。
    let (_, timeout, _) = parse_rpc_frame(&json!({
        "type": "rpc", "id": 1, "method": "GET", "path": "/api/x", "timeout": 9999
    }))
    .unwrap();
    assert_eq!(timeout, Some(120));
    let (_, timeout, _) = parse_rpc_frame(&json!({
        "type": "rpc", "id": 1, "method": "GET", "path": "/api/x", "timeout": 0
    }))
    .unwrap();
    assert_eq!(timeout, Some(1));
    assert!(parse_rpc_frame(&json!({
        "type": "rpc", "id": 1, "method": "GET", "path": "/api/x", "timeout": "fast"
    }))
    .is_err());
}

#[test]
fn parse_rpc_frame_rejects_bad_frames() {
    let cases = [
        json!({"type": "nope", "id": 1, "method": "GET", "path": "/api/x"}),
        json!({"type": "rpc", "method": "GET", "path": "/api/x"}),           // 缺 id
        json!({"type": "rpc", "id": 1.5, "method": "GET", "path": "/api/x"}), // id 非整数
        json!({"type": "rpc", "id": 1, "method": "PATCH", "path": "/api/x"}), // 方法不在白名单
        json!({"type": "rpc", "id": 1, "method": "GET", "path": "/static/x"}), // 非 /api/
        json!({"type": "rpc", "id": 1, "method": "GET", "path": "/api/admin/ws"}), // 递归拒绝
        json!({"type": "rpc", "id": 1, "method": "GET", "path": "/api/x", "body": {}}), // GET 带 body
        json!({"type": "rpc", "id": 1, "method": "POST", "path": "/api/x"}),  // 写缺 body
        json!({"type": "rpc", "id": 1, "method": "POST", "path": "/api/x", "body": [1]}), // body 非对象
    ];
    for bad in &cases {
        assert!(parse_rpc_frame(bad).is_err(), "应拒绝: {bad}");
    }
}

// ---- RPC 执行：oneshot 直驱 mini Router（不起 HTTP） ----

/// 带超时读一帧并解析为 JSON（沿用本文件 recv_frame 语义）。
async fn recv_rpc(rx: &mut tokio::sync::mpsc::Receiver<String>) -> serde_json::Value {
    recv_frame(rx).await
}

#[tokio::test]
async fn rpc_executes_through_router_and_returns_status_and_body() {
    // mini Router：GET /api/echo 回 JSON；POST /api/echo 回显 body。
    let router = axum::Router::new()
        .route(
            "/api/echo",
            axum_get(|| async {
                axum::Json(json!({"ok": true, "from": "get"}))
            })
            .post(|body: axum::Json<serde_json::Value>| async move {
                axum::Json(json!({"ok": true, "echoed": body.0}))
            }),
        )
        .with_state(());
    let router = std::sync::Arc::new(router);

    // GET：命中 handler，status 200 + body 原样。
    let frame = json!({"type": "rpc", "id": 1, "method": "GET", "path": "/api/echo"});
    let reply: serde_json::Value =
        serde_json::from_str(&execute_rpc(router.clone(), &frame).await).unwrap();
    assert_eq!(reply["type"], "rpc_result");
    assert_eq!(reply["id"], 1);
    assert_eq!(reply["status"], 200);
    assert_eq!(reply["body"]["from"], "get");

    // POST：body 进 handler。
    let frame = json!({
        "type": "rpc", "id": 2, "method": "POST", "path": "/api/echo",
        "body": {"x": 1}
    });
    let reply: serde_json::Value =
        serde_json::from_str(&execute_rpc(router, &frame).await).unwrap();
    assert_eq!(reply["status"], 200);
    assert_eq!(reply["body"]["echoed"]["x"], 1);
}

#[tokio::test]
async fn rpc_maps_http_404_and_frame_errors_to_result_frames() {
    let router = std::sync::Arc::new(axum::Router::new().with_state(()));

    // 路由不存在 → axum 默认 404（空 body）→ status 原样透传、body 置 null。
    let frame = json!({"type": "rpc", "id": 3, "method": "GET", "path": "/api/none"});
    let reply: serde_json::Value =
        serde_json::from_str(&execute_rpc(router.clone(), &frame).await).unwrap();
    assert_eq!(reply["type"], "rpc_result");
    assert_eq!(reply["status"], 404);
    assert!(reply["body"].is_null());

    // 帧非法 → status 0 + 原因，id 尽量回带。
    let frame = json!({"type": "rpc", "id": 4, "method": "BREW", "path": "/api/none"});
    let reply: serde_json::Value =
        serde_json::from_str(&execute_rpc(router, &frame).await).unwrap();
    assert_eq!(reply["status"], 0);
    assert_eq!(reply["id"], 4);
    assert!(reply["error"].is_string());
}

#[tokio::test]
async fn loop_without_rpc_router_answers_status_zero() {
    let (_ver_tx, ver_rx) = tokio::sync::watch::channel(0u64);
    let (in_tx, in_rx) = tokio::sync::mpsc::channel::<WsIn>(16);
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<String>(64);
    tokio::spawn(admin_ws_loop(in_rx, out_tx, ver_rx, None));
    assert_eq!(recv_frame(&mut out_rx).await["type"], "hello");

    in_tx
        .send(WsIn::Text(
            "{\"type\":\"rpc\",\"id\":9,\"method\":\"GET\",\"path\":\"/api/stats\"}".into(),
        ))
        .await
        .unwrap();
    let reply = recv_rpc(&mut out_rx).await;
    assert_eq!(reply["type"], "rpc_result");
    assert_eq!(reply["id"], 9);
    assert_eq!(reply["status"], 0);
    assert!(reply["error"].is_string());
}
