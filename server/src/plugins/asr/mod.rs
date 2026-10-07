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

/// 识别引擎接口：把一段（已切分好的）单声道 f32 音频转为文本。
pub trait AsrEngine: Send + Sync {
    fn recognize(&self, samples: &[f32], sample_rate: u32) -> Result<String>;
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AsrConfig {
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
            model: default_asr_model(),
            tokens: default_asr_tokens(),
            language: default_language(),
            use_itn: default_true(),
            num_threads: default_num_threads(),
            provider: default_provider(),
        }
    }
}
