//! Silero VAD（sherpa-onnx）——VAD 插件的本地实现。
//! `#[cfg(feature = "sherpa")]`：仅在启用 sherpa feature 的构建中存在。

use super::VadConfig;
use anyhow::{Context, Result};

use super::VadEngine;

#[cfg(feature = "sherpa")]
pub struct SherpaVad {
    detector: sherpa_onnx::VoiceActivityDetector,
}

#[cfg(feature = "sherpa")]
impl SherpaVad {
    pub fn new(cfg: &VadConfig) -> Result<Self> {
        use sherpa_onnx::{SileroVadModelConfig, VadModelConfig};
        let mut silero = SileroVadModelConfig::default();
        silero.model = Some(cfg.model.clone());
        silero.threshold = cfg.threshold;
        silero.min_silence_duration = cfg.min_silence_duration;
        silero.min_speech_duration = cfg.min_speech_duration;

        let mut vad_cfg = VadModelConfig::default();
        vad_cfg.silero_vad = silero;
        vad_cfg.sample_rate = 16000;
        vad_cfg.num_threads = 1;
        vad_cfg.provider = Some("cpu".to_string());

        let detector = sherpa_onnx::VoiceActivityDetector::create(&vad_cfg, 30.0)
            .context("创建 Silero VAD 失败（检查模型路径或原生库）")?;
        Ok(Self { detector })
    }
}

#[cfg(feature = "sherpa")]
impl VadEngine for SherpaVad {
    fn accept(&mut self, samples: &[f32], cb: &mut dyn FnMut(Vec<f32>)) {
        self.detector.accept_waveform(samples);
        while let Some(seg) = self.detector.front() {
            cb(seg.samples().to_vec());
            self.detector.pop();
        }
    }

    fn flush(&mut self, cb: &mut dyn FnMut(Vec<f32>)) {
        self.detector.flush();
        while let Some(seg) = self.detector.front() {
            cb(seg.samples().to_vec());
            self.detector.pop();
        }
    }
}
