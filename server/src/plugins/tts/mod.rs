//! 语音合成（TTS）引擎：本地 `sherpa-onnx` Kokoro INT8（`[tts].engine = "kokoro"`）
//! 或讯飞 **AIUI 主动合成**（`engine = "xfyun"`，见 [`aiui`]；shared 签名助手见 [`xfyun`]）。无 mock。
//!
//! 合成结果为单声道 f32 PCM；下行前由音频层做（按需）重采样与 Opus 编码。
//!
//! ## 配置约定（P4，见 `docs/plugin-architecture-unification.md` §3.3）
//!
//! ```toml
//! [tts]
//! engine = "kokoro"        # 实现 id（注册表 tts.kokoro）；旧键 backend 仍可读（"sherpa" 亦映射）
//! cache_entries = 256      # **跨实现通用项**才留本体
//!
//! [tts.kokoro]             # 实现私有段（本地实现也进子段——统一规则）
//! model = "/data/models/Kokoro/model.int8.onnx"
//! lang  = "zh"
//!
//! [tts.xfyun]              # 远程实现私有段（凭据）
//! app_id = "..."
//! ```
//!
//! **旧写法仍可读**：`[tts].model` 等直接放本体的字段、以及 `backend = "sherpa"`，
//! 由 [`TtsConfig::normalize`] 在加载时搬到规范位置（见模块内 `LEGACY_BODY_FIELDS`），
//! 保存时只写新结构（旧字段一律 `skip_serializing`）。因此 P4 对本文件是**只增不删**。
//!
//! ## 扩展新供应商（目标 = 加 1 条描述符）
//!
//! ① `plugins/tts/<id>.rs` 实现 `TtsEngine`；② 本文件加 `[tts.<id>]` 凭据/参数结构 + 一个
//! `build_<id>()`；③ `plugins/registry/impls.rs` 加 1 个描述符（`fields` 放 `registry/fields.rs`）。
//! 不再需要：枚举变体 / `backend_kind()` / `engine_signature()` / 中心化 `match` 分发
//! —— 这些在 P4 已全部删除（分发改由描述符的 `build`/`signature` 承担）。
//!
//! ## 流式接口
//! [`TtsEngine::synthesize_stream`] 是**首选入口**：合成过程中按片回调 PCM（增量），
//! 调用方边收边下发，显著降低首字延迟（讯飞为服务端分片返回；Kokoro 为逐段回调）。
//! [`TtsEngine::synthesize`] 保留给需要整段的场景，默认按流式收集实现（反之亦然）。
//!
//! ⚠️ 两者都是**同步阻塞调用**（本地推理实测 8~20s；在线为网络等待），调用方
//!（[`crate::app::session`]）必须用 `spawn_blocking` 隔离，否则独占 tokio worker，
//! 期间同 runtime 的其他会话/HTTP 全部卡死。

use serde::{Deserialize, Serialize};

use crate::config::{canonical_engine, EngineSpec};

/// TTS 可选实现（`[tts].engine` 的权威表）。
///
/// 别名只用于**读**：`sherpa` 是 P4 前 `[tts].backend` 的历史值（当时指"本地 Kokoro"），
/// 规范化后统一为 `kokoro`（= 描述符 id `tts.kokoro` 的后半段）。
pub const TTS_ENGINES: &[EngineSpec] = &[
    EngineSpec {
        id: "kokoro",
        aliases: &["sherpa"],
    },
    EngineSpec {
        id: "xfyun",
        aliases: &[],
    },
];

const DEFAULT_TTS_ENGINE: &str = "kokoro";

/// TTS 配置（`[tts]`）：本体只留**跨实现通用项** + 各实现的私有段。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TtsConfig {
    /// 实现 id：`"kokoro"`（本地 Kokoro INT8，默认）| `"xfyun"`（科大讯飞在线）。
    /// 空 = 未指定 → 取 [`DEFAULT_TTS_ENGINE`]（旧配置无此键时照常工作）。
    #[serde(default)]
    pub engine: String,
    /// TTS 结果缓存条目数（按句缓存已编码下行 Opus 帧，跨会话共享；
    /// 命中时零合成延迟、直接按实时节奏下发）。0 = 关闭缓存。
    /// **跨实现通用**（缓存的是已编码帧，与谁合成的无关），故留本体。
    #[serde(default = "default_cache_entries")]
    pub cache_entries: u32,
    /// 语速倍率：**跨实现通用**（`TtsEngine::synthesize_stream` 的入参，Kokoro 与讯飞都用）。
    #[serde(default = "default_speed")]
    pub speed: f32,
    /// 发音人 id：跨实现的**参数槽**（Kokoro 用 sid，讯飞忽略并改由 `[tts.xfyun].voice` 决定）。
    #[serde(default)]
    pub speaker: i32,
    /// 本地 Kokoro INT8（`[tts.kokoro]`）。
    #[serde(default)]
    pub kokoro: KokoroTtsConfig,
    /// 科大讯飞在线 TTS（`[tts.xfyun]`）。
    #[serde(default)]
    pub xfyun: XfyunTtsConfig,

    // ------------------------------------------------------------------
    // 以下为 **P4 之前的旧位置**：只读（`skip_serializing`）。
    // 加载时由 `normalize()` 搬进 `[tts.kokoro]`，保存时不再写回。
    // ------------------------------------------------------------------
    /// 旧键 `[tts].backend`（= `engine` 的前身）。
    #[serde(default, skip_serializing)]
    pub backend: String,
    #[serde(default, skip_serializing)]
    pub model: String,
    #[serde(default, skip_serializing)]
    pub voices: String,
    #[serde(default, skip_serializing)]
    pub tokens: String,
    #[serde(default, skip_serializing)]
    pub data_dir: String,
    #[serde(default, skip_serializing)]
    pub dict_dir: String,
    #[serde(default, skip_serializing)]
    pub lexicon: String,
    #[serde(default, skip_serializing)]
    pub lang: String,
    #[serde(default, skip_serializing)]
    pub num_threads: u32,
}

/// 旧位置字段名（`[tts]` 本体）→ 规范位置（`[tts.kokoro]`）。
///
/// `app/ws/config.rs` 用它把**旧客户端的补丁**重写到规范路径——否则「客户端改 model」
/// 会被"新位置优先"规则静默丢弃（旧客户端仍在用的 macApp 构建会踩到）。
/// `speed` / `speaker` **不在**表里：它们是跨实现参数，本来就留在 `[tts]` 本体（位置没变）。
pub const LEGACY_BODY_FIELDS: &[&str] = &[
    "model",
    "voices",
    "tokens",
    "data_dir",
    "dict_dir",
    "lexicon",
    "lang",
    "num_threads",
];

/// 本地 Kokoro INT8 合成参数（`[tts.kokoro]`；字段语义同 P4 前的 `[tts]` 本体）。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct KokoroTtsConfig {
    /// Kokoro INT8 模型路径（sherpa-onnx 离线合成器）。
    #[serde(default = "default_tts_model")]
    pub model: String,
    #[serde(default = "default_tts_voices")]
    pub voices: String,
    #[serde(default = "default_tts_tokens")]
    pub tokens: String,
    #[serde(default = "default_tts_data_dir")]
    pub data_dir: String,
    #[serde(default = "default_tts_dict_dir")]
    pub dict_dir: String,
    #[serde(default = "default_tts_lexicon")]
    pub lexicon: String,
    /// Kokoro 语言（模型创建时固定；"zh"/"en"，中文场景用 "zh"）。
    #[serde(default = "default_tts_lang")]
    pub lang: String,
    #[serde(default = "default_tts_threads")]
    pub num_threads: u32,
}

impl Default for KokoroTtsConfig {
    fn default() -> Self {
        KokoroTtsConfig {
            model: default_tts_model(),
            voices: default_tts_voices(),
            tokens: default_tts_tokens(),
            data_dir: default_tts_data_dir(),
            dict_dir: default_tts_dict_dir(),
            lexicon: default_tts_lexicon(),
            lang: default_tts_lang(),
            num_threads: default_tts_threads(),
        }
    }
}

impl TtsConfig {
    /// 规范化后的实现 id（空 → 默认；旧 `backend` 兜底；`sherpa` → `kokoro`）。
    /// 未知值**原样返回**：由 `PluginHost` 诊断并报错（见 `registry::unknown_engine_hint`），
    /// 不静默回退默认值。
    pub fn engine_id(&self) -> String {
        let raw = if !self.engine.trim().is_empty() {
            self.engine.as_str()
        } else {
            self.backend.as_str()
        };
        canonical_engine(raw, TTS_ENGINES, DEFAULT_TTS_ENGINE)
    }


    /// 配置规范化（幂等）：补 `engine` 键 + 把旧位置字段搬进 `[tts.kokoro]`。
    ///
    /// 「新位置优先」：只有当 `[tts.kokoro]` 对应字段**仍是默认值**、而旧位置的值
    /// **有实质设置**时才搬（旧位置的值恰好等于默认值 → 搬不搬等价，跳过以免噪音日志）。
    pub fn normalize(&mut self) {
        // (1) engine 键：旧 backend 兜底 + 别名规范化（`sherpa` → `kokoro`）
        let legacy_raw = self.backend.trim().to_string();
        let canonical = self.engine_id();
        if !legacy_raw.is_empty() && self.engine.trim().is_empty() {
            tracing::info!(
                "[tts] 旧键 backend=\"{legacy_raw}\" 已迁移为 engine=\"{canonical}\"（保存后只写 engine）"
            );
        } else if !self.engine.trim().is_empty() && self.engine.trim() != canonical {
            tracing::info!(
                "[tts].engine=\"{}\" 已规范化为 \"{canonical}\"",
                self.engine.trim()
            );
        }
        self.engine = canonical;

        // (2) 旧扁平字段 → [tts.kokoro]
        let def = KokoroTtsConfig::default();
        let mut moved: Vec<&'static str> = Vec::new();
        macro_rules! absorb_str {
            ($legacy:ident, $field:ident, $name:literal) => {
                if !self.$legacy.trim().is_empty() && self.kokoro.$field == def.$field {
                    self.kokoro.$field = self.$legacy.clone();
                    moved.push($name);
                }
            };
        }
        absorb_str!(model, model, "model");
        absorb_str!(voices, voices, "voices");
        absorb_str!(tokens, tokens, "tokens");
        absorb_str!(data_dir, data_dir, "data_dir");
        absorb_str!(dict_dir, dict_dir, "dict_dir");
        absorb_str!(lexicon, lexicon, "lexicon");
        absorb_str!(lang, lang, "lang");
        if self.num_threads != 0 && self.kokoro.num_threads == def.num_threads {
            self.kokoro.num_threads = self.num_threads;
            moved.push("num_threads");
        }
        if !moved.is_empty() {
            tracing::info!(
                "[tts] 旧位置字段已迁移到 [tts.kokoro]（本次保存即写入新结构）: {moved:?}"
            );
        }
    }
}

fn default_cache_entries() -> u32 {
    256
}

fn default_tts_model() -> String {
    "/data/models/Kokoro/model.int8.onnx".into()
}

fn default_tts_voices() -> String {
    "/data/models/Kokoro/voices.bin".into()
}

fn default_tts_tokens() -> String {
    "/data/models/Kokoro/tokens.txt".into()
}

fn default_tts_data_dir() -> String {
    "/data/models/Kokoro/espeak-ng-data".into()
}

fn default_tts_dict_dir() -> String {
    "/data/models/Kokoro/dict".into()
}

fn default_tts_lexicon() -> String {
    "/data/models/Kokoro/lexicon-us-en.txt,/data/models/Kokoro/lexicon-zh.txt".into()
}

fn default_tts_lang() -> String {
    "zh".into()
}

fn default_speed() -> f32 {
    1.0
}

/// TTS 合成线程数默认 4。
/// 旧默认 1 的理由是「与 ASR 错峰」——实机实测该顾虑不成立：语音流水线本身
/// ASR → LLM → TTS 串行，TTS 合成时 ASR 并不在跑；单线程让合成只剩 1/4 算力，
/// 实测 RTF≈7（4 个字要 8+ 秒，CPU 仅 25%）。多设备并发是吞吐问题、4 核本来就不够，
/// 不该牺牲单次合成的延迟。容器部署已由 docker-compose.yml 的 cpus:"3.5" 限核
/// （留 0.5 核给宿主机），4 线程在配额内调度；如需更保守可下调到 3。
fn default_tts_threads() -> u32 {
    4
}

// 讯飞 AIUI 主动合成（在线 TTS 实现；音色探测同源，见 [`aiui::probe_voice`]）
pub(crate) mod aiui;
#[cfg(feature = "sherpa")]
mod kokoro;
// pub(crate)：AIUI 全链路插件复用同厂商的 HMAC 签名算法
pub(crate) mod xfyun;

// bin crate 内部暂无直接引用者：作为插件对外 API 面保留
#[allow(unused_imports)]
pub use xfyun::XfyunTtsConfig;

use anyhow::Result;
#[cfg(feature = "sherpa")]
use anyhow::Context;
#[cfg(feature = "sherpa")]
use kokoro::SherpaTts;
use std::sync::Arc;

/// 流式分片回调：`(输出采样率, 本片增量 PCM)`；返回 `false` 表示调用方要求取消
///（如打断/连接断开），引擎应尽快停止合成并返回 `Ok(())`。
pub type TtsChunkCallback = Box<dyn FnMut(u32, &[f32]) -> bool + Send>;

/// 合成引擎接口：文本 → 单声道 f32 PCM + 采样率。
/// `speaker` 为语音角色（Kokoro sid / 讯飞忽略），由调用方按请求传入。
pub trait TtsEngine: Send + Sync {
    /// 流式合成（**引擎必须实现**）：按片回调**增量** PCM（非累计），返回时合成结束。
    /// 回调返回 `false` = 调用方取消（打断/断开），引擎应尽快停止并返回 `Ok(())`。
    fn synthesize_stream(
        &self,
        text: &str,
        speed: f32,
        speaker: i32,
        chunk_cb: TtsChunkCallback,
    ) -> Result<()>;

    /// 整段合成（默认：收集 [`Self::synthesize_stream`] 的输出）。
    /// 当前调用方均走流式；保留为接口完整性（如离线批量合成）。
    #[allow(dead_code)]
    fn synthesize(&self, text: &str, speed: f32, speaker: i32) -> Result<(Vec<f32>, u32)> {
        // 回调要求 'static（且引擎实现内部可能再 move）：状态放共享容器，外部读取
        let collected: Arc<std::sync::Mutex<(Vec<f32>, u32)>> =
            Arc::new(std::sync::Mutex::new((Vec::new(), 0)));
        let sink = collected.clone();
        self.synthesize_stream(
            text,
            speed,
            speaker,
            Box::new(move |rate, chunk| {
                let mut c = sink.lock().unwrap_or_else(|e| e.into_inner());
                c.1 = rate;
                c.0.extend_from_slice(chunk);
                true
            }),
        )?;
        let out = std::mem::take(&mut *collected.lock().unwrap_or_else(|e| e.into_inner()));
        Ok(out)
    }

    /// 合成输出采样率（下发链路的重采样源采样率）。
    fn output_sample_rate(&self) -> u32 {
        24_000
    }

    /// 引擎标识（测试台 tts_test 结果回报用）：`kokoro` / `xfyun`。
    fn name(&self) -> &'static str {
        "unknown"
    }
}

/// sherpa 真引擎构造（两入口共用）：集中 `SherpaTts::new` 与报错文案，避免重复。
#[cfg(feature = "sherpa")]
fn build_sherpa_tts(cfg: &KokoroTtsConfig) -> Result<Arc<dyn TtsEngine>> {
    let engine = SherpaTts::new(cfg).context("创建 Kokoro TTS 失败")?;
    Ok(Arc::new(engine))
}

/// 构造本地 Kokoro 引擎（描述符 `tts.kokoro` 的 `build`）。
pub fn build_kokoro(cfg: &KokoroTtsConfig) -> Result<Arc<dyn TtsEngine>> {
    #[cfg(feature = "sherpa")]
    {
        build_sherpa_tts(cfg)
    }
    #[cfg(not(feature = "sherpa"))]
    {
        let _ = cfg;
        anyhow::bail!("本二进制未启用 `sherpa` feature，无法构建 TTS 引擎（请用 --features sherpa 编译）");
    }
}

/// 按指定语言构造 Kokoro 引擎（测试台的多语言切换用）。
/// Kokoro 的 `lang` 在建模时固定，切语言 = 重建引擎（engine.rs 按语言缓存）。
pub fn build_kokoro_with_lang(cfg: &KokoroTtsConfig, lang: &str) -> Result<Arc<dyn TtsEngine>> {
    #[cfg(feature = "sherpa")]
    {
        let mut c = cfg.clone();
        c.lang = lang.to_string();
        // build_sherpa_tts 已返回 Arc<dyn TtsEngine>——不要再 Arc::new 包一层
        //（曾因双层 Arc 致 Docker sherpa 构建失败，见 18b6e8d）。
        build_sherpa_tts(&c).with_context(|| format!("创建 lang={lang} 的 Kokoro TTS 失败"))
    }
    #[cfg(not(feature = "sherpa"))]
    {
        let _ = (cfg, lang);
        anyhow::bail!(
            "本二进制未启用 `sherpa` feature，无法构建 lang={lang} 的 TTS 引擎（请用 --features sherpa 编译）"
        );
    }
}

/// 构造讯飞 AIUI 在线合成引擎（描述符 `tts.xfyun` 的 `build`）。
pub fn build_xfyun(cfg: &XfyunTtsConfig) -> Result<Arc<dyn TtsEngine>> {
    Ok(Arc::new(aiui::AiuiTts::new(cfg)?))
}

impl Default for TtsConfig {
    fn default() -> Self {
        TtsConfig {
            engine: String::new(), // 空 → engine_id() 取默认（kokoro）；normalize() 会补全
            cache_entries: default_cache_entries(),
            speed: default_speed(),
            speaker: 0,
            kokoro: KokoroTtsConfig::default(),
            xfyun: XfyunTtsConfig::default(),
            backend: String::new(),
            model: String::new(),
            voices: String::new(),
            tokens: String::new(),
            data_dir: String::new(),
            dict_dir: String::new(),
            lexicon: String::new(),
            lang: String::new(),
            num_threads: 0,
        }
    }
}

#[cfg(test)]
mod tests;
