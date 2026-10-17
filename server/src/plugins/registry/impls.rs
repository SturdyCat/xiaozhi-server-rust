//! 各实现的**选择 / 签名 / 构建**与描述符常量（校验文案见 `validate.rs`）。
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
use super::fields_nlu::NLU_FIELDS;
use super::fields_wake::WAKE_FIELDS;
use super::validate::{
    validate_aiui, validate_asr, validate_command, validate_llm, validate_memory, validate_nlu,
    validate_soul, validate_tts_kokoro, validate_tts_xfyun, validate_vad, validate_wake,
    warn_llm, warn_memory,
};
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
/// P1-7 多后端：`selected` 按实现拆分（不拆的话 active() 对 graph/store 都为真，
/// 两条 memory 描述符会同时选中，`active_impl` 无法唯一确定）。
fn sel_memory_graph(cfg: &Config) -> bool {
    cfg.memory.enabled && cfg.memory.engine == crate::plugins::memory::MemoryBackend::Graph
}
fn sel_memory_store(cfg: &Config) -> bool {
    cfg.memory.enabled && cfg.memory.engine == crate::plugins::memory::MemoryBackend::Store
}

fn sel_memory_rag(cfg: &Config) -> bool {
    cfg.memory.enabled && cfg.memory.engine == crate::plugins::memory::MemoryBackend::Rag
}
fn sel_soul(cfg: &Config) -> bool {
    cfg.soul.enabled
}
fn sel_command(cfg: &Config) -> bool {
    cfg.command.enabled
}
fn sel_wake(cfg: &Config) -> bool {
    cfg.wakeup.enabled
}
fn sel_nlu(cfg: &Config) -> bool {
    cfg.nlu.enabled || !cfg.nlu.intents.is_empty()
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

fn sig_wake(cfg: &Config) -> String {
    cfg.wakeup.signature()
}
/// NLU 无共享实例（每会话按快照构造技能目录），签名只用于状态上报与热切换判定。
/// 涵盖 enabled + 阈值 + 意图数 + 槽位数 + 每个意图的模板原文（模板/阈值改动必须触发重建）。
fn sig_nlu(cfg: &Config) -> String {
    let mut s = format!(
        "{}|{}|{}|{}|{}",
        cfg.nlu.enabled,
        cfg.nlu.model_threshold,
        cfg.nlu.model_margin,
        cfg.nlu.intents.len(),
        cfg.nlu.slots.len()
    );
    // bert_* 一并入签：模型/词表/标签路径或阈值改动必须触发 TinyBERT 引擎重建。
    s.push_str(&crate::plugins::nlu::bert_signature(&cfg.nlu));
    for it in &cfg.nlu.intents {
        s.push('|');
        s.push_str(&it.name);
        for t in &it.templates {
            s.push('|');
            s.push_str(t);
        }
    }
    s
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
    selected: sel_memory_graph,
};

/// 纯存储后端（P1-7：`[memory].engine = "store"`）：只落对话原文，零 LLM、零召回。
///
/// 与 graph 的关键差异：`requires = &[]`（不依赖 LlmUpstream——构造根本不碰抽取路由）、
/// `warn_if = None`（"不召回"是设计定位不是能力受限）。validate/signature/build 与 graph
/// 共用同一组函数：签名含 engine 字段（切换必触发重建）、validate 的 db_path/scope 检查
/// 对 store 同样必需、build 经 `build_memory` 工厂内部分发（store 分支不构建 LLM）。
pub const MEMORY_STORE: PluginDescriptor = PluginDescriptor {
    id: "memory.store",
    capability: Capability::Memory,
    display: "纯存储（只存原文 · 不抽取不召回）",
    local: true,
    hot: HotReload::NextSession,
    requires: &[],
    fields_path: &["memory"],
    // 无私有字段：[memory] 段的公共字段在 MEMORY_FIELDS 已登记，两个实现共享同一份
    fields: MEMORY_FIELDS,
    signature: sig_memory,
    validate: validate_memory,
    build: Some(build_memory_d),
    warn_if: None,
    selected: sel_memory_store,
};

/// 本地文档 RAG（P1-9：`[memory].engine = "rag"`）：扫描目录建词法索引，召回文档分块。
///
/// 与 store 同款的注册表形状：`requires = &[]`（构造不碰 LLM）、`warn_if = None`
/// （docs_dir 为空 = 零知识零影响，是正常态不是降级）。validate/signature/build 与
/// graph/store 共用同一组函数；rag 参数组（`[memory.rag]`）在 MEMORY_RAG_FIELDS 单独登记。
pub const MEMORY_RAG: PluginDescriptor = PluginDescriptor {
    id: "memory.rag",
    capability: Capability::Memory,
    display: "本地文档 RAG（扫描目录 · 词法检索）",
    local: true,
    hot: HotReload::NextSession,
    requires: &[],
    fields_path: &["memory"],
    fields: MEMORY_FIELDS,
    signature: sig_memory,
    validate: validate_memory,
    build: Some(build_memory_d),
    warn_if: None,
    selected: sel_memory_rag,
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
    display: "指令闸门（退下/闭嘴/关闭 → 结束聆听）",
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

/// 唤醒问候语（`[wakeup]`）：设备唤醒 → LLM **之前**的拦截。
///
/// **可选能力**：默认 `enabled=false` 时不选中、零开销、行为与加它之前完全一致；
/// 坏配置（启用但问候语为空 / 过长 / 与 `[tts].cache_entries=0` 矛盾）只降级（`Degraded`），
/// 绝不阻止启动——语音服务照常。
/// 无共享实例：每会话按配置快照构造 `WakeGreeting`（与 `[command]` 同为"每会话有状态对象"模式）。
pub const WAKE_GREETING: PluginDescriptor = PluginDescriptor {
    id: "wakeup.greeting",
    capability: Capability::Wake,
    display: "唤醒问候语（唤醒即回「我在」，跳过 LLM/TTS）",
    local: true,
    hot: HotReload::NextSession,
    requires: &[],
    fields_path: &["wakeup"],
    fields: WAKE_FIELDS,
    signature: sig_wake,
    validate: validate_wake,
    build: None,
    warn_if: None,
    selected: sel_wake,
};

/// 规则 NLU（`[nlu]`）：ASR → LLM **之间**的本地意图识别层，是 `[command]` 的超集。
///
/// **可选能力**：默认 `enabled=false` 时不选中、零开销、行为与加它之前完全一致；
/// 坏配置（启用但意图无模板）只降级（`Degraded`），绝不阻止启动——语音服务照常。
/// 无共享实例：每会话按配置快照构造 `NluEngine`（与 `[command]`/`[wakeup]` 同为"每会话有状态对象"模式）。
pub const NLU_RULES: PluginDescriptor = PluginDescriptor {
    id: "nlu.rules",
    capability: Capability::Nlu,
    display: "规则 NLU（意图+槽位，替代送 LLM）",
    local: true,
    hot: HotReload::NextSession,
    requires: &[],
    fields_path: &["nlu"],
    fields: NLU_FIELDS,
    signature: sig_nlu,
    validate: validate_nlu,
    build: None,
    warn_if: None,
    selected: sel_nlu,
};
