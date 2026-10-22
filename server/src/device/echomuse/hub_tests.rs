//! EchoMuse hub 认领/归还语义测试（channel 直驱，不起真实 WebSocket）。

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use super::ble::BleLink;
use super::{
    hub_claim_data, hub_data, hub_insert, hub_remove, hub_restore_data, ControlOut, PendingDevice,
};
use crate::workflow::protocol::IncomingFrame;

/// hub 是进程级单例，涉及它的测试串行执行。
pub(super) static HUB_LOCK: Mutex<()> = Mutex::new(());

/// 登记一台测试设备，返回 control 出站接收端（drop 即会话收尾语义）。
pub(super) fn insert_online(device_id: &str) -> mpsc::UnboundedReceiver<ControlOut> {
    let (up_tx, _up_rx) = mpsc::unbounded_channel::<IncomingFrame>();
    let (ctl_tx, ctl_rx) = mpsc::unbounded_channel::<ControlOut>();
    let (data_tx, data_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    hub_insert(
        device_id,
        PendingDevice {
            up_tx,
            data_rx: Some(data_rx),
            since: std::time::Instant::now(),
            ctl_tx: Some(ctl_tx),
            data_tx: Some(data_tx),
            ble: Arc::new(Mutex::new(BleLink::default())),
            management: Arc::new(super::runtime::LiveControl::new(None, false)),
        },
    );
    ctl_rx
}

/// data 认领后条目保留（设备完全在线期间管理面 hub_ctl/hub_data/hub_ble 仍可用），
/// 且认领期间不重复发放。
#[test]
fn hub_claim_keeps_entry_and_is_once() {
    use super::{hub_ble, hub_ctl};
    let _g = HUB_LOCK.lock().unwrap();
    let ctl_rx = insert_online("dev-claim-1");
    // 首次认领成功；条目仍在：ctl/data 出站口与 ble 链路可继续访问。
    let claim = hub_claim_data("dev-claim-1").expect("首次认领应成功");
    drop(claim.data_rx);
    assert!(hub_ctl("dev-claim-1").is_some());
    assert!(hub_data("dev-claim-1").is_some());
    assert!(hub_ble("dev-claim-1", |l| l.alloc_req()).is_some());
    // 认领期间二次认领失败（接收端已被取走）。
    assert!(hub_claim_data("dev-claim-1").is_none());
    // 不存在的设备认领失败。
    assert!(hub_claim_data("dev-claim-ghost").is_none());
    hub_remove("dev-claim-1");
    assert!(hub_ctl("dev-claim-1").is_none());
    assert!(hub_data("dev-claim-1").is_none());
    drop(ctl_rx);
}

/// data 任务退出归还认领：同会话重拨立即可再认领；条目已被新 register 替换
/// （`data_tx` 不同 channel）时拒绝归还，保护新会话的全新交接件不被旧认领顶掉。
#[test]
fn hub_restore_data_reclaims_same_session_only() {
    let _g = HUB_LOCK.lock().unwrap();
    let ctl_rx = insert_online("dev-restore-1");
    let claim = hub_claim_data("dev-restore-1").expect("首次认领应成功");
    assert!(hub_claim_data("dev-restore-1").is_none());
    // 归还（探针同 channel）→ 设备重拨立即再认领成功。
    hub_restore_data(&claim.device_id, claim.data_rx, &claim.data_tx);
    let again = hub_claim_data("dev-restore-1").expect("归还后应可再认领");
    // 条目被新 register 替换（同 id 重登记，全新 channel）后旧认领归还 → 拒绝。
    let ctl_rx2 = insert_online("dev-restore-1");
    hub_restore_data(&again.device_id, again.data_rx, &again.data_tx);
    // 新条目自己的 data_rx 应原封不动：经 hub_data 下发的标记帧能从再认领里收到。
    hub_data("dev-restore-1")
        .expect("新条目 data_tx 应在")
        .send(b"fresh".to_vec())
        .unwrap();
    let mut fresh = hub_claim_data("dev-restore-1").expect("替换后的新条目应可认领");
    assert_eq!(fresh.data_rx.try_recv().unwrap(), b"fresh".to_vec());
    hub_remove("dev-restore-1");
    drop(ctl_rx);
    drop(ctl_rx2);
}

#[test]
fn hub_restore_discards_queued_frames_before_reclaim() {
    let _g = HUB_LOCK.lock().unwrap();
    let id = "dev-data-no-replay";
    let ctl_rx = insert_online(id);
    let claim = hub_claim_data(id).unwrap();
    claim.data_tx.send(super::frames::frame_ble_gatt(
        r#"{"t":"write","req":1,"addr":"aa:bb:cc:dd:ee:ff","handle":3,"value":"AQ==","response":true}"#,
    )).unwrap();
    claim.data_tx.send(b"stale audio".to_vec()).unwrap();
    hub_restore_data(id, claim.data_rx, &claim.data_tx);
    let mut fresh = hub_claim_data(id).unwrap();
    assert!(
        fresh.data_rx.try_recv().is_err(),
        "old frames must not replay on a new data socket"
    );
    fresh.data_tx.send(b"new audio".to_vec()).unwrap();
    assert_eq!(fresh.data_rx.try_recv().unwrap(), b"new audio");
    hub_remove(id);
    drop(ctl_rx);
}

#[tokio::test]
async fn owning_data_restore_rearms_capture_but_old_restore_cannot_cancel_new_control() {
    use crate::workflow::protocol::UtteranceEnd;
    let _guard = HUB_LOCK.lock().unwrap();
    let id = "dev-turn-data-reconnect";
    let _old_ctl = insert_online(id);
    let old = hub_claim_data(id).unwrap();
    let (up, mut rx) = mpsc::unbounded_channel();
    // Use the same captured LiveControl that production data_loop uses.
    let mut sink = futures_util::sink::drain::<axum::extract::ws::Message>();
    super::turn::start_turn(&old.management.turn, &mut sink, &up, "action 键")
        .await
        .unwrap();
    rx.try_recv().unwrap();
    rx.try_recv().unwrap();
    hub_restore_data(id, old.data_rx, &old.data_tx);
    let again = hub_claim_data(id).unwrap();
    assert!(Arc::ptr_eq(&old.management, &again.management));
    super::turn::start_turn(&again.management.turn, &mut sink, &up, "action 键")
        .await
        .unwrap();
    assert!(matches!(rx.try_recv().unwrap(), IncomingFrame::Text(t) if t.contains("detect")));
    rx.try_recv().unwrap();

    let _new_ctl = insert_online(id);
    let fresh = hub_claim_data(id).unwrap();
    assert!(!Arc::ptr_eq(&again.management, &fresh.management));
    super::turn::start_turn(&fresh.management.turn, &mut sink, &up, "action 键")
        .await
        .unwrap();
    rx.try_recv().unwrap();
    rx.try_recv().unwrap();
    // Stale data claim cannot clear replacement's turn or return its receiver.
    hub_restore_data(id, again.data_rx, &again.data_tx);
    super::turn::start_turn(&fresh.management.turn, &mut sink, &up, "action 键")
        .await
        .unwrap();
    assert!(rx.try_recv().is_err());
    fresh
        .management
        .turn
        .finish(UtteranceEnd::Speech, &up, |_| {});
    assert!(matches!(
        rx.try_recv().unwrap(),
        IncomingFrame::EndOfUtterance(UtteranceEnd::Speech)
    ));
    hub_remove(id);
}
