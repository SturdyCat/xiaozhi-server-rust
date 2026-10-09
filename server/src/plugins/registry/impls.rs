//! 各实现的**选择 / 签名 / 校验 / 构建**与描述符常量。
//!
//! 每个实现 = 本文件里几个私有 fn + 一个 `pub const PluginDescriptor`；
//! 新增实现 = 此处加 1 条（字段元数据表放 `fields.rs`）。

use anyhow::Result;

use crate::config::Config;
use crate::plugins::llm::build_llm;
use crate::plugins::tts::{build_kokoro, build_xfyun};

use super::fields::*;
use super::fields_command::COMMAND_FIELDS;
use super::fields_context::{MEMORY_FIELDS, SOUL_FIELDS};
use super::{Built, Capability, HotReload, PluginDescriptor, Requirement};

// ---- 选中判定（P4：统一按 `engine` 键 = 描述符 id 去掉 "<cap>." 前缀）----

fn sel_always(_cfg: &Config) -> bool {
    true
}
fn sel_asr(cfg: &Config) -> bool {
    cfg.asr.engine_id() == "sensevoice"
}
fn sel_vad(cfg: &Config) -> bool {
    cfg.vad.engine_id() == "silero"
}
fn sel_tts_kokoro(cfg: &Config) -> bool {
    cfg.tts.engine_id() == "kokoro"
}
fn sel_tts_xfyun(cfg: &Config) -> bool {
    cfg.tts.engine_id() == "xfyun"
}
fn sel_llm(cfg: &Config) -> bool {
    cfg.llm.engine_id() == "openai"
}
fn sel_aiui(cfg: &Config) -> bool {
    cfg.aiui.enabled
}
fn sel_memory(cfg: &Config) -> bool {
    cfg.memory.active()
}
fn sel_soul(cfg: &Config) -> bool {
    cfg.soul.enabled
}
fn sel_command(cfg: &Config) -> bool {
    cfg.command.enabled
}

// ---- 未知 engine 校验（P4 约定：静默回退默认是最糟的失败模式，必须报错并给可选值）----

// ---- 签名（仅内存比较）----

fn sig_asr(cfg: &Config) -> String {
    let a = &cfg.asr;
    format!(
        "{}|{}|{}|{}|{}|{}",
        a.model, a.tokens, a.language, a.use_itn, a.num_threads, a.provider
    )
}
fn sig_vad(cfg: &Config) -> String {
    let v = &cfg.vad;
    format!(
        "{}|{}|{}|{}",
        v.model, v.threshold, v.min_silence_duration, v.min_speech_duration
    )
}
/// Kokoro 签名：字段与 P4 前 `TtsConfig::engine_signature()` 的 sherpa 分支**等价**
///（回归安全：签名只用于内存内热切换判定，但要保证"没改配置就不重建"）。
fn sig_tts_kokoro(cfg: &Config) -> String {
    let k = &cfg.tts.kokoro;
    format!("kokoro|{}|{}|{}", k.model, k.lang, k.num_threads)
}
/// ⚠️ 含密钥，**绝不打日志**（仅内存比较）。
fn sig_tts_xfyun(cfg: &Config) -> String {
    let x = &cfg.tts.xfyun;
    format!(
        "xfyun|{}|{}|{}|{}",
        x.app_id, x.api_key, x.api_secret, x.voice
    )
}
fn sig_llm(cfg: &Config) -> String {
    let l = &cfg.llm;
    format!(
        "{}|{}|{}|{}|{}|{}",
        l.api_base, l.api_key, l.model, l.temperature, l.stream, l.system_prompt
    )
}
fn sig_aiui(cfg: &Config) -> String {
    let a = &cfg.aiui;
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        a.enabled,
        a.appid,
        a.api_key,
        a.api_secret,
        a.scene,
        a.sn_prefix,
        a.voice,
        a.speed,
        a.volume,
        a.pitch,
        a.prompt,
        a.pace_ms
    )
}

fn sig_memory(cfg: &Config) -> String {
    cfg.memory.signature()
}
fn sig_soul(cfg: &Config) -> String {
    cfg.soul.signature()
}
/// 指令闸门无共享实例（每会话按快照构造 `CommandGate`），签名只用于状态上报。
fn sig_command(cfg: &Config) -> String {
    cfg.command.signature()
}

// ---- 校验（错误文案必须含「怎么修」）----

fn validate_asr(cfg: &Config) -> Result<()> {
    if !cfg!(feature = "sherpa") {
        anyhow::bail!(
            "本二进制未启用 `sherpa` feature：SenseVoice ASR 需要 --features sherpa 编译（Docker 镜像已内置）。"
        );
    }
    if cfg.asr.model.trim().is_empty() {
        anyhow::bail!(
            "[asr].model 未配置：请填 SenseVoice INT8 模型路径（如 /data/models/SenseVoiceSmall/model.int8.onnx）。\
             容器内缺失模型时入口脚本会自动下载；离线环境请先预置到挂载卷。"
        );
    }
    Ok(())
}

fn validate_vad(cfg: &Config) -> Result<()> {
    // 仅 sherpa 构建需要真实模型（未启用 feature 时用占位实现，见 plugins/vad/mod.rs）
    #[cfg(feature = "sherpa")]
    if cfg.vad.model.trim().is_empty() {
        anyhow::bail!(
            "[vad].model 未配置：请填 Silero VAD 模型路径（如 /data/models/silero_vad.onnx）。\
             容器内缺失模型时入口脚本会自动下载；离线环境请先预置到挂载卷。"
        );
    }
    let _ = cfg;
    Ok(())
}

fn validate_tts_xfyun(cfg: &Config) -> Result<()> {
    let x = &cfg.tts.xfyun;
    if x.app_id.trim().is_empty() || x.api_key.trim().is_empty() || x.api_secret.trim().is_empty() {
        anyhow::bail!(
            "[tts].engine=\"xfyun\" 需要 [tts.xfyun] 的 app_id / api_key / api_secret（讯飞开放平台「在线语音合成」控制台获取）。"
        );
    }
    Ok(())
}

/// Kokoro 的硬条件：`[tts.kokoro]` 的模型/音色数据/词表路径必须非空
/// （P4 前这些字段在 `[tts]` 本体，`normalize()` 已负责搬家）。
fn validate_tts_kokoro(cfg: &Config) -> Result<()> {
    if !cfg!(feature = "sherpa") {
        anyhow::bail!(
            "本二进制未启用 `sherpa` feature：本地 Kokoro TTS 需要 --features sherpa 编译；\
             或把 [tts].engine 切到在线服务商（如 xfyun）。"
        );
    }
    let k = &cfg.tts.kokoro;
    let missing: Vec<&str> = [
        ("model", k.model.as_str()),
        ("voices", k.voices.as_str()),
        ("tokens", k.tokens.as_str()),
        ("data_dir", k.data_dir.as_str()),
    ]
    .iter()
    .filter(|(_, v)| v.trim().is_empty())
    .map(|(n, _)| *n)
    .collect();
    if !missing.is_empty() {
        anyhow::bail!(
            "[tts.kokoro] 缺少必要路径 {:?}：请填 Kokoro INT8 模型包内的对应文件路径\
             （默认 /data/models/Kokoro/…；容器内缺失模型时入口脚本会自动下载）。若配置写在旧位置 \
             `[tts].model`，保存一次即自动迁移到 `[tts.kokoro]`。",
            missing
        );
    }
    Ok(())
}

fn validate_llm(cfg: &Config) -> Result<()> {
    // 硬条件：地址缺失无法调用（保留既有行为：密钥为空**不**阻止启动，见 warn_llm）
    if cfg.llm.api_base.trim().is_empty() {
        anyhow::bail!(
            "[llm].api_base 未配置：请填 OpenAI 兼容 **Responses API** 地址（如 https://api.example.com/v1/responses）。"
        );
    }
    Ok(())
}

/// LLM 软性问题：密钥为空 —— 服务照常启动，但每次对话都会在上游 401/403。
/// 状态上报为 `Degraded`（而不是静默），且热刷新时**不会**用空密钥覆盖正在工作的引擎。
fn warn_llm(cfg: &Config) -> Option<String> {
    if cfg.llm.api_key.trim().is_empty() {
        Some(
            "[llm].api_key 为空：每次对话都会在上游报 401/403；在管理页填入密钥即可（新会话生效）"
                .into(),
        )
    } else {
        None
    }
}

fn validate_aiui(cfg: &Config) -> Result<()> {
    if cfg.aiui.enabled
        && (cfg.aiui.appid.trim().is_empty()
            || cfg.aiui.api_key.trim().is_empty()
            || cfg.aiui.api_secret.trim().is_empty())
    {
        anyhow::bail!(
            "[aiui].enabled 需要 appid / api_key / api_secret（AIUI 平台应用三要素）；\
             不想用全链路模式就把 enabled 改回 false。"
        );
    }
    Ok(())
}

fn validate_memory(cfg: &Config) -> Result<()> {
    cfg.memory.validate()
}

/// 记忆的软性问题：**没有 embedding**（当前为本地词法召回）、抽取路由缺凭据等。
/// 这些都不是错误 —— 记忆照常工作，只是能力受限，必须让 UI 能说出来（对齐 DSH 的
/// 「`enabled` 与实际激活态分开上报」），否则用户只会得到"开了没用"的结论。
fn warn_memory(cfg: &Config) -> Option<String> {
    cfg.memory.warning(&cfg.llm)
}

fn validate_soul(cfg: &Config) -> Result<()> {
    cfg.soul.validate()
}

/// 指令闸门的硬条件：启用时词表不得为空。
/// （词表为空 = 闸门不拦任何话——不致命但必须能被 `/api/plugins` 说成 `Degraded`，
/// 否则用户会得到"配了没用"的静默失败。）
fn validate_command(cfg: &Config) -> Result<()> {
    cfg.command.validate()
}

// ---- 构建入口 ----

fn build_asr_d(cfg: &Config) -> Result<Built> {
    Ok(Built::Asr(crate::plugins::asr::build_asr(&cfg.asr)?))
}
/// Kokoro 的构建入口**只读 `[tts.kokoro]`**（P4 私有段）：新增供应商时各写各的，无需中心分发。
fn build_tts_kokoro_d(cfg: &Config) -> Result<Built> {
    Ok(Built::Tts(build_kokoro(&cfg.tts.kokoro)?))
}
/// 讯飞在线合成（`[tts.xfyun]`）。
fn build_tts_xfyun_d(cfg: &Config) -> Result<Built> {
    Ok(Built::Tts(build_xfyun(&cfg.tts.xfyun)?))
}
fn build_llm_d(cfg: &Config) -> Result<Built> {
    Ok(Built::Llm(build_llm(&cfg.llm)))
}
fn build_memory_d(cfg: &Config) -> Result<Built> {
    Ok(Built::Memory(crate::plugins::memory::build_memory(
        &cfg.memory,
        &cfg.llm,
    )?))
}

// ---- 描述符常量 ----

pub const ASR_SENSEVOICE: PluginDescriptor = PluginDescriptor {
    id: "asr.sensevoice",
    capability: Capability::Asr,
    display: "SenseVoice INT8（本地离线）",
    local: true,
    hot: HotReload::RestartOnly,
    requires: &[Requirement::SherpaFeature, Requirement::ModelFiles],
    fields_path: &["asr"],
    fields: ASR_FIELDS,
    signature: sig_asr,
    validate: validate_asr,
    build: Some(build_asr_d),
    warn_if: None,
    selected: sel_asr,
};

pub const VAD_SILERO: PluginDescriptor = PluginDescriptor {
    id: "vad.silero",
    capability: Capability::Vad,
    display: "Silero VAD（本地离线）",
    local: true,
    // 每会话新建实例，读取 live_config() 快照 → 改阈值对新会话生效
    hot: HotReload::NextSession,
    requires: &[Requirement::ModelFiles],
    fields_path: &["vad"],
    fields: VAD_FIELDS,
    signature: sig_vad,
    validate: validate_vad,
    // VAD 是每会话有状态对象（`Engines::new_vad`），没有共享实例可构建
    build: None,
    warn_if: None,
    selected: sel_vad,
};

pub const TTS_KOKORO: PluginDescriptor = PluginDescriptor {
    id: "tts.kokoro",
    capability: Capability::Tts,
    display: "Kokoro INT8（本地离线）",
    local: true,
    hot: HotReload::NextSession,
    requires: &[Requirement::SherpaFeature, Requirement::ModelFiles],
    // P4：本地实现也进私有段——`[tts.kokoro]`
    fields_path: &["tts", "kokoro"],
    fields: TTS_KOKORO_FIELDS,
    signature: sig_tts_kokoro,
    validate: validate_tts_kokoro,
    build: Some(build_tts_kokoro_d),
    warn_if: None,
    selected: sel_tts_kokoro,
};

pub const TTS_XFYUN: PluginDescriptor = PluginDescriptor {
    id: "tts.xfyun",
    capability: Capability::Tts,
    display: "科大讯飞 AIUI 在线合成",
    local: false,
    hot: HotReload::NextSession,
    requires: &[Requirement::Credentials],
    fields_path: &["tts", "xfyun"],
    fields: TTS_XFYUN_FIELDS,
    signature: sig_tts_xfyun,
    validate: validate_tts_xfyun,
    build: Some(build_tts_xfyun_d),
    warn_if: None,
    selected: sel_tts_xfyun,
};

pub const LLM_OPENAI: PluginDescriptor = PluginDescriptor {
    id: "llm.openai",
    capability: Capability::Llm,
    display: "OpenAI 兼容 Responses API",
    local: false,
    // 会话开始按签名热切换（`llm: RwLock<Llm>` + `Engines::refresh_from_disk`）
    hot: HotReload::NextSession,
    requires: &[Requirement::LlmUpstream],
    fields_path: &["llm"],
    fields: LLM_OPENAI_FIELDS,
    signature: sig_llm,
    validate: validate_llm,
    warn_if: Some(warn_llm),
    build: Some(build_llm_d),
    selected: sel_llm,
};

pub const AIUI_FULLCHAIN: PluginDescriptor = PluginDescriptor {
    id: "fullchain.aiui",
    capability: Capability::FullChain,
    display: "讯飞 AIUI 云端闭环（ASR+LLM+TTS）",
    local: false,
    // 刻意 RestartOnly：它决定**整条流水线的走向**，且启用时凭据缺失是致命错误
    //（boot 快速失败）。热启用会带来"半配置状态下切换链路"的风险，收益不值。
    hot: HotReload::RestartOnly,
    requires: &[Requirement::Credentials],
    fields_path: &["aiui"],
    fields: AIUI_FIELDS,
    signature: sig_aiui,
    validate: validate_aiui,
    // 每会话构造（`AiuiSession::new(cfg, sn)` 需要会话 sn），不进共享引擎池
    build: None,
    warn_if: None,
    selected: sel_aiui,
};

/// 发音人目录探测（app 层服务；P4 纳入统一动作模型，此处只做描述/状态）。
pub const VOICES_CATALOG: PluginDescriptor = PluginDescriptor {
    id: "voices.catalog",
    capability: Capability::Voices,
    display: "发音人目录（真实 API 探测）",
    local: false,
    hot: HotReload::Live,
    requires: &[],
    fields_path: &[],
    fields: &[],
    signature: |_| String::new(),
    validate: |_| Ok(()),
    build: None,
    warn_if: None,
    selected: sel_always,
};

/// 图记忆（本地 SQLite + 词法召回 + 后台抽取）。
///
/// **可选能力**：坏掉只降级（`Phase::Degraded`），绝不拖垮语音服务；默认 `enabled=false`
/// 时不选中，装配为 `NoopMemory`（零依赖零成本）。
pub const MEMORY_GRAPH: PluginDescriptor = PluginDescriptor {
    id: "memory.graph",
    capability: Capability::Memory,
    display: "图记忆（本地 SQLite · 词法召回）",
    local: true,
    hot: HotReload::NextSession,
    // 抽取是一次真实 LLM 调用：没有可用上游时，记忆仍能写入与词法召回，只是没有摘要
    requires: &[Requirement::LlmUpstream],
    fields_path: &["memory"],
    fields: MEMORY_FIELDS,
    signature: sig_memory,
    validate: validate_memory,
    build: Some(build_memory_d),
    warn_if: Some(warn_memory),
    selected: sel_memory,
};

/// 人格档案（`[soul]`）：不产出共享实例（纯配置 → prompt 段）。
pub const SOUL_PROFILE: PluginDescriptor = PluginDescriptor {
    id: "soul.profile",
    capability: Capability::Soul,
    display: "人格档案（有序 prompt 段）",
    local: true,
    hot: HotReload::NextSession,
    requires: &[],
    fields_path: &["soul"],
    fields: SOUL_FIELDS,
    signature: sig_soul,
    validate: validate_soul,
    // 纯配置→段渲染，无需构建共享实例（每轮按 live_config 组装）
    build: None,
    warn_if: None,
    selected: sel_soul,
};

/// 固件托管（app 层服务；只做描述/状态）。
pub const FIRMWARE_HOST: PluginDescriptor = PluginDescriptor {
    id: "firmware.host",
    capability: Capability::Firmware,
    display: "固件托管（上传/下载/版本）",
    local: true,
    hot: HotReload::Live,
    requires: &[],
    fields_path: &[],
    fields: &[],
    signature: |_| String::new(),
    validate: |_| Ok(()),
    build: None,
    warn_if: None,
    selected: sel_always,
};

/// 语音指令闸门（`[command]`）：ASR → LLM **之前**的拦截。
///
/// **可选能力**：默认 `enabled=false` 时不选中、零开销、行为与加它之前完全一致；
/// 坏配置（启用但词表为空）只降级（`Degraded`），绝不阻止启动——语音服务照常。
/// 无共享实例：每会话按配置快照构造 `CommandGate`（与 VAD 同为"每会话有状态对象"模式）。
pub const COMMAND_GATE: PluginDescriptor = PluginDescriptor {
    id: "command.gate",
    capability: Capability::Command,
    display: "指令闸门（退下/闭嘴/关闭 → 断开会话）",
    local: true,
    hot: HotReload::NextSession,
    requires: &[],
    fields_path: &["command"],
    fields: COMMAND_FIELDS,
    signature: sig_command,
    validate: validate_command,
    build: None,
    warn_if: None,
    selected: sel_command,
};
