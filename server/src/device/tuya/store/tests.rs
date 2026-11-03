//! `store.rs` 单测：真实 SQLite（临时目录），无网络、无 sleep。

use super::*;
use std::path::PathBuf;

fn temp_store(tag: &str) -> (PathBuf, TuyaStore) {
    let dir = std::env::temp_dir().join(format!(
        "xiaozhi-tuya-store-test-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    (dir.clone(), TuyaStore::new(&dir))
}

fn sample(id: &str) -> DeviceRow {
    DeviceRow {
        device_id: id.to_string(),
        local_key: format!("key-{id}"),
        name: format!("设备 {id}"),
        model: "cz-1".into(),
        product_id: "prod".into(),
        ip: "192.168.1.50".into(),
        protocol_version: "3.3".into(),
        online: false,
        last_seen_ms: None,
        alias: String::new(),
        updated_at: now_ms(),
    }
}

#[test]
fn upsert_get_list_roundtrip() {
    let (_dir, store) = temp_store("roundtrip");
    store.upsert_device(&sample("dev-a")).expect("upsert a");
    store.upsert_device(&sample("dev-b")).expect("upsert b");

    let got = store.get_device("dev-a").expect("get").expect("exists");
    assert_eq!(got.local_key, "key-dev-a");
    assert_eq!(got.model, "cz-1");
    assert!(!got.online);

    let list = store.list_devices().expect("list");
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].device_id, "dev-a"); // 按 device_id 排序

    assert!(store.get_device("nope").expect("get missing").is_none());
}

#[test]
fn upsert_preserves_alias_and_prior_ip() {
    let (_dir, store) = temp_store("alias");
    store.upsert_device(&sample("dev")).expect("first upsert");
    store.set_alias("dev", "客厅插座").expect("alias");

    // 云同步再来一次：alias 是本地展示名，不该被云端空值冲掉
    let mut row = sample("dev");
    row.alias = String::new();
    row.ip = String::new(); // 云侧没有 ip → 保留旧值
    store.upsert_device(&row).expect("second upsert");

    let got = store.get_device("dev").expect("get").expect("exists");
    assert_eq!(got.alias, "客厅插座");
    assert_eq!(got.ip, "192.168.1.50");
}

#[test]
fn set_online_updates_last_seen_and_ip() {
    let (_dir, store) = temp_store("online");
    store.upsert_device(&sample("dev")).expect("upsert");
    store.set_online("dev", true, Some("192.168.1.99")).expect("online");

    let got = store.get_device("dev").expect("get").expect("exists");
    assert!(got.online);
    assert!(got.last_seen_ms.is_some());
    assert_eq!(got.ip, "192.168.1.99");

    store.set_online("dev", false, None).expect("offline");
    let got = store.get_device("dev").expect("get").expect("exists");
    assert!(!got.online);
    // 离线不清 last_seen、不改 ip
    assert!(got.last_seen_ms.is_some());
    assert_eq!(got.ip, "192.168.1.99");
}

#[test]
fn merge_discovery_only_updates_registered_devices() {
    let (_dir, store) = temp_store("discovery");
    store.upsert_device(&sample("dev")).expect("upsert");

    assert!(store
        .merge_discovery("dev", "192.168.1.77", "3.4")
        .expect("merge known"));
    let got = store.get_device("dev").expect("get").expect("exists");
    assert_eq!(got.ip, "192.168.1.77");
    assert_eq!(got.protocol_version, "3.4");

    // 未登记设备：不自动纳管（返回 false，目录无变化）
    assert!(!store
        .merge_discovery("ghost", "192.168.1.78", "3.3")
        .expect("merge unknown"));
    assert!(store.get_device("ghost").expect("get").is_none());
}

#[test]
fn sync_history_roundtrip() {
    let (_dir, store) = temp_store("history");
    assert!(store.last_sync().expect("empty").is_none());

    store
        .record_sync(&SyncRecord {
            started_at: 1_000,
            finished_at: 1_500,
            result: "ok".into(),
            device_count: 3,
            message: String::new(),
        })
        .expect("record 1");
    store
        .record_sync(&SyncRecord {
            started_at: 2_000,
            finished_at: 2_400,
            result: "partial".into(),
            device_count: 2,
            message: "第 2 页失败".into(),
        })
        .expect("record 2");

    let last = store.last_sync().expect("last").expect("exists");
    assert_eq!(last.result, "partial");
    assert_eq!(last.device_count, 2);
    assert_eq!(last.message, "第 2 页失败");
}

#[test]
fn db_file_permissions_are_owner_only() {
    let (dir, store) = temp_store("perms");
    store.list_devices().expect("open");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(dir.join("devices.db")).expect("meta");
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
