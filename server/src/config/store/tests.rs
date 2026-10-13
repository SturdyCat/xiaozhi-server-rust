//! `config/store.rs` 的回归测试（P0：存储抽象的**行为不变**口径）。
//!
//! 外移自内联块（`AGENTS.md` §5.10：测试一律独立文件）。
//! 覆盖：文件后端 persist↔load 往返（含 `normalize()` 生效）、不留 `.tmp` 残留、
//! 覆盖写已有文件、以及 `location() == None` 这条「不可持久化」契约
//! （各调用点靠它区分可写/不可写，**不能**靠 `load()` 报不报错判定）。

use super::*;
use crate::config::Config;

/// 临时目录（测试自清理；UUID 防并发互相踩）。
fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("xz_store_{tag}_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 文件后端：persist → load 往返，且旧写法在落盘时被 `normalize()` 折成规范形状。
#[test]
fn file_store_persist_roundtrip_normalizes() {
    let dir = temp_dir("roundtrip");
    let path = dir.join("config.toml");
    let cfg: Config = toml::from_str(
        r#"
[server]
port = 8123

[tts]
backend = "xfyun"

[tts.xfyun]
app_id = "a"
voice = "xiaoyan"
"#,
    )
    .unwrap();

    let store = FileConfigStore::new(path.to_str().unwrap());
    assert_eq!(store.location(), Some(path.to_str().unwrap()));
    store.persist(&cfg).unwrap();

    let loaded = store.load().unwrap();
    assert_eq!(loaded.server.port, 8123);
    assert_eq!(loaded.tts.xfyun.voice, "xiaoyan");
    // 落盘前 normalize：旧 `backend` 写成 `engine`，值不丢（与改动前 persist_config 同口径）
    assert_eq!(loaded.tts.engine, "xfyun");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 原子写不残留临时文件；覆盖写已有文件同样成立（管理页重复保存是常态）。
#[test]
fn file_store_persist_leaves_no_tmp_and_overwrites() {
    let dir = temp_dir("atomic");
    let path = dir.join("config.toml");
    let store = FileConfigStore::new(path.to_str().unwrap());

    let mut cfg = Config::default();
    cfg.server.port = 8000;
    store.persist(&cfg).unwrap();
    cfg.server.port = 9001;
    store.persist(&cfg).unwrap();

    let entries: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec!["config.toml".to_string()], "不应残留 .tmp");
    assert_eq!(store.load().unwrap().server.port, 9001);

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 内置默认启动：`location() == None` 是「不可持久化」的**唯一判据**，
/// 且两个错误都必须中文可行动（含"怎么修"）——UI 直接展示这些文案。
#[test]
fn default_store_is_not_persistable_with_actionable_errors() {
    let store = DefaultConfigStore;
    assert_eq!(store.location(), None, "无外部来源 → 不可持久化");

    let load_err = store.load().unwrap_err().to_string();
    assert!(
        load_err.contains("XIAOZHI_CONFIG") && load_err.contains("--config"),
        "读取错误要给出两种指定方式: {load_err}"
    );

    let persist_err = store.persist(&Config::default()).unwrap_err().to_string();
    assert!(
        persist_err.contains("无法持久化") && persist_err.contains("--config"),
        "写入错误要含原因与怎么修: {persist_err}"
    );
}
