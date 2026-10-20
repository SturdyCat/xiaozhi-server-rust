//! `[wakeup]`（唤醒问候语）的字段元数据表。
//!
//! 拆出本文件的唯一原因见 `AGENTS.md` §5.10（文件规模约定）：`fields.rs` 已近 500 行。
//! 与 `fields_command.rs` 同构，由 `impls.rs` 的 `WAKE_GREETING` 描述符直接引用。
//!
//! ⚠️ 这些是**元数据**：真实默认值仍在 `WakeConfig` 的 serde `default_*()` 里（单一权威）。
//! 漂移由 `registry/tests.rs` 的 `field_keys_exist_in_config` 兜底。

use super::{FieldKind, FieldSchema, HotReload};

/// `[wakeup]` 顶层字段（路径 `["wakeup"]`）。
pub(super) const WAKE_FIELDS: &[FieldSchema] = &[
    FieldSchema {
        key: "enabled",
        label: "启用唤醒问候语",
        kind: FieldKind::Bool,
        default_hint: "false",
        help: "打开后，设备唤醒（固件上报 listen state=detect）时立刻说一句问候语，\
               跳过 LLM 与正常回复——唤醒是全系统最高频的动作，为一句固定的话付一次大模型调用\
               没有道理。成品音频按音色复用：进程内命中内存缓存，重启后命中磁盘（见 persist），\
               稳态下每次唤醒零 LLM、零 TTS。关闭时唤醒后静默等待，与没有这个插件完全一致。",
        hot: HotReload::NextSession,
        required: true,
    },
    FieldSchema {
        key: "greeting",
        label: "问候语",
        kind: FieldKind::Str,
        default_hint: "我在",
        help: "唤醒后立刻说的话。支持多句：换行分隔，每次唤醒随机选一句（「我在 / 在呢 / 你好呀」\
               换着说）；单行 = 单句。建议每句都短（≤ 10 字）：问候语播放期间上行音频会在缓冲区\
               排队，太长会推迟用户第一句话的处理。改文案会换缓存键，每句各自重新合成一次并落盘。",
        hot: HotReload::NextSession,
        required: false,
    },
    FieldSchema {
        key: "persist",
        label: "持久化复用（重启后仍免合成）",
        kind: FieldKind::Bool,
        default_hint: "true",
        help: "把问候语的成品音频（已编码的下行 Opus 帧，约 2KB）写到配置文件同级的 \
               hot-audio/ 目录，进程重启后直接读盘复用，无需重新调用 TTS。\
               ⚠️ 需要 [tts].cache_entries > 0（复用要经内存缓存承载成品帧）；\
               缓存被关掉时配置校验会拦下并提示。",
        hot: HotReload::NextSession,
        required: false,
    },
];
