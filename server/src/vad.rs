//! 语音活动检测（VAD）引擎：`sherpa-onnx` `VoiceActivityDetector` + Silero VAD。
//!
//! [`MockVad`]（能量阈值占位）仅存在于**未启用 `sherpa` feature 的编译**中
//! （供 `cargo check/test` 编译通过）；生产二进制恒为 [`SherpaVad`]，无 mock。
//!
//! VAD 在服务器端运行：把连续上行音频切分为有效语音段，再送 ASR。
//!
//! ⚠️ [`VadEngine::accept`] 每帧调用，Silero 推理为**同步 CPU 调用**；虽单次轻量，
//! 但每帧都跑，调用方（[`crate::session`]）在解码后的音频循环里直接同步调用即可
//! （不似 ASR/TTS 那样单次耗时数秒，无需 `spawn_blocking`）。

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

/// 能量阈值占位实现——**仅未启用 `sherpa` feature 的编译**（测试/检查用），生产不含。
#[cfg(not(feature = "sherpa"))]
pub struct MockVad {
    threshold: f32,
}

#[cfg(not(feature = "sherpa"))]
impl MockVad {
    pub fn new(threshold: f32) -> Self {
        Self { threshold }
    }
}

#[cfg(not(feature = "sherpa"))]
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
        // 占位：无残留语音段
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
