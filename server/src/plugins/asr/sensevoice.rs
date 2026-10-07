//! SenseVoice INT8 离线识别器（sherpa-onnx）——ASR 插件的本地实现。
//! `#[cfg(feature = "sherpa")]`：仅在启用 sherpa feature 的构建中存在。

use super::AsrConfig;
use anyhow::Result;
#[cfg(feature = "sherpa")]
use anyhow::Context;
use std::sync::Arc;

use super::AsrEngine;

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

