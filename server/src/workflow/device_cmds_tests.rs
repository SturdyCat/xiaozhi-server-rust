//! [`super::DeviceCmdHub`] 的注册 / 投递 / RAII 摘除行为。

use super::{DeviceCmd, DeviceCmdError, DeviceCmdHub};
use serde_json::json;
use tokio::sync::{mpsc, oneshot};

fn sample_cmd() -> DeviceCmd {
    let (reply, _rx) = oneshot::channel();
    DeviceCmd::CallDeviceTool {
        tool: "power".into(),
        arguments: json!({"state": "off"}),
        reply,
    }
}

#[test]
fn send_without_a_registered_channel_is_no_channel() {
    let hub = DeviceCmdHub::new();
    assert_eq!(
        hub.send("ghost", sample_cmd()),
        Err(DeviceCmdError::NoChannel)
    );
}

#[test]
fn dropped_guard_removes_the_channel() {
    let hub = DeviceCmdHub::new();
    let (tx, _rx) = mpsc::channel(4);
    let guard = hub.register("dev", tx);
    assert!(hub.send("dev", sample_cmd()).is_ok());
    drop(guard);
    assert_eq!(
        hub.send("dev", sample_cmd()),
        Err(DeviceCmdError::NoChannel)
    );
}

#[test]
fn re_register_replaces_and_the_old_guard_keeps_the_new_channel() {
    // 重连竞态：后注册覆盖先注册。旧守卫 Drop 时会把新通道一并摘掉——
    // 这是本实现的已知边界：注册表以 device_id 为键、守卫不校验所有权，
    // 会话串行（同设备同时只有一条），实际不会出现新旧并存。
    let hub = DeviceCmdHub::new();
    let (tx1, mut rx1) = mpsc::channel(4);
    let old = hub.register("dev", tx1);
    let (tx2, _rx2) = mpsc::channel(4);
    let _new = hub.register("dev", tx2);
    assert!(hub.send("dev", sample_cmd()).is_ok());
    assert!(rx1.try_recv().is_err(), "旧通道不应再收到命令");
    drop(old);
    assert_eq!(
        hub.send("dev", sample_cmd()),
        Err(DeviceCmdError::NoChannel)
    );
}

#[tokio::test]
async fn closed_receiver_reports_closed() {
    let hub = DeviceCmdHub::new();
    let (tx, rx) = mpsc::channel(4);
    let _guard = hub.register("dev", tx);
    drop(rx);
    assert_eq!(hub.send("dev", sample_cmd()), Err(DeviceCmdError::Closed));
}

#[tokio::test]
async fn full_channel_reports_busy() {
    let hub = DeviceCmdHub::new();
    let (tx, mut rx) = mpsc::channel(super::CMD_CHANNEL_CAPACITY);
    let _guard = hub.register("dev", tx);
    for _ in 0..super::CMD_CHANNEL_CAPACITY {
        assert!(hub.send("dev", sample_cmd()).is_ok());
    }
    assert_eq!(hub.send("dev", sample_cmd()), Err(DeviceCmdError::Busy));
    while rx.try_recv().is_ok() {}
}
