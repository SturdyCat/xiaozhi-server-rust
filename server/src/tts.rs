//! 语音合成（TTS）引擎：`sherpa-onnx` `OfflineTts` +
//! `OfflineTtsKokoroModelConfig`（Kokoro INT8）。无 mock——TTS 只有这一条真实路径。
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

/// Kokoro INT8 离线合成器（sherpa-onnx）。
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

/// sherpa 真引擎构造（两入口共用）：集中 `SherpaTts::new` 与报错文案，避免重复。
#[cfg(feature = "sherpa")]
fn build_sherpa_tts(cfg: &TtsConfig) -> Result<Arc<dyn TtsEngine>> {
    let engine = SherpaTts::new(cfg).context("创建 Kokoro TTS 失败")?;
    Ok(Arc::new(engine))
}

/// 根据配置构造 TTS 引擎（仅 `sherpa` 一条路径；未启用 feature 直接报错）。
pub fn build_tts(cfg: &TtsConfig) -> Result<Arc<dyn TtsEngine>> {
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

/// 按指定语言构造 TTS 引擎（测试台的多语言切换用）。
/// Kokoro 的 `lang` 在建模时固定，切语言 = 重建引擎（engine.rs 按语言缓存）。
pub fn build_tts_with_lang(cfg: &TtsConfig, lang: &str) -> Result<Arc<dyn TtsEngine>> {
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
        anyhow::bail!("本二进制未启用 `sherpa` feature，无法构建 TTS 引擎（请用 --features sherpa 编译）");
    }
}
