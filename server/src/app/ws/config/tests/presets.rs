//! 配置预设（P1-5）的回归测试：save / apply / delete 进库 + apply 复用
//! `merge_client_patch` 部分更新语义。
//!
//! 从父模块 `tests.rs` 拆出（AGENTS.md §5.10 文件规模约定：行为不变的搬移）。
//! 使用父模块的公共 fixture（`super::*`）。

use super::*;

// ============================================================
// 预设管线（P1-5 R3-8）：save / apply / delete 进库 + apply 复用 merge_client_patch 语义
// ============================================================
//
// 说明：构造完整 `Engines`（会加载真实模型）在单测里过重，故直接驱动 handler 委托的纯函数
// （`SqliteConfigStore` 的 preset CRUD + `merge_client_patch` + `persist_config_file`），
// 验证「保存快照 → 列出 → 应用（部分更新写回全局）→ 删除」的入库往返与语义；
// axum handler 只是薄封装，由编译期类型检查 + 与既有端点同构（同一套 `put_config` 管线）保证。

fn preset_tmp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xz_preset_{}_{}", label, Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("创建临时预设测试目录");
    dir
}

#[test]
fn preset_save_and_list_roundtrip() {
    let dir = preset_tmp_dir("save_list");
    let (_db, store) = sqlite_store(&dir, "save_list");
    let current = Config::default();
    let name = "night";

    // 模拟 POST /api/config/presets：把当前配置快照存入 cfg_presets
    store.save_preset(name, &current).expect("保存预设");

    // 模拟 GET /api/config/presets：罗列预设名
    let names = store.list_presets().expect("列出预设");
    assert!(names.contains(&name.to_string()), "预设列表应包含 {name}");

    // 往返：读回预设应与当前配置一致（校验提及字段 + 一个未提及字段）
    let back = store.load_preset(name).expect("读预设").expect("预设存在");
    assert_eq!(back.llm.model, current.llm.model);
    assert_eq!(back.tts.speaker, current.tts.speaker);
    assert_eq!(back.tts.speed, current.tts.speed, "未提及字段保持默认");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preset_apply_reuses_merge_client_patch_semantics() {
    let dir = preset_tmp_dir("apply");
    let (db, store) = sqlite_store(&dir, "apply");
    let current = Config::default();

    // 先把基准配置落库（模拟运行中的全局配置）
    persist_config_file(db.to_str().unwrap(), &current).expect("写基准配置");

    // 写一个与全局不同的预设（仅提 llm.model 与 tts.speaker）
    let mut preset_cfg = Config::default();
    preset_cfg.llm.model = "preset-model".to_string();
    preset_cfg.tts.speaker = 9;
    store.save_preset("tune", &preset_cfg).expect("保存预设");
    let stored = store.load_preset("tune").expect("读预设").expect("预设存在");

    // 模拟 POST /api/config/presets/{name}/apply：复用 put_config 同款部分更新管线
    let preset_json = serde_json::to_value(&stored).expect("预设序列化");
    let applied = merge_client_patch(&current, &preset_json).expect("应用预设");
    persist_config_file(db.to_str().unwrap(), &applied).expect("写回全局配置");

    // 重新读库：被预设覆盖的字段生效，未提及字段保持基准（部分更新语义）
    let reloaded = store.load().expect("重读全局配置");
    assert_eq!(reloaded.llm.model, "preset-model", "apply 应覆盖 llm.model");
    assert_eq!(reloaded.tts.speaker, 9, "apply 应覆盖 tts.speaker");
    assert_eq!(reloaded.tts.speed, current.tts.speed, "apply 不应动未提及字段");
    assert_eq!(reloaded.llm.api_base, current.llm.api_base, "apply 不应动未提及字段");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preset_delete_removes_row() {
    let dir = preset_tmp_dir("delete");
    let (_db, store) = sqlite_store(&dir, "delete");
    let current = Config::default();
    let name = "todelete";
    store.save_preset(name, &current).expect("保存预设");
    assert!(store.list_presets().unwrap().contains(&name.to_string()));

    // 模拟 DELETE /api/config/presets/{name}
    store.delete_preset(name).expect("删除预设");
    assert!(!store.list_presets().unwrap().contains(&name.to_string()), "删除后预设应不存在");

    let _ = std::fs::remove_dir_all(&dir);
}
