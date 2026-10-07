//! 语音活动检测（VAD）引擎：`sherpa-onnx` `VoiceActivityDetector` + Silero VAD。
//!
//! [`MockVad`]（能量阈值占位）仅存在于**未启用 `sherpa` feature 的编译**中
//! （供 `cargo check/test` 编译通过）；生产二进制恒为 [`SherpaVad`]，无 mock。
//!
//! VAD 在服务器端运行：把连续上行音频切分为有效语音段，再送 ASR。
//!
//! ⚠️ [`VadEngine::accept`] 每帧调用，Silero 推理为**同步 CPU 调用**；虽单次轻量，
//! 但每帧都跑，调用方（[`crate::app::session`]）在解码后的音频循环里直接同步调用即可
//! （不似 ASR/TTS 那样单次耗时数秒，无需 `spawn_blocking`）。

use serde::{Deserialize, Serialize};
/// VAD 引擎接口。每会话持有一个独立实例（内部有状态）。
pub trait VadEngine: Send {
    /// 喂入一帧单声道 f32 音频；检测到的完整语音段通过 `cb` 回调传出。
    fn accept(&mut self, samples: &[f32], cb: &mut dyn FnMut(Vec<f32>));
    /// 输入结束时 flush 残留语音段。
    fn flush(&mut self, cb: &mut dyn FnMut(Vec<f32>));
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VadConfig {
    /// Silero VAD 模型路径；为空视为未配置（sherpa 构建下引擎初始化会报错）。
    #[serde(default = "default_vad_model")]
    pub model: String,
    #[serde(default = "default_threshold")]
    pub threshold: f32,
    #[serde(default = "default_min_silence")]
    pub min_silence_duration: f32,
    #[serde(default = "default_min_speech")]
    pub min_speech_duration: f32,
}

impl VadConfig {
    /// 模型路径非空即视为配置了真实 VAD。
    #[allow(dead_code)]
    pub fn is_real(&self) -> bool {
        !self.model.is_empty()
    }
}
fn default_vad_model() -> String {
    "/data/models/silero_vad.onnx".into()
}

fn default_threshold() -> f32 {
    0.5
}

fn default_min_silence() -> f32 {
    0.25
}

fn default_min_speech() -> f32 {
    0.25
}

#[cfg(feature = "sherpa")]
mod silero;

#[cfg(feature = "sherpa")]
pub use silero::SherpaVad;

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

impl Default for VadConfig {
    fn default() -> Self {
        VadConfig {
            model: default_vad_model(),
            threshold: default_threshold(),
            min_silence_duration: default_min_silence(),
            min_speech_duration: default_min_speech(),
        }
    }
}
