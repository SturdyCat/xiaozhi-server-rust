//! Public plugin descriptor constants.

use super::super::fields::*;
use super::super::fields_command::COMMAND_FIELDS;
use super::super::fields_context::{MEMORY_FIELDS, SOUL_FIELDS};
use super::super::fields_nlu::NLU_FIELDS;
use super::super::fields_wake::WAKE_FIELDS;
use super::super::validate::{
    validate_aiui, validate_asr, validate_command, validate_llm, validate_memory, validate_nlu,
    validate_soul, validate_tts_kokoro, validate_tts_wasm_policy, validate_tts_xfyun, validate_vad,
    validate_wake, warn_llm, warn_memory,
};
use super::super::{Capability, HotReload, PluginDescriptor, Requirement};
use super::*;

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

/// 远端 ASR 引擎（wasm 插件，P4b）：**requires 为空**——不需要模型文件/凭据/sherpa，
/// "远端用户免模型"部署可用；运行期依赖（宿主/插件装载）由 `WasmAsrEngine::recognize`
/// 显式报错（enabled 是配置意图，phase 是运行事实——缺失不炸 boot）。
pub const ASR_WASM_REMOTE: PluginDescriptor = PluginDescriptor {
    id: "asr.wasm-remote",
    capability: Capability::Asr,
    display: "远端引擎（wasm 插件，utterance 级）",
    local: false,
    hot: HotReload::RestartOnly,
    requires: &[],
    fields_path: &["asr"],
    fields: &[],
    signature: sig_asr_wasm,
    validate: validate_asr_wasm,
    build: Some(build_asr_wasm_d),
    warn_if: None,
    selected: sel_asr_wasm,
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

/// WASM 会话级控制策略 + native Silero 帧级 VAD；WASM 不承载 PCM。
pub const VAD_WASM_POLICY: PluginDescriptor = PluginDescriptor {
    id: "vad.wasm-policy",
    capability: Capability::Vad,
    display: "WASM VAD 控制策略 + native Silero",
    local: false,
    hot: HotReload::NextSession,
    requires: &[Requirement::SherpaFeature, Requirement::ModelFiles],
    fields_path: &["vad"],
    fields: &[],
    signature: sig_vad_wasm_policy,
    validate: validate_vad,
    // VAD 是每会话有状态对象；策略在 acquire_vad 前执行，不伪造共享 Built::Vad。
    build: None,
    warn_if: None,
    selected: sel_vad_wasm_policy,
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

/// WASM 控制策略 + 原生 Kokoro PCM（P4）：WASM 不承载模型或音频数据。
pub const TTS_WASM_POLICY: PluginDescriptor = PluginDescriptor {
    id: "tts.wasm-policy",
    capability: Capability::Tts,
    display: "WASM TTS 控制策略 + Kokoro 原生引擎",
    local: false,
    hot: HotReload::NextSession,
    requires: &[Requirement::SherpaFeature, Requirement::ModelFiles],
    fields_path: &["tts"],
    fields: &[],
    signature: sig_tts_wasm_policy,
    validate: validate_tts_wasm_policy,
    build: Some(build_tts_wasm_policy_d),
    warn_if: None,
    selected: sel_tts_wasm_policy,
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

/// WASM 控制策略 + 原生 OpenAI Responses 数据面（P4）。
pub const LLM_WASM_POLICY: PluginDescriptor = PluginDescriptor {
    id: "llm.wasm-policy",
    capability: Capability::Llm,
    display: "WASM LLM 控制策略 + OpenAI 原生 provider",
    local: false,
    hot: HotReload::NextSession,
    requires: &[Requirement::LlmUpstream],
    fields_path: &["llm"],
    fields: LLM_OPENAI_FIELDS,
    signature: sig_llm_wasm_policy,
    validate: validate_llm,
    warn_if: Some(warn_llm),
    build: Some(build_llm_wasm_policy_d),
    selected: sel_llm_wasm_policy,
};

/// 原生 Tuya 局域网连接器（rustuya 数据面；当前由 TuyaEngine 共享装配）。
#[cfg(feature = "tuya-plugin")]
pub const TUYA_NATIVE: PluginDescriptor = PluginDescriptor {
    id: "tuya.native",
    capability: Capability::Tuya,
    display: "Tuya 原生局域网连接器",
    local: true,
    hot: HotReload::Live,
    requires: &[],
    fields_path: &[],
    fields: &[],
    signature: sig_tuya,
    validate: |_| Ok(()),
    build: None,
    warn_if: None,
    selected: sel_tuya_native,
};

/// WASM 控制策略 + 原生 Tuya 局域网数据面（P5）。
#[cfg(feature = "tuya-plugin")]
pub const TUYA_WASM_POLICY: PluginDescriptor = PluginDescriptor {
    id: "tuya.wasm-policy",
    capability: Capability::Tuya,
    display: "WASM Tuya 控制策略 + 原生局域网数据面",
    local: false,
    hot: HotReload::Live,
    requires: &[],
    fields_path: &[],
    fields: &[],
    signature: sig_tuya,
    validate: |_| Ok(()),
    build: None,
    warn_if: None,
    selected: sel_tuya_wasm_policy,
};

/// 原生 EchoMuse 设备入口；路由、会话与协议仍由 native 插件负责。
#[cfg(feature = "echomuse-plugin")]
pub const ECHOMUSE_NATIVE: PluginDescriptor = PluginDescriptor {
    id: "echomuse.native",
    capability: Capability::Echomuse,
    display: "EchoMuse 原生设备入口",
    local: true,
    hot: HotReload::RestartOnly,
    requires: &[],
    fields_path: &[],
    fields: &[],
    signature: sig_echomuse,
    validate: |_| Ok(()),
    build: None,
    warn_if: None,
    selected: sel_echomuse_native,
};

/// EchoMuse 的 WASM 控制策略元数据；本阶段不实现 WASM bridge 或额外路由。
#[cfg(feature = "echomuse-plugin")]
pub const ECHOMUSE_WASM_POLICY: PluginDescriptor = PluginDescriptor {
    id: "echomuse.wasm-policy",
    capability: Capability::Echomuse,
    display: "WASM EchoMuse 控制策略 + 原生设备入口",
    local: false,
    hot: HotReload::RestartOnly,
    requires: &[],
    fields_path: &[],
    fields: &[],
    signature: sig_echomuse,
    validate: |_| Ok(()),
    build: None,
    warn_if: None,
    selected: sel_echomuse_wasm_policy,
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

/// end_session保存态训练词表；不再提供关键词识别。执行复用统一模型门禁。
pub const COMMAND_GATE: PluginDescriptor = PluginDescriptor {
    id: "command.gate",
    capability: Capability::Command,
    display: "结束聆听（模型技能训练词表）",
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

/// 统一模型意图目录。常量名保留源码兼容，公开实现ID为nlu.model。
/// 默认模型开关关闭，启用却不可用时由识别入口明确失败，不做规则或LLM回退。
pub const NLU_RULES: PluginDescriptor = PluginDescriptor {
    id: "nlu.model",
    capability: Capability::Nlu,
    display: "模型意图（唯一识别与同技能抽槽）",
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
