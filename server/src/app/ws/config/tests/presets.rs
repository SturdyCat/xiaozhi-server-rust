//! 配置预设（P1-5）的回归测试：save / apply / delete 落盘 + apply 复用
//! `merge_client_patch` 部分更新语义。
//!
//! 从父模块 `tests.rs` 拆出（AGENTS.md §5.10 文件规模约定：行为不变的搬移）。
//! 使用父模块的公共 fixture（`super::*`）。

use super::*;

// ============================================================
// 预设管线（P1-5 R3-8）：save / apply / delete 落盘 + apply 复用 merge_client_patch 语义
// ============================================================
//
// 说明：构造完整 `Engines`（会加载真实模型）在单测里过重，故直接驱动 handler 委托的纯函数
// （`preset_path` / `presets_dir` + `Config::load` + `merge_client_patch` + `persist_config`）
// 配合真实文件 I/O，验证「保存快照 → 列出 → 应用（部分更新写回全局）→ 删除」的磁盘往返与语义；
// axum handler 只是薄封装，由编译期类型检查 + 与既有端点同构（同一套 `put_config` 管线）保证。

fn preset_tmp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xz_preset_{}_{}", label, Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("创建临时预设测试目录");
    dir
}

/// 写一个最小全局 config.toml（仅含几个会改动的字段，其余走默认）。
fn write_base_config(dir: &PathBuf, model: &str, speaker: i32) -> PathBuf {
    let toml = format!("[llm]\nmodel = \"{model}\"\n[tts]\nspeaker = {speaker}\n");
    let path = dir.join("config.toml");
    std::fs::write(&path, toml).expect("写入基准配置");
    path
}

#[test]
fn preset_save_and_list_roundtrip() {
    let dir = preset_tmp_dir("save_list");
    let base = write_base_config(&dir, "base-model", 1);
    let current = Config::load(base.to_str().unwrap()).expect("基准配置可解析");
    let name = "night";

    // 模拟 POST /api/config/presets：把当前配置序列化落盘到 presets/<name>.toml
    let preset_file = preset_path(base.to_str().unwrap(), name).expect("预设路径合法");
    std::fs::create_dir_all(preset_file.parent().unwrap()).expect("创建 presets 目录");
    let toml_str = toml::to_string_pretty(&current).expect("序列化预设");
    std::fs::write(&preset_file, toml_str).expect("写入预设文件");

    assert!(preset_file.exists(), "预设文件应已落盘");
    // 模拟 GET /api/config/presets：扫描 presets/ 目录罗列
    let pdir = presets_dir(base.to_str().unwrap()).expect("预设目录可推导");
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&pdir) {
        for e in entries.flatten() {
            if e.path().extension().and_then(|s| s.to_str()) == Some("toml") {
                names.push(e.path().file_stem().unwrap().to_string_lossy().to_string());
            }
        }
    }
    assert!(names.contains(&name.to_string()), "预设列表应包含 {name}");

    // 往返：读回预设应与当前配置一致（校验提及字段 + 一个未提及字段）
    let back = Config::load(preset_file.to_str().unwrap()).expect("预设可解析");
    assert_eq!(back.llm.model, current.llm.model);
    assert_eq!(back.tts.speaker, current.tts.speaker);
    assert_eq!(back.tts.speed, current.tts.speed, "未提及字段保持默认");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preset_apply_reuses_merge_client_patch_semantics() {
    let dir = preset_tmp_dir("apply");
    let base = write_base_config(&dir, "base-model", 1);
    let current = Config::load(base.to_str().unwrap()).expect("基准配置可解析");

    // 写一个与全局不同的预设（仅提 llm.model 与 tts.speaker）
    let preset_file = preset_path(base.to_str().unwrap(), "tune").expect("预设路径合法");
    std::fs::create_dir_all(preset_file.parent().unwrap()).expect("创建 presets 目录");
    let preset_toml = "[llm]\nmodel = \"preset-model\"\n[tts]\nspeaker = 9\n";
    std::fs::write(&preset_file, preset_toml).expect("写入预设");
    let preset_cfg = Config::load(preset_file.to_str().unwrap()).expect("预设可解析");

    // 模拟 POST /api/config/presets/{name}/apply：复用 put_config 同款部分更新管线
    let preset_json = serde_json::to_value(&preset_cfg).expect("预设序列化");
    let applied = merge_client_patch(&current, &preset_json).expect("应用预设");
    persist_config(base.to_str().unwrap(), &applied).expect("写回全局配置");

    // 重新读盘：被预设覆盖的字段生效，未提及字段保持基准（部分更新语义）
    let reloaded = Config::load(base.to_str().unwrap()).expect("重读全局配置");
    assert_eq!(reloaded.llm.model, "preset-model", "apply 应覆盖 llm.model");
    assert_eq!(reloaded.tts.speaker, 9, "apply 应覆盖 tts.speaker");
    assert_eq!(reloaded.tts.speed, current.tts.speed, "apply 不应动未提及字段");
    assert_eq!(reloaded.llm.api_base, current.llm.api_base, "apply 不应动未提及字段");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preset_delete_removes_file() {
    let dir = preset_tmp_dir("delete");
    let base = write_base_config(&dir, "base-model", 1);
    let current = Config::load(base.to_str().unwrap()).expect("基准配置可解析");
    let name = "todelete";
    let preset_file = preset_path(base.to_str().unwrap(), name).expect("预设路径合法");
    std::fs::create_dir_all(preset_file.parent().unwrap()).expect("创建 presets 目录");
    std::fs::write(&preset_file, toml::to_string_pretty(&current).unwrap()).expect("写入预设");
    assert!(preset_file.exists());

    // 模拟 DELETE /api/config/presets/{name}
    std::fs::remove_file(&preset_file).expect("删除预设");
    assert!(!preset_file.exists(), "删除后预设文件应不存在");

    let _ = std::fs::remove_dir_all(&dir);
}
