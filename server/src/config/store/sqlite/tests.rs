//! `SqliteConfigStore` 单元测试：round-trip、overlay/preset CRUD、
//! 只读查询护栏、备份、空库判定。

use std::sync::Arc;

use crate::config::store::sqlite::SqliteConfigStore;
use crate::config::store::ConfigStore;
use crate::config::Config;

/// 在临时文件上开一个配置库（测试间互不干扰）。
fn tmp_store(tag: &str) -> (Arc<SqliteConfigStore>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("xz_cfg_test_{tag}_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("cfg_{tag}.db"));
    let _ = std::fs::remove_file(&path);
    let store = SqliteConfigStore::open(path.to_str().unwrap()).unwrap();
    (store, path)
}

#[test]
fn load_persist_round_trip() {
    let (store, path) = tmp_store("roundtrip");
    let cfg = Config::default();
    store.persist(&cfg).unwrap();
    assert!(!store.is_empty());
    let loaded = store.load().unwrap();
    // 除运行期默认值外，关键字段应一致（这里仅验证可往返解析）。
    assert_eq!(loaded.server.port, cfg.server.port);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn persist_identical_content_twice_succeeds() {
    // 回归：cfg_revisions.revision 是内容指纹主键，内容不变的重复保存
    // （如密钥打码回传）不得撞 UNIQUE 约束导致保存失败。
    let (store, path) = tmp_store("dupsave");
    let cfg = Config::default();
    store.persist(&cfg).unwrap();
    store.persist(&cfg).unwrap(); // 第二次内容相同 → 历史表跳过，不报错
    let n: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM cfg_revisions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1, "同内容重复保存只应留一条历史");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn empty_store_loads_default() {
    let (store, path) = tmp_store("empty");
    assert!(store.is_empty());
    let loaded = store.load().unwrap();
    // 空库 load 返回内置默认（与文件后端"无文件→内置默认"语义一致）
    assert_eq!(loaded.server.port, Config::default().server.port);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn bootstrap_default_if_empty_idempotent() {
    let (store, path) = tmp_store("bootstrap");
    // 空库 → 首次自举写入默认配置（source="bootstrap"）
    assert!(store.bootstrap_default_if_empty().unwrap());
    assert!(!store.is_empty());
    let loaded = store.load().unwrap();
    assert_eq!(loaded.server.port, Config::default().server.port);
    // 非空库 → 幂等：不再写入，返回 false
    assert!(!store.bootstrap_default_if_empty().unwrap());
    // 自举留下的初始版本可追溯（cfg_revisions 有 source="bootstrap" 一行）
    let n: i64 = store
        .query("SELECT COUNT(*) AS n FROM cfg_revisions WHERE source = 'bootstrap'")
        .unwrap()
        .first()
        .and_then(|row| row.get("n").cloned())
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    assert_eq!(
        n, 1,
        "自举应在 cfg_revisions 留下 source=bootstrap 的初始版本"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn overlay_crud() {
    let (store, path) = tmp_store("overlay");
    store
        .save_overlay("dev-1", &serde_json::json!({"tts":{"speaker":3}}))
        .unwrap();
    let got = store.load_overlay("dev-1").unwrap().unwrap();
    assert_eq!(got["tts"]["speaker"], 3);
    // 覆盖写
    store
        .save_overlay("dev-1", &serde_json::json!({"tts":{"speaker":5}}))
        .unwrap();
    let got = store.load_overlay("dev-1").unwrap().unwrap();
    assert_eq!(got["tts"]["speaker"], 5);
    // 不存在的设备 → None
    assert!(store.load_overlay("nope").unwrap().is_none());
    // 删除
    store.delete_overlay("dev-1").unwrap();
    assert!(store.load_overlay("dev-1").unwrap().is_none());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn preset_crud() {
    let (store, path) = tmp_store("preset");
    let cfg = Config::default();
    store.save_preset("night", &cfg).unwrap();
    let names = store.list_presets().unwrap();
    assert_eq!(names, vec!["night".to_string()]);
    let loaded = store.load_preset("night").unwrap().unwrap();
    assert_eq!(loaded.server.port, cfg.server.port);
    // 覆盖写
    let mut cfg2 = Config::default();
    cfg2.server.port = 7777;
    store.save_preset("night", &cfg2).unwrap();
    let loaded = store.load_preset("night").unwrap().unwrap();
    assert_eq!(loaded.server.port, 7777);
    // 不存在 → None
    assert!(store.load_preset("missing").unwrap().is_none());
    // 删除
    store.delete_preset("night").unwrap();
    assert!(store.list_presets().unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn query_read_only_guard() {
    let (store, path) = tmp_store("query");
    // 必须先有数据
    store.persist(&Config::default()).unwrap();
    // 合法 SELECT
    let rows = store
        .query("SELECT id, revision FROM cfg_documents")
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], 1);
    // 多语句被拒
    assert!(store.query("SELECT 1; DROP TABLE cfg_documents").is_err());
    // 写入被拒
    assert!(store
        .query("UPDATE cfg_documents SET revision='x'")
        .is_err());
    // DDL 被拒
    assert!(store.query("CREATE TABLE evil(x INTEGER)").is_err());
    // 空串被拒
    assert!(store.query("  ").is_err());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn backup_produces_file() {
    let (store, path) = tmp_store("backup");
    store.persist(&Config::default()).unwrap();
    let backup = path.with_file_name("cfg_backup_test.db");
    let _ = std::fs::remove_file(&backup);
    store.backup(backup.to_str().unwrap()).unwrap();
    assert!(backup.exists());
    // 备份文件可被再次打开并读到数据
    let reopened = SqliteConfigStore::open(backup.to_str().unwrap()).unwrap();
    let rows = reopened.query("SELECT id FROM cfg_documents").unwrap();
    assert_eq!(rows.len(), 1);
    let _ = std::fs::remove_file(&backup);
    let _ = std::fs::remove_file(&path);
}
