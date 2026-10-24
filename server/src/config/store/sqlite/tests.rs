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

// ---- 存量配置默认值对齐（config/defaults.rs）----

/// 造一份"旧版本落库"的文档：标准行为全为旧默认（false），版本记为 0。
fn insert_legacy_doc(store: &SqliteConfigStore, mutate: impl FnOnce(&mut Config)) {
    let mut cfg = Config::default();
    cfg.wakeup.enabled = false;
    cfg.command.enabled = false;
    cfg.nlu.enabled = false;
    cfg.soul.enabled = false;
    cfg.memory.enabled = false;
    cfg.memory.recall_enabled = false;
    cfg.memory.extraction_enabled = false;
    cfg.echomuse.enabled = false;
    cfg.llm.enabled = false;
    mutate(&mut cfg);
    let doc = serde_json::to_string(&cfg).unwrap();
    store
        .conn()
        .execute(
            "INSERT INTO cfg_documents (id, doc, revision, updated_at, updated_by, defaults_version)
             VALUES (1, ?1, 'legacy-rev', 0, 'test', 0)
             ON CONFLICT(id) DO UPDATE SET doc=excluded.doc",
            rusqlite::params![doc],
        )
        .unwrap();
}

#[test]
fn align_defaults_migrates_legacy_document() {
    let (store, path) = tmp_store("align_legacy");
    insert_legacy_doc(&store, |_| {});
    let report = store.align_defaults().unwrap().expect("旧库必须被对齐");
    assert_eq!(report.from_version, 0);
    assert_eq!(
        report.changed.len(),
        crate::config::defaults::STANDARD_BEHAVIOR_ON.len()
    );
    let loaded = store.load().unwrap();
    assert!(loaded.wakeup.enabled, "唤醒问候语必须被打开");
    assert_eq!(loaded.wakeup.greeting, "我在");
    assert!(loaded.command.enabled && loaded.nlu.enabled);
    assert!(loaded.memory.enabled && loaded.memory.recall_enabled);
    assert!(loaded.echomuse.enabled && loaded.llm.enabled);
    // 版本已推进 + 留了一条可回滚的历史
    let ver: i64 = store
        .conn()
        .query_row(
            "SELECT defaults_version FROM cfg_documents WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        ver as u32,
        crate::config::defaults::CURRENT_DEFAULTS_VERSION
    );
    let n: i64 = store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM cfg_revisions WHERE source = 'defaults-migration'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "对齐必须留 history（可用 revision 回滚）");
    // 幂等：第二次不再动库
    assert!(store.align_defaults().unwrap().is_none());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn align_defaults_keeps_user_customized_switches() {
    let (store, path) = tmp_store("align_keep");
    // 用户当年手动打开过技能级联（true ≠ 旧默认 false）→ 必须原样保留
    insert_legacy_doc(&store, |cfg| cfg.nlu.enabled = true);
    let report = store.align_defaults().unwrap().expect("仍需对齐其余项");
    assert!(
        report.kept.contains(&"nlu.enabled".to_string()),
        "{}",
        report.summary()
    );
    let loaded = store.load().unwrap();
    assert!(loaded.nlu.enabled);
    assert!(loaded.wakeup.enabled, "没动过的项照常对齐");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn align_defaults_noop_on_fresh_bootstrap() {
    let (store, path) = tmp_store("align_fresh");
    assert!(store.bootstrap_default_if_empty().unwrap());
    // 自举写入的就是当前默认值 → 对齐无事可做（不重复写库）
    assert!(store.align_defaults().unwrap().is_none());
    let _ = std::fs::remove_file(&path);
}

/// 老库（只有 m1、没有 `defaults_version` 列）→ 打开时自动补列并完成对齐。
#[test]
fn legacy_db_without_column_is_migrated() {
    let dir = std::env::temp_dir().join(format!("xz_cfg_test_m1_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("legacy.db");
    let _ = std::fs::remove_file(&path);
    {
        // 手工造一个"发布版 m1 库"：建表 + 记 version=1 + 写旧默认文档
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE _cfg_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);
             CREATE TABLE cfg_documents (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 doc TEXT NOT NULL, revision TEXT NOT NULL,
                 updated_at INTEGER NOT NULL, updated_by TEXT
             );
             CREATE TABLE cfg_revisions (
                 revision TEXT PRIMARY KEY, doc TEXT NOT NULL, created_at INTEGER NOT NULL,
                 source TEXT NOT NULL, author TEXT, note TEXT
             );
             INSERT INTO _cfg_migrations (version, applied_at) VALUES (1, 0);",
        )
        .unwrap();
        let mut cfg = Config::default();
        cfg.wakeup.enabled = false;
        cfg.command.enabled = false;
        let doc = serde_json::to_string(&cfg).unwrap();
        conn.execute(
            "INSERT INTO cfg_documents (id, doc, revision, updated_at, updated_by)
             VALUES (1, ?1, 'legacy', 0, 'bootstrap')",
            rusqlite::params![doc],
        )
        .unwrap();
    }
    let store = SqliteConfigStore::open(path.to_str().unwrap()).unwrap();
    let report = store.align_defaults().unwrap().expect("老库必须被对齐");
    assert!(report.changed.iter().any(|c| c.path == "wakeup.enabled"));
    let loaded = store.load().unwrap();
    assert!(loaded.wakeup.enabled && loaded.command.enabled);
    let _ = std::fs::remove_file(&path);
}
