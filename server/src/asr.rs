//! 语音识别（ASR）引擎抽象。
//!
//! - 默认实现 [`MockAsr`]：无需模型，回显固定文本，便于本地联调协议。
//! - `--features sherpa` 时 [`SherpaAsr`] 使用 `sherpa-onnx` 的
//!   `OfflineRecognizer` + `OfflineSenseVoiceModelConfig`（SenseVoice INT8）。
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

/// 无模型回显实现，用于 `asr.backend = "mock"` 或默认配置。
pub struct MockAsr;

impl AsrEngine for MockAsr {
    fn recognize(&self, _samples: &[f32], _sample_rate: u32) -> Result<String> {
        Ok("这是本地语音助手的测试识别结果。".to_string())
    }
}

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

/// 根据配置构造 ASR 引擎。
pub fn build_asr(cfg: &AsrConfig) -> Result<Arc<dyn AsrEngine>> {
    if cfg.is_sherpa() {
        #[cfg(feature = "sherpa")]
        {
            let engine = SherpaAsr::new(cfg).context("创建 SenseVoice 识别器失败")?;
            return Ok(Arc::new(engine));
        }
        #[cfg(not(feature = "sherpa"))]
        {
            anyhow::bail!("backend=sherpa 但当前未启用 `sherpa` feature，请用 --features sherpa 编译");
        }
    }
    Ok(Arc::new(MockAsr))
}
