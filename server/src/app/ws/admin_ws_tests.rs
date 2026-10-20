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
    tokio::spawn(admin_ws_loop(in_rx, out_tx, ver_rx));

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
    tokio::spawn(admin_ws_loop(in_rx, out_tx, ver_rx));
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
    tokio::spawn(admin_ws_loop(in_rx, out_tx, ver_rx));
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
    tokio::spawn(admin_ws_loop(in_rx, out_tx, ver_rx));
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
