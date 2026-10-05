//! 语音合成（TTS）引擎：本地 `sherpa-onnx` Kokoro INT8（`backend = "sherpa"`）
//! 或科大讯飞在线合成（`backend = "xfyun"`，见 [`crate::xfyun_tts`]）。无 mock。
//!
//! 合成结果为单声道 f32 PCM；下行前由音频层做（按需）重采样与 Opus 编码。
//!
//! ## 流式接口
//! [`TtsEngine::synthesize_stream`] 是**首选入口**：合成过程中按片回调 PCM（增量），
//! 调用方边收边下发，显著降低首字延迟（讯飞为服务端分片返回；Kokoro 为逐段回调）。
//! [`TtsEngine::synthesize`] 保留给需要整段的场景，默认按流式收集实现（反之亦然）。
//!
//! ⚠️ 两者都是**同步阻塞调用**（本地推理实测 8~20s；在线为网络等待），调用方
//!（[`crate::session`]）必须用 `spawn_blocking` 隔离，否则独占 tokio worker，
//! 期间同 runtime 的其他会话/HTTP 全部卡死。

use crate::config::{TtsBackendKind, TtsConfig};
use anyhow::Result;
#[cfg(feature = "sherpa")]
use anyhow::Context;
use std::sync::Arc;

/// 流式分片回调：`(输出采样率, 本片增量 PCM)`；返回 `false` 表示调用方要求取消
///（如打断/连接断开），引擎应尽快停止合成并返回 `Ok(())`。
pub type TtsChunkCallback = Box<dyn FnMut(u32, &[f32]) -> bool + Send>;

/// 合成引擎接口：文本 → 单声道 f32 PCM + 采样率。
/// `speaker` 为语音角色（Kokoro sid / 讯飞忽略），由调用方按请求传入。
pub trait TtsEngine: Send + Sync {
    /// 流式合成（**引擎必须实现**）：按片回调**增量** PCM（非累计），返回时合成结束。
    /// 回调返回 `false` = 调用方取消（打断/断开），引擎应尽快停止并返回 `Ok(())`。
    fn synthesize_stream(
        &self,
        text: &str,
        speed: f32,
        speaker: i32,
        chunk_cb: TtsChunkCallback,
    ) -> Result<()>;

    /// 整段合成（默认：收集 [`Self::synthesize_stream`] 的输出）。
    /// 当前调用方均走流式；保留为接口完整性（如离线批量合成）。
    #[allow(dead_code)]
    fn synthesize(&self, text: &str, speed: f32, speaker: i32) -> Result<(Vec<f32>, u32)> {
        // 回调要求 'static（且引擎实现内部可能再 move）：状态放共享容器，外部读取
        let collected: Arc<std::sync::Mutex<(Vec<f32>, u32)>> =
            Arc::new(std::sync::Mutex::new((Vec::new(), 0)));
        let sink = collected.clone();
        self.synthesize_stream(
            text,
            speed,
            speaker,
            Box::new(move |rate, chunk| {
                let mut c = sink.lock().unwrap_or_else(|e| e.into_inner());
                c.1 = rate;
                c.0.extend_from_slice(chunk);
                true
            }),
        )?;
        let out = std::mem::take(&mut *collected.lock().unwrap_or_else(|e| e.into_inner()));
        Ok(out)
    }

    /// 合成输出采样率（下发链路的重采样源采样率）。
    fn output_sample_rate(&self) -> u32 {
        24_000
    }

    /// 引擎标识（测试台 tts_test 结果回报用）：`sherpa` / `xfyun`。
    fn name(&self) -> &'static str {
        "unknown"
    }
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

/// sherpa 真引擎构造（两入口共用）：集中 `SherpaTts::new` 与报错文案，避免重复。
#[cfg(feature = "sherpa")]
fn build_sherpa_tts(cfg: &TtsConfig) -> Result<Arc<dyn TtsEngine>> {
    let engine = SherpaTts::new(cfg).context("创建 Kokoro TTS 失败")?;
    Ok(Arc::new(engine))
}

/// 根据配置构造 TTS 引擎：`backend` 选择 `sherpa`（本地 Kokoro）或 `xfyun`（讯飞在线）。
pub fn build_tts(cfg: &TtsConfig) -> Result<Arc<dyn TtsEngine>> {
    match cfg.backend_kind() {
        TtsBackendKind::Xfyun => Ok(Arc::new(crate::xfyun_tts::XfyunTts::new(&cfg.xfyun)?)),
        TtsBackendKind::Sherpa => build_sherpa(cfg),
    }
}

/// 按指定语言构造 TTS 引擎（测试台的多语言切换用；仅对 Kokoro 有意义）。
/// Kokoro 的 `lang` 在建模时固定，切语言 = 重建引擎（engine.rs 按语言缓存）；
/// 讯飞在线引擎由 `[tts.xfyun].voice` 决定音色，忽略 `lang`。
pub fn build_tts_with_lang(cfg: &TtsConfig, lang: &str) -> Result<Arc<dyn TtsEngine>> {
    match cfg.backend_kind() {
        TtsBackendKind::Xfyun => build_tts(cfg),
        TtsBackendKind::Sherpa => {
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
                let _ = lang;
                anyhow::bail!("本二进制未启用 `sherpa` feature，无法构建 TTS 引擎（请用 --features sherpa 编译）");
            }
        }
    }
}

/// sherpa（本地 Kokoro）分支：未启用 feature 直接报错。
fn build_sherpa(cfg: &TtsConfig) -> Result<Arc<dyn TtsEngine>> {
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
