//! `[nlu]`（规则 NLU）的字段元数据表。
//!
//! 拆出本文件的唯一原因见 `AGENTS.md` §5.10（文件规模约定）：`fields.rs` 已近 500 行。
//! 与 `fields_command.rs` 同构，由 `impls.rs` 的 `NLU_RULES` 描述符直接引用。
//!
//! ⚠️ 这些是**元数据**：真实默认值仍在 `NluConfig` 的 serde `default` 里（单一权威）。
//! 漂移由 `registry/tests.rs` 的 `field_keys_exist_in_config` 兜底。
//!
//! 说明：`[nlu]` 的 `intents` / `slots` 是**嵌套数组结构**（意图模板 + 槽位值表），
//! 通用 schema 无法用单个输入框表达，因此不在此登记为扁平字段——它们在管理页以"高级配置"
//! 形式直接编辑 TOML（见 `enabled` 帮助文案与 `config.example.toml` 范例）。

use super::{FieldKind, FieldSchema, HotReload};

/// `[nlu]` 顶层字段（路径 `["nlu"]`）。
pub(super) const NLU_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        key: "enabled",
        label: "启用规则 NLU",
        kind: FieldKind::Bool,
        default_hint: "false",
        help: "打开后，识别文本先过本地规则引擎：命中已知意图即本地执行（**不再送大模型**），\
               只有未知意图才放行给 LLM。内置 `end_session`（退下/闭嘴/关闭）来自 `[command]` 段，\
               此处额外定义带槽位的自定义意图（如「把{room}的灯{action}」）。\
               关闭时本段不产生任何意图，行为与没有这个插件完全一致。\
               自定义意图与槽位为嵌套结构，在管理台「技能」分区可视化配置（等价于本文件 `[nlu]` 段，\
               范例见 config.example.toml）：`intents` 每项含 name / templates（含 {slot} 占位）/ \
               action（kind + reply）；`slots` 每项含 name / values（封闭值表，用于校验与抽取）/ optional。",
        hot: HotReload::NextSession,
        required: true,
    },
    FieldSchema {
        key: "model_threshold",
        label: "轻量模型阈值",
        kind: FieldKind::Float,
        default_hint: "0.9",
        help: "规则未命中后的本地模糊打分层（零外网）：对技能模板的锚点覆盖与槽位值表打分，\
               达到该阈值且领先第二名 model_margin 才直接执行（不再送大模型）。\
               取值 0~1，越高越保守；1.0 = 关闭该级。放行的句子才走云 LLM。",
        hot: HotReload::NextSession,
        required: false,
    },
    FieldSchema {
        key: "model_margin",
        label: "轻量模型领先幅度",
        kind: FieldKind::Float,
        default_hint: "0.1",
        help: "最高分与**其他技能**最高分的最小差值：相近指令打分接近时不本地执行，\
               交给大模型判断（防「开灯/关灯」这类歧义误触发）。取值 0~1。",
        hot: HotReload::NextSession,
        required: false,
    },
];
