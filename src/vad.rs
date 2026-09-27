//! 语音活动检测（VAD）引擎抽象。
//!
//! - [`MockVad`]：能量阈值切分，无模型，用于本地联调。
//! - `--features sherpa` 时 [`SherpaVad`] 使用 `sherpa-onnx`
//!   `VoiceActivityDetector` + Silero VAD。
//!
//! VAD 在服务器端运行：把连续上行音频切分为有效语音段，再送 ASR。

#[cfg(feature = "sherpa")]
use anyhow::Result;
#[cfg(feature = "sherpa")]
use crate::config::VadConfig;
#[cfg(feature = "sherpa")]
use anyhow::Context;

/// VAD 引擎接口。每会话持有一个独立实例（内部有状态）。
pub trait VadEngine: Send {
    /// 喂入一帧单声道 f32 音频；检测到的完整语音段通过 `cb` 回调传出。
    fn accept(&mut self, samples: &[f32], cb: &mut dyn FnMut(Vec<f32>));
    /// 输入结束时 flush 残留语音段。
    fn flush(&mut self, cb: &mut dyn FnMut(Vec<f32>));
}

/// 无模型能量阈值实现，用于 `mock` 模式。
pub struct MockVad {
    threshold: f32,
}

impl MockVad {
    pub fn new(threshold: f32) -> Self {
        Self { threshold }
    }
}

impl VadEngine for MockVad {
    fn accept(&mut self, samples: &[f32], cb: &mut dyn FnMut(Vec<f32>)) {
        let n = samples.len().max(1) as f32;
        let energy = samples.iter().map(|s| s * s).sum::<f32>() / n;
        // 阈值同时用作能量门限（本环境无真实标定，仅作占位切分）。
        if energy > self.threshold.max(0.0001) {
            cb(samples.to_vec());
        }
    }

    fn flush(&mut self, _cb: &mut dyn FnMut(Vec<f32>)) {
        // mock：无残留语音段
    }
}

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
            cb(seg.samples.clone());
            self.detector.pop();
        }
    }

    fn flush(&mut self, cb: &mut dyn FnMut(Vec<f32>)) {
        self.detector.flush();
        while let Some(seg) = self.detector.front() {
            cb(seg.samples.clone());
            self.detector.pop();
        }
    }
}
