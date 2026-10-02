//! 语音合成（TTS）引擎抽象。
//!
//! - [`MockTts`]：生成静音 PCM，无模型，用于本地联调下行链路。
//! - `--features sherpa` 时 [`SherpaTts`] 使用 `sherpa-onnx`
//!   `OfflineTts` + `OfflineTtsKokoroModelConfig`（Kokoro INT8）。
//!
//! 合成结果为单声道 f32 PCM；下行前由音频层做（按需）重采样与 Opus 编码。
//!
//! ⚠️ [`TtsEngine::synthesize`] 是**同步阻塞的 CPU 密集调用**（Kokoro 推理，
//! 实测单次 8~20s）。调用方（[`crate::session`]）必须用 `spawn_blocking` 隔离，
//! 否则独占 tokio worker，期间同 runtime 的其他会话/HTTP 全部卡死。

use crate::config::TtsConfig;
use anyhow::Result;
#[cfg(feature = "sherpa")]
use anyhow::Context;
use std::sync::Arc;

/// 合成引擎接口：文本 → 单声道 f32 PCM + 采样率。
/// `speaker` 为语音角色（Kokoro sid），由调用方按请求传入。
pub trait TtsEngine: Send + Sync {
    fn synthesize(&self, text: &str, speed: f32, speaker: i32) -> Result<(Vec<f32>, u32)>;
}

/// 无模型实现，生成 0.5s 静音（24k），便于本地联调。
pub struct MockTts;

impl TtsEngine for MockTts {
    fn synthesize(&self, _text: &str, _speed: f32, _speaker: i32) -> Result<(Vec<f32>, u32)> {
        let sr = 24_000u32;
        let n = (sr as f32 * 0.5) as usize;
        Ok((vec![0.0; n], sr))
    }
}

#[cfg(feature = "sherpa")]
pub struct SherpaTts {
    tts: Arc<sherpa_onnx::OfflineTts>,
}

#[cfg(feature = "sherpa")]
impl SherpaTts {
    pub fn new(cfg: &TtsConfig) -> Result<Self> {
        use sherpa_onnx::{
            OfflineTtsConfig, OfflineTtsKokoroModelConfig, OfflineTtsModelConfig,
        };
        let mut model_config = OfflineTtsModelConfig::default();
        model_config.kokoro = OfflineTtsKokoroModelConfig {
            model: Some(cfg.model.clone()),
            voices: Some(cfg.voices.clone()),
            tokens: Some(cfg.tokens.clone()),
            data_dir: Some(cfg.data_dir.clone()),
            dict_dir: Some(cfg.dict_dir.clone()),
            lexicon: Some(cfg.lexicon.clone()),
            length_scale: 1.0,
            lang: Some(cfg.lang.clone()),
        };
        model_config.num_threads = cfg.num_threads as i32;
        let config = OfflineTtsConfig {
            model: model_config,
            ..Default::default()
        };
        let tts = sherpa_onnx::OfflineTts::create(&config)
            .context("创建 Kokoro TTS 失败（检查模型路径或原生库）")?;
        Ok(Self { tts: Arc::new(tts) })
    }
}

#[cfg(feature = "sherpa")]
impl TtsEngine for SherpaTts {
    fn synthesize(&self, text: &str, speed: f32, speaker: i32) -> Result<(Vec<f32>, u32)> {
        use sherpa_onnx::GenerationConfig;
        let gen = GenerationConfig {
            sid: speaker,
            speed,
            ..Default::default()
        };
        let audio = self
            .tts
            .generate_with_config::<fn(&[f32], f32) -> bool>(text, &gen, None);
        match audio {
            Some(a) => {
                let sr = a.sample_rate() as u32;
                Ok((a.samples().to_vec(), sr))
            }
            None => anyhow::bail!("TTS 合成失败（检查 Kokoro 模型文件完整性）"),
        }
    }
}

/// 根据配置构造 TTS 引擎。
pub fn build_tts(cfg: &TtsConfig) -> Result<Arc<dyn TtsEngine>> {
    if cfg.is_sherpa() {
        #[cfg(feature = "sherpa")]
        {
            let engine = SherpaTts::new(cfg).context("创建 Kokoro TTS 失败")?;
            return Ok(Arc::new(engine));
        }
        #[cfg(not(feature = "sherpa"))]
        {
            anyhow::bail!("backend=sherpa 但当前未启用 `sherpa` feature，请用 --features sherpa 编译");
        }
    }
    Ok(Arc::new(MockTts))
}

/// 按指定语言构造 TTS 引擎（网页测试台的多语言切换用）。
/// 非 sherpa 后端时与 `build_tts` 等价（mock 与语言无关）。
pub fn build_tts_with_lang(cfg: &TtsConfig, lang: &str) -> Result<Arc<dyn TtsEngine>> {
    if cfg.is_sherpa() {
        #[cfg(feature = "sherpa")]
        {
            let mut c = cfg.clone();
            c.lang = lang.to_string();
            let engine = SherpaTts::new(&c)
                .with_context(|| format!("创建 lang={lang} 的 Kokoro TTS 失败"))?;
            return Ok(Arc::new(engine));
        }
        #[cfg(not(feature = "sherpa"))]
        {
            let _ = lang;
            anyhow::bail!("backend=sherpa 但当前未启用 `sherpa` feature，请用 --features sherpa 编译");
        }
    }
    Ok(Arc::new(MockTts))
}
