//! 每设备配置 overlay —— 白名单字段的「全局配置 ⊕ 设备增量」合并。
//!
//! 设计要点（见 `docs/p3-round3-design.md`）：
//! - overlay **只含会话内参数**（tts 音色/语速、llm 提示词与历史长度、soul 人设、
//!   wakeup/command 话术、memory 召回参数）；结构型参数（server/audio/aiui）一律不可覆盖。
//! - overlay **永不触碰引擎签名**：白名单字段都不在任一引擎的 `signature` 闭包里，
//!   因此叠加 overlay 不会触发 TTS/LLM/Memory 的共享引擎重建（引擎仍按全局配置构建）。
//! - overlay/预设现均存于 SQLite 配置库（`cfg_device_overlays` / `cfg_presets`，见
//!   `config/store/sqlite.rs`）；旧的 TOML 文件落点已随 `FileConfigStore` 一并移除。
//! - 本模块**不依赖引擎**，纯配置层；`engine.rs` 与 `app/ws` 都来复用它。

use serde_json::Value;

use crate::config::Config;

/// overlay 白名单：允许覆盖的字段路径。
///
/// 保守口径（用户拍板「不含 engine 切换」）：只放会话内、且不影响共享引擎签名的字段。
/// ⚠️ `tts.kokoro.lang` 故意**不在**表中——Kokoro 的 `lang` 烤死在共享引擎池
/// （`plugins/tts/kokoro.rs`），逐设备覆盖须重建共享引擎，违反「共享引擎只按全局构建」契约。
/// ⚠️ `memory.engine`/`enabled`/`db_path`/`extractor`/`rag`/`retention`/`scope_prefix` 等
/// 决定引擎构建或共享存储行为，全部排除。
pub(crate) fn overlay_allowed(path: &[&str]) -> bool {
    match path.first().copied() {
        Some("tts") => matches!(
            path,
            ["tts", "speaker"] | ["tts", "speed"] | ["tts", "xfyun", "voice"]
        ),
        Some("llm") => matches!(path, ["llm", "system_prompt"] | ["llm", "max_history"]),
        // 整段允许（人设仅影响 prompt 渲染，无引擎影响）
        Some("soul") => true,
        // 整段允许（仅 greeting 话术）
        Some("wakeup") => true,
        // 整段允许（仅 keywords/reply）
        Some("command") => true,
        Some("memory") => matches!(
            path,
            ["memory", "recall_enabled"]
                | ["memory", "fresh_turn_count"]
                | ["memory", "recall_max_nodes"]
                | ["memory", "recall_max_tokens"]
                | ["memory", "recall_budget_ms"]
                | ["memory", "inject_position"]
        ),
        _ => false,
    }
}

/// 校验 overlay JSON：任一越界字段即返回「怎么修」式中文错误（含越界路径）。
///
/// `PUT /api/devices/{id}/overlay` 用它**拒绝**越界写入；读盘侧用 [`prune_disallowed`]
/// 做防御性裁剪（磁盘被手改也不让越界项悄悄生效，同时不连坐整台设备）。
pub(crate) fn validate_overlay(value: &Value) -> Result<(), String> {
    if !value.is_object() {
        return Err("overlay 必须是对象（如 {\"tts\":{\"speaker\":2}}）".to_string());
    }
    let mut err: Option<String> = None;
    validate_node(value, &mut Vec::new(), &mut err);
    match err {
        None => Ok(()),
        Some(field) => Err(format!(
            "字段 {field} 不允许在设备 overlay 中覆盖（只可覆盖会话内参数：tts 音色/语速、\
             llm 提示词与历史长度、soul 人设、wakeup/command 话术、memory 召回参数；\
             结构型参数 server/audio 等不可覆盖）。请用 GET /api/config 修改全局配置。"
        )),
    }
}

fn validate_node(value: &Value, path: &mut Vec<String>, err: &mut Option<String>) {
    if let Value::Object(obj) = value {
        for (k, v) in obj {
            path.push(k.clone());
            if v.is_object() {
                validate_node(v, path, err);
            } else {
                let p: Vec<&str> = path.iter().map(|s| s.as_str()).collect();
                if !overlay_allowed(&p) && err.is_none() {
                    *err = Some(p.join("."));
                }
            }
            path.pop();
        }
    }
}

/// 防御性裁剪：原地删除所有越界叶子（含因此其变空的父对象）。
///
/// 读盘侧调用——磁盘 overlay 若被手改出越界字段，只丢该字段并告警，不让整台设备回退全局。
pub(crate) fn prune_disallowed(value: &mut Value) {
    let mut path = Vec::new();
    prune_node(value, &mut path);
}

fn prune_node(value: &mut Value, path: &mut Vec<String>) {
    if let Value::Object(obj) = value {
        let keys: Vec<String> = obj.keys().cloned().collect();
        for k in keys {
            path.push(k.clone());
            let v = obj.get_mut(&k).unwrap();
            if v.is_object() {
                prune_node(v, path);
                if v.as_object().is_some_and(|o| o.is_empty()) {
                    obj.remove(&k);
                }
            } else {
                let p: Vec<&str> = path.iter().map(|s| s.as_str()).collect();
                if !overlay_allowed(&p) {
                    obj.remove(&k);
                }
            }
            path.pop();
        }
    }
}

/// 全局配置 ⊕ overlay → 生效配置。
///
/// overlay 只覆盖它**实际含有**的字段（JSON 层递归合并，与 `merge_client_patch` 同语义），
/// 未提及字段保持全局值。返回前 `normalize()`（幂等）。
pub(crate) fn merge_overlay(base: &Config, overlay: &Value) -> anyhow::Result<Config> {
    let mut global = serde_json::to_value(base)?;
    crate::config::merge_patch(&mut global, overlay);
    let mut cfg: Config = serde_json::from_value(global)?;
    cfg.normalize();
    Ok(cfg)
}
