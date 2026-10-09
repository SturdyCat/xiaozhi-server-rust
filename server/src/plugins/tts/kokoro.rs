//! Kokoro INT8 离线合成器（sherpa-onnx）——TTS 插件的本地实现。
//! `#[cfg(feature = "sherpa")]`：仅在启用 sherpa feature 的构建中存在。

use super::KokoroTtsConfig;
use anyhow::{Context, Result};
use std::sync::Arc;

use super::{TtsChunkCallback, TtsEngine};

/// Kokoro INT8 离线合成器（sherpa-onnx）。
#[cfg(feature = "sherpa")]
pub struct SherpaTts {
    tts: Arc<sherpa_onnx::OfflineTts>,
}

#[cfg(feature = "sherpa")]
impl SherpaTts {
    pub fn new(cfg: &KokoroTtsConfig) -> Result<Self> {
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
    /// 流式合成：sherpa-onnx 的进度回调给出的是**累计**样本（"samples generated so far"），
    /// 这里 diff 出新增部分；并按 ~100ms 攒批再回调，避免逐 step 的碎片化分片；
    /// `progress >= 1.0` 时冲刷尾批。回调返回 `false` 时透传给底层以尽早停止合成。
    fn synthesize_stream(
        &self,
        text: &str,
        speed: f32,
        speaker: i32,
        chunk_cb: TtsChunkCallback,
    ) -> Result<()> {
        use sherpa_onnx::GenerationConfig;
        let gen = GenerationConfig {
            sid: speaker,
            speed,
            ..Default::default()
        };
        let sr = self.tts.sample_rate().max(0) as u32;
        // 攒批门槛按实际采样率折算 ~100ms（下限 1，防御 sr=0）
        let flush_at = (sr as usize / 10).max(1);

        // 进度回调要求 'static：状态放进共享容器，闭包与外部各持一份
        #[derive(Default)]
        struct StreamState {
            pending: Vec<f32>,
            consumed: usize,
            cancelled: bool,
        }
        // 回调（要求 'static）与外部收尾都要用：放进共享容器
        let cb = Arc::new(std::sync::Mutex::new(chunk_cb));
        let state = Arc::new(std::sync::Mutex::new(StreamState::default()));
        let state_cb = state.clone();
        let cb_cb = cb.clone();
        let audio = self.tts.generate_with_config(
            text,
            &gen,
            Some(move |samples: &[f32], progress: f32| {
                // 取新增区间（先算切片再进锁，避免同时可变/不可变借用）
                let mut s = state_cb.lock().unwrap_or_else(|e| e.into_inner());
                if samples.len() > s.consumed {
                    let new_part = &samples[s.consumed..];
                    s.pending.extend_from_slice(new_part);
                    s.consumed = samples.len();
                }
                if s.pending.len() >= flush_at || progress >= 1.0 {
                    if !s.pending.is_empty() {
                        let chunk = std::mem::take(&mut s.pending);
                        drop(s); // 释放状态锁后再回调（回调方可能耗时）
                        let mut cb = cb_cb.lock().unwrap_or_else(|e| e.into_inner());
                        if !cb(sr, &chunk) {
                            let mut s = state_cb.lock().unwrap_or_else(|e| e.into_inner());
                            s.cancelled = true;
                            return false;
                        }
                    }
                }
                true
            }),
        );

        let (cancelled, consumed) = {
            let s = state.lock().unwrap_or_else(|e| e.into_inner());
            (s.cancelled, s.consumed)
        };
        if cancelled {
            // 调用方取消（打断/断开）：正常返回，不视为错误
            return Ok(());
        }
        match audio {
            Some(a) => {
                // 防御：个别实现可能不把 progress 推到 1（或压根不回调），补齐剩余样本
                let total = a.samples().len();
                if consumed < total {
                    let rest = &a.samples()[consumed..];
                    if !rest.is_empty() {
                        let mut cb = cb.lock().unwrap_or_else(|e| e.into_inner());
                        if !cb(sr, rest) {
                            return Ok(());
                        }
                    }
                }
                Ok(())
            }
            None => anyhow::bail!("TTS 合成失败（检查 Kokoro 模型文件完整性）"),
        }
    }

    fn output_sample_rate(&self) -> u32 {
        self.tts.sample_rate().max(1) as u32
    }

    fn name(&self) -> &'static str {
        "sherpa"
    }
}
