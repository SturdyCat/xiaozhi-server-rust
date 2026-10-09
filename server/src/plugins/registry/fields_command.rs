//! `[command]`（语音指令闸门）的字段元数据表。
//!
//! 拆出本文件的唯一原因见 `AGENTS.md` §5.10（文件规模约定）：`fields.rs` 已近 500 行。
//! 与 `fields.rs` 的"实现私有字段表"同构，由 `impls.rs` 的 `COMMAND_GATE` 描述符直接引用。
//!
//! ⚠️ 这些是**元数据**：真实默认值仍在 `CommandConfig` 的 serde `default_*()` 里（单一权威）。
//! 漂移由 `registry/tests.rs` 的 `field_keys_exist_in_config` 兜底。

use super::{FieldKind, FieldSchema, HotReload};

/// `[command]` 顶层字段（路径 `["command"]`）。
pub(super) const COMMAND_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        key: "enabled",
        label: "启用指令闸门",
        kind: FieldKind::Bool,
        default_hint: "false",
        help: "打开后，识别到指令词（默认「退下/闭嘴/关闭」）会先说告别语再断开本次会话，\
               不再调用大模型；关闭时行为与没有这个插件完全一致。",
        hot: HotReload::NextSession,
        required: true,
    },
    FieldSchema {
        key: "keywords",
        label: "指令词表",
        kind: FieldKind::Text,
        default_hint: "退下 / 闭嘴 / 关闭",
        help: "每行一个词，保存后新会话生效。识别文本会先去标点与句末语气词再比较\
               （「退下吧。」等价于「退下」）。",
        hot: HotReload::NextSession,
        required: false,
    },
    FieldSchema {
        key: "match_mode",
        label: "匹配方式",
        kind: FieldKind::Enum(&["exact", "contains"]),
        default_hint: "exact",
        help: "exact = 整句等于指令词（推荐，安全）；contains = 句中出现即命中——\
               注意那会把「关闭闹钟」「把灯关闭」也当成关机指令。",
        hot: HotReload::NextSession,
        required: false,
    },
    FieldSchema {
        key: "max_chars",
        label: "最长字数（超过则不做指令判断）",
        kind: FieldKind::Int,
        default_hint: "5",
        help: "识别文本（去掉标点与句末语气词后）超过这个字数就不做指令判断、直接交给大模型，\
               避免「帮我关闭卧室的灯」这类长句因为含指令词而误断会话；真正的指令只有两三个字。\
               0 = 不限制（等于拆掉这道保险，不建议）。注意：指令词本身长于该值时会永远命中不了，\
               保存时会被拦下并提示。",
        hot: HotReload::NextSession,
        required: false,
    },
    FieldSchema {
        key: "reply",
        label: "告别语",
        kind: FieldKind::Str,
        default_hint: "好的，我先退下了。",
        help: "命中后先说这句再断开（走本地 TTS，一次性合成）；留空 = 直接断开、不出声。",
        hot: HotReload::NextSession,
        required: false,
    },
];
