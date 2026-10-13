//! 语音识别（ASR）引擎：`sherpa-onnx` 的 `OfflineRecognizer` +
//! `OfflineSenseVoiceModelConfig`（SenseVoice INT8）。无 mock——ASR 只有这一条真实路径。
//!
//! 注意：SenseVoice 在 sherpa-onnx 中是**离线**识别器，方案为
//! VAD 切出语音段后逐段送入识别（非 `OnlineRecognizer` 流式）。
//!
//! ⚠️ [`AsrEngine::recognize`] 是**同步阻塞的 CPU 密集调用**（SenseVoice 推理）。
//! 调用方（[`crate::app::session`]）必须用 `tokio::task::spawn_blocking` 隔离，
//! 否则独占 tokio worker，期间同 runtime 的其他会话/HTTP 全部卡死。

use anyhow::Result;
use serde::{Deserialize, Serialize};
#[cfg(feature = "sherpa")]
use anyhow::Context;
#[cfg(feature = "sherpa")]
use sensevoice::SherpaAsr;
use std::sync::Arc;

use crate::config::{canonical_engine, EngineSpec};

/// ASR 可选实现（`[asr].engine` 的权威表）。当前只有 SenseVoice 一条真实路径。
pub const ASR_ENGINES: &[EngineSpec] = &[EngineSpec {
    id: "sensevoice",
    aliases: &[],
}];
const DEFAULT_ASR_ENGINE: &str = "sensevoice";

/// 识别引擎接口：把一段（已切分好的）单声道 f32 音频转为文本。
pub trait AsrEngine: Send + Sync {
    fn recognize(&self, samples: &[f32], sample_rate: u32) -> Result<String>;
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AsrConfig {
    /// 实现 id（`[asr].engine`，约定见 `crate::config` 模块文档）；空 = 默认 `sensevoice`。
    #[serde(default)]
    pub engine: String,
    /// SenseVoice INT8 模型路径（sherpa-onnx 离线识别器）。
    #[serde(default = "default_asr_model")]
    pub model: String,
    #[serde(default = "default_asr_tokens")]
    pub tokens: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_true")]
    pub use_itn: bool,
    #[serde(default = "default_num_threads")]
    pub num_threads: u32,
    #[serde(default = "default_provider")]
    pub provider: String,
    /// 同时进行的 ASR 推理路上限（跨会话排队背压，缺口①）。
    ///
    /// 默认 1：每路占用 `num_threads` 线程（默认 2），恰等于 `engine.rs //!` 预算表的
    /// ASR 段线程预算——多设备同时说话时在闸门处排队而不是把 CPU 压垮。
    /// 算力富余（或 ASR 为在线引擎）可放宽。改后需重启（ASR 段为 RestartOnly）。
    #[serde(default = "default_one")]
    pub max_concurrent: u32,
}

impl AsrConfig {
    /// 规范化后的实现 id（空 → 默认；未知值原样返回，由校验报错）。
    pub fn engine_id(&self) -> String {
        canonical_engine(&self.engine, ASR_ENGINES, DEFAULT_ASR_ENGINE)
    }
    pub fn normalize(&mut self) {
        self.engine = self.engine_id();
        // 0 许可会把识别永久堵死，clamp ≥ 1（见 app::inference 模块注释）。
        self.max_concurrent = self.max_concurrent.max(1);
    }
}
fn default_asr_model() -> String {
    "/data/models/SenseVoiceSmall/model.int8.onnx".into()
}

fn default_asr_tokens() -> String {
    "/data/models/SenseVoiceSmall/tokens.txt".into()
}

fn default_language() -> String {
    "auto".into()
}

fn default_true() -> bool {
    true
}

fn default_num_threads() -> u32 {
    2
}

fn default_provider() -> String {
    "cpu".into()
}

/// `[asr].max_concurrent` 默认 1（推理闸门许可数，见 `app::inference`）。
fn default_one() -> u32 {
    1
}

#[cfg(feature = "sherpa")]
mod sensevoice;

/// 根据配置构造 ASR 引擎（仅 `sherpa` 一条路径；未启用 feature 直接报错）。
pub fn build_asr(cfg: &AsrConfig) -> Result<Arc<dyn AsrEngine>> {
    #[cfg(feature = "sherpa")]
    {
        Ok(Arc::new(
            SherpaAsr::new(cfg).context("创建 SenseVoice 识别器失败")?,
        ))
    }
    #[cfg(not(feature = "sherpa"))]
    {
        let _ = cfg;
        anyhow::bail!("本二进制未启用 `sherpa` feature，无法构建 ASR 引擎（请用 --features sherpa 编译）");
    }
}

impl Default for AsrConfig {
    fn default() -> Self {
        AsrConfig {
            engine: String::new(), // 空 → engine_id() 取默认；normalize() 会补全
            model: default_asr_model(),
            tokens: default_asr_tokens(),
            language: default_language(),
            use_itn: default_true(),
            num_threads: default_num_threads(),
            provider: default_provider(),
            max_concurrent: default_one(),
        }
    }
}
