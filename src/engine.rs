//! 全局共享引擎集合。ASR / TTS 模型重（数百 MB），进程内只加载一次。

use crate::asr::{build_asr, AsrEngine};
use crate::config::Config;
use crate::llm::{build_llm, Llm};
use crate::tts::{build_tts, TtsEngine};
use crate::vad::{MockVad, VadEngine};
#[cfg(feature = "sherpa")]
use crate::vad::SherpaVad;
use anyhow::Result;
use std::sync::Arc;

/// 进程内共享的引擎（ASR / TTS / LLM）与每会话 VAD 工厂。
pub struct Engines {
    pub asr: Arc<dyn AsrEngine>,
    pub tts: Arc<dyn TtsEngine>,
    pub llm: Llm,
    vad_model: String,
    vad_threshold: f32,
    pub config: Arc<Config>,
}

impl Engines {
    /// 依据配置构造引擎；`asr/tts.backend = "sherpa"` 时加载真实模型。
    pub fn new(config: &Config) -> Result<Arc<Self>> {
        let asr = build_asr(&config.asr)?;
        let tts = build_tts(&config.tts)?;
        let llm = build_llm(&config.llm);
        Ok(Arc::new(Self {
            asr,
            tts,
            llm,
            vad_model: config.vad.model.clone(),
            vad_threshold: config.vad.threshold,
            config: Arc::new(config.clone()),
        }))
    }

    /// 为每个会话创建一个独立的 VAD 实例（内部有状态）。
    pub fn new_vad(&self) -> Box<dyn VadEngine> {
        #[cfg(feature = "sherpa")]
        {
            if !self.vad_model.is_empty() {
                if let Ok(v) = SherpaVad::new(&self.config.vad) {
                    return Box::new(v);
                }
            }
        }
        Box::new(MockVad::new(self.vad_threshold))
    }
}
