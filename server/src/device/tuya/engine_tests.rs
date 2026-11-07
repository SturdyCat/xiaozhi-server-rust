//! 在线轮询失败日志去重回归（真实 SQLite 临时目录，无网络、无 sleep）。
//!
//! 历史刷屏根因：listener DP 推送 / 云同步会把 DB 的 online 翻回 1，导致
//! `fails >= 2 && row.online` 每个轮询周期重新成立，“标记离线” INFO 整点
//! 重复。修复后同一离线事件只打一次，探测成功（reset_poll）复位。

use super::*;
use std::path::PathBuf;

fn temp_engine(tag: &str) -> (PathBuf, TuyaEngine) {
    let dir = std::env::temp_dir().join(format!(
        "xiaozhi-tuya-engine-test-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    (dir.clone(), TuyaEngine::new(&dir))
}

fn online_row(id: &str) -> DeviceRow {
    DeviceRow {
        device_id: id.to_string(),
        local_key: format!("key-{id}"),
        name: format!("设备 {id}"),
        model: "cz-1".into(),
        product_id: "prod".into(),
        ip: "192.168.1.50".into(),
        protocol_version: "3.3".into(),
        online: true,
        last_seen_ms: None,
        alias: String::new(),
        updated_at: now_ms(),
    }
}

fn db_online(engine: &TuyaEngine, id: &str) -> bool {
    engine
        .store
        .get_device(id)
        .expect("get_device")
        .expect("row exists")
        .online
}

#[test]
fn offline_log_fires_once_per_episode() {
    let (_dir, engine) = temp_engine("episode");
    engine
        .store
        .upsert_device(&online_row("dev-ep"))
        .expect("upsert");
    let row = online_row("dev-ep");

    // 第 1 次失败：不翻转、不打日志
    assert!(!engine.note_poll_failure(&row));
    assert!(db_online(&engine, "dev-ep"));

    // 第 2 次失败：离线事件开始 → 打日志 + 落 online=0
    assert!(engine.note_poll_failure(&row));
    assert!(!db_online(&engine, "dev-ep"));

    // 第 3 次失败：同一离线事件延续，不再打日志
    assert!(!engine.note_poll_failure(&row));

    // 模拟 listener 推送 / 云同步把 online 翻回 1：日志仍然保持安静
    engine.store.set_online("dev-ep", true, None).expect("flip");
    assert!(!engine.note_poll_failure(&row));

    // 探测成功 → reset_poll 复位；随后再次连续失败 = 新离线事件，再报一次
    engine.reset_poll("dev-ep");
    assert!(!engine.note_poll_failure(&row));
    assert!(engine.note_poll_failure(&row));
    assert!(!db_online(&engine, "dev-ep"));
}

#[test]
fn already_offline_row_never_logs() {
    let (_dir, engine) = temp_engine("already-offline");
    let mut row = online_row("dev-off");
    row.online = false; // DB 里已是离线（如服务重启后首轮）
    engine.store.upsert_device(&row).expect("upsert");

    // 状态没有变化（offline → offline），连续失败也不打“标记离线”日志
    assert!(!engine.note_poll_failure(&row));
    assert!(!engine.note_poll_failure(&row));
    assert!(!engine.note_poll_failure(&row));
    assert!(!db_online(&engine, "dev-off"));
}
