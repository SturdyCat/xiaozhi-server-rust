//! 注册表的 JSON 投影（手写 wire format，保证 API 契约稳定、可单测）。
//!
//! 从 `mod.rs` 拆出（AGENTS.md §5.10 文件规模约定：行为不变的搬移）：
//! `GET /api/plugins` 的实现列表投影与 `GET /api/config/schema` 的字段元数据投影。

use super::{
    capability_common_fields, impls_of, FieldKind, FieldSchema, ALL_CAPABILITIES, APP_SECTION_HOT,
};

// ============================================================
// JSON 投影（手写 wire format，保证 API 契约稳定、可单测）
// ============================================================

/// 「高级字段」清单（P0-1）：命中字段的 schema JSON 带 `advanced: true`，管理页默认折叠进
/// 「高级选项」区（新手少 seeing、调优的人展开）。
///
/// 用**集中清单**而非给 `FieldSchema` 加字段：4 个 fields 文件共 100 处静态初始化器
/// 零改动；漂移由 `tests::advanced_keys_exist_in_registry` 兜底（字段改名/删除必须同步这里）。
/// 完整 key = `fields_path` + `key` 以 `.` 连接（如 `tts.kokoro.num_threads`）。
pub static ADVANCED_KEYS: &[&str] = &[
    // ASR：识别调优
    "asr.use_itn",
    "asr.num_threads",
    // VAD：切句时序微调（threshold 是"灵敏不灵敏"的主旋钮，保留在基础区）
    "vad.min_speech_duration",
    "vad.min_silence_duration",
    // TTS：缓存与线程
    "tts.cache_entries",
    "tts.kokoro.num_threads",
    // Memory：检索预算与库运维
    "memory.recall_budget_ms",
    "memory.recall_max_nodes",
    "memory.recall_max_tokens",
    "memory.db_busy_timeout_ms",
    "memory.maintenance_interval",
    "memory.retention.retention_days",
    "memory.quarantine_max_attempts",
    // AIUI 全链路：云端合成节奏微调
    "aiui.pace_ms",
];

/// `fields_path` + 字段 key → 全局唯一完整 key（`ADVANCED_KEYS` 的对账口径）。
pub(super) fn full_field_key(path: &[&str], key: &str) -> String {
    let mut s = path.join(".");
    if !s.is_empty() {
        s.push('.');
    }
    s.push_str(key);
    s
}

/// 字段元数据 → JSON（`GET /api/config/schema` 用）。`path` = 所属 `fields_path`
/// （用于 `advanced` 对账，见 [`ADVANCED_KEYS`]）。
pub fn field_json(path: &[&str], f: &FieldSchema) -> serde_json::Value {
    let mut v = serde_json::json!({
        "key": f.key,
        "label": f.label,
        "kind": f.kind.as_str(),
        // 密钥字段只回类型标记：管理页据此**不回显明文**、留空即保留原值
        "secret": f.kind.is_secret(),
        "default_hint": f.default_hint,
        "help": f.help,
        "hot": f.hot.as_str(),
        "hot_hint": f.hot.hint(),
        "required": f.required,
        // P0-1：高级字段标记（管理页默认折叠，见 ADVANCED_KEYS 模块注释）
        "advanced": ADVANCED_KEYS.contains(&full_field_key(path, f.key).as_str()),
    });
    if let FieldKind::Enum(options) = f.kind {
        v["options"] = serde_json::json!(options);
    }
    v
}

fn fields_json(path: &[&str], fields: &[FieldSchema]) -> serde_json::Value {
    serde_json::Value::Array(fields.iter().map(|f| field_json(path, f)).collect())
}

/// `GET /api/config/schema` 响应体：按能力聚合（公共字段 + 各实现的私有字段）。
///
/// ⚠️ 这是**元数据**，不是自动表单：前端 tab 仍手写（DSH 的 `autoGenerate` 至今无客户端使用，
/// 见 `docs/plugin-architecture-unification.md` §11.1）。这里解决的是"字段默认值/热生效语义/
/// 密钥标记/校验提示"在前后端各写一遍导致的漂移。
pub fn schema_json() -> serde_json::Value {
    let capabilities: Vec<serde_json::Value> = ALL_CAPABILITIES
        .iter()
        .map(|&cap| {
            let common: Vec<serde_json::Value> = capability_common_fields(cap)
                .map(|(path, fields)| {
                    serde_json::json!({ "fields_path": path, "fields": fields_json(path, fields) })
                })
                .collect();
            let impls: Vec<serde_json::Value> = impls_of(cap)
                .map(|d| {
                    serde_json::json!({
                        "id": d.id,
                        "display": d.display,
                        "local": d.local,
                        "hot": d.hot.as_str(),
                        "hot_hint": d.hot.hint(),
                        "requires": d.requires.iter().map(|r| r.as_str()).collect::<Vec<_>>(),
                        "fields_path": d.fields_path,
                        "fields": fields_json(d.fields_path, d.fields),
                    })
                })
                .collect();
            serde_json::json!({
                "id": cap.id(),
                "display": cap.display(),
                "required": cap.required(),
                "implemented": cap.implemented(),
                "common_fields": common,
                "implementations": impls,
            })
        })
        .collect();
    serde_json::json!({
        "capabilities": capabilities,
        // **非插件**配置段（`[server]` / `[audio]`）的档位：它们不在能力注册表里，
        // 但管理页同样需要"改这段多久生效"的权威答案（此前只能靠卡片文案口口相传）。
        "app_sections": APP_SECTION_HOT
            .iter()
            .map(|(section, hot)| serde_json::json!({
                "id": section,
                "hot": hot.as_str(),
                "hot_hint": hot.hint(),
            }))
            .collect::<Vec<_>>(),
    })
}
