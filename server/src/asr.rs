//! 语音识别（ASR）引擎：`sherpa-onnx` 的 `OfflineRecognizer` +
//! `OfflineSenseVoiceModelConfig`（SenseVoice INT8）。无 mock——ASR 只有这一条真实路径。
//!
//! 注意：SenseVoice 在 sherpa-onnx 中是**离线**识别器，方案为
//! VAD 切出语音段后逐段送入识别（非 `OnlineRecognizer` 流式）。
//!
//! ⚠️ [`AsrEngine::recognize`] 是**同步阻塞的 CPU 密集调用**（SenseVoice 推理）。
//! 调用方（[`crate::session`]）必须用 `tokio::task::spawn_blocking` 隔离，
//! 否则独占 tokio worker，期间同 runtime 的其他会话/HTTP 全部卡死。

use crate::config::AsrConfig;
use anyhow::Result;
#[cfg(feature = "sherpa")]
use anyhow::Context;
use std::sync::Arc;

/// 识别引擎接口：把一段（已切分好的）单声道 f32 音频转为文本。
pub trait AsrEngine: Send + Sync {
    fn recognize(&self, samples: &[f32], sample_rate: u32) -> Result<String>;
}

/// SenseVoice INT8 离线识别器（sherpa-onnx）。
#[cfg(feature = "sherpa")]
pub struct SherpaAsr {
    recognizer: Arc<sherpa_onnx::OfflineRecognizer>,
}

#[cfg(feature = "sherpa")]
impl SherpaAsr {
    pub fn new(cfg: &AsrConfig) -> Result<Self> {
        use sherpa_onnx::{
            OfflineModelConfig, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig,
        };
        let mut model_config = OfflineModelConfig::default();
        model_config.tokens = Some(cfg.tokens.clone());
        model_config.num_threads = cfg.num_threads as i32;
        model_config.provider = Some(cfg.provider.clone());
        model_config.sense_voice = OfflineSenseVoiceModelConfig {
            model: Some(cfg.model.clone()),
            language: Some(cfg.language.clone()),
            use_itn: cfg.use_itn,
        };
        let config = OfflineRecognizerConfig {
            model_config,
            ..Default::default()
        };
        let recognizer = sherpa_onnx::OfflineRecognizer::create(&config)
            .context("创建 SenseVoice 识别器失败（检查模型路径或原生库）")?;
        Ok(Self {
            recognizer: Arc::new(recognizer),
        })
    }
}

#[cfg(feature = "sherpa")]
impl AsrEngine for SherpaAsr {
    fn recognize(&self, samples: &[f32], sample_rate: u32) -> Result<String> {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(sample_rate as i32, samples);
        self.recognizer.decode(&stream);
        match stream.get_result() {
            Some(r) => Ok(r.text),
            None => anyhow::bail!("SenseVoice 识别失败（无返回结果）"),
        }
    }
}

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
