//! 进程级共享引擎集合。ASR / TTS 模型重（数百 MB），进程内只加载一次（`Arc<dyn Trait>`），
//! 所有会话共享；VAD 是流式有状态对象，按会话用 [`Engines::new_vad`] 单独创建。
//!
//! ## 并发与 CPU 预算（低功耗主机关键）
//!
//! 单个 WS 连接 = 一个 tokio task（session），引擎跨会话共享；音频编解码为纯函数、无共享可变状态。
//! 重推理（ASR 识别 / TTS 合成）是**同步阻塞的 CPU 密集调用**，调用方（session）必须用
//! `spawn_blocking` 隔离，否则独占 tokio worker，期间同 runtime 的其他会话/HTTP 全部卡死
//! （实测单次 TTS 8~20s）。峰值 CPU 占用预算（N5105 等 4 核小主机）：
//! tokio worker（默认 2）+ ASR 识别 2 线程 + TTS 合成（[tts].num_threads，默认 4）+ VAD 1 线程，
//! 各段错峰执行，控制在 4 核以内为其他服务留余量；容器侧由 `docker-compose.yml` 的
//! `cpus:"3.5"` 进一步限核。详见 [`crate::session`] 与各引擎模块注释。
//!
//! ## TTS 语言池
//! Kokoro 的 `lang` 在建模时固定，故 [`Engines::tts_for`] 按语言按需构建并缓存引擎
//! （首次约数秒），`mock` 与配置默认语言直接返回 [`Engines::tts`] 默认引擎。

use crate::asr::{build_asr, AsrEngine};
use crate::config::Config;
use crate::llm::{build_llm, Llm};
use crate::tts::{build_tts, build_tts_with_lang, TtsEngine};
use crate::vad::{MockVad, VadEngine};
#[cfg(feature = "sherpa")]
use crate::vad::SherpaVad;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// 进程内共享的引擎（ASR / TTS / LLM）与每会话 VAD 工厂。
pub struct Engines {
    pub asr: Arc<dyn AsrEngine>,
    tts: Arc<dyn TtsEngine>,
    /// 按语言缓存的 TTS 引擎（sherpa 的 Kokoro `lang` 在建模时固定，
    /// 网页测试台切换语言时按需构建对应引擎；mock 与语言无关）。
    tts_pool: Mutex<HashMap<String, Arc<dyn TtsEngine>>>,
    pub llm: Llm,
    vad_threshold: f32,
    pub config: Arc<Config>,
    /// server 启动时实际加载的配置文件路径；为 `None` 表示用的是内置默认（mock）配置，
    /// 此时 `PUT /api/config` 无法持久化（需以 `--config` 指定文件后重启）。
    pub config_path: Option<String>,
}

impl Engines {
    /// 依据配置构造引擎；`asr/tts.backend = "sherpa"` 时加载真实模型。
    pub fn new(config: &Config, config_path: Option<String>) -> Result<Arc<Self>> {
        let asr = build_asr(&config.asr)?;
        let tts = build_tts(&config.tts)?;
        let llm = build_llm(&config.llm);
        Ok(Arc::new(Self {
            asr,
            tts,
            tts_pool: Mutex::new(HashMap::new()),
            llm,
            vad_threshold: config.vad.threshold,
            config: Arc::new(config.clone()),
            config_path,
        }))
    }

    /// 默认 TTS 引擎（服务器配置语言；设备流水线路径用）。
    pub fn tts(&self) -> Arc<dyn TtsEngine> {
        self.tts.clone()
    }

    /// 按语言获取 TTS 引擎：已缓存直接返回；否则按需构建并缓存（首次约数秒）。
    /// 构建失败时回退默认引擎并告警。mock 后端与语言无关，直接返回默认。
    pub fn tts_for(&self, lang: &str) -> Arc<dyn TtsEngine> {
        let lang = lang.trim();
        if lang.is_empty()
            || !cfg!(feature = "sherpa")
            || self.config.tts_is_mock()
            || lang == self.config.tts.lang
        {
            return self.tts.clone();
        }
        // 先查缓存（持锁）；命中直接返回，未命中立即释放锁，
        // 避免下面的模型加载（数秒）在持锁状态下阻塞其他会话的 tts_for。
        {
            let pool = self.tts_pool.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(e) = pool.get(lang) {
                return e.clone();
            }
        }
        let t0 = std::time::Instant::now();
        let built = match build_tts_with_lang(&self.config.tts, lang) {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!("构建 lang={lang} 的 TTS 引擎失败（耗时 {}ms），回退默认引擎: {err:#}", t0.elapsed().as_millis());
                return self.tts.clone();
            }
        };
        // 重新加锁写入；插入前再查一次，避免并发构建重复加载同一模型。
        let mut pool = self.tts_pool.lock().unwrap_or_else(|e| e.into_inner());
        if pool.get(lang).is_none() {
            pool.insert(lang.to_string(), built.clone());
            tracing::info!(
                "已构建 lang={lang} 的 TTS 引擎并缓存（耗时 {}ms，num_threads={}）",
                t0.elapsed().as_millis(),
                self.config.tts.num_threads
            );
        }
        built
    }

    /// 为每个会话创建一个独立的 VAD 实例（内部有状态）。
    ///
    /// VAD 与 ASR/TTS 独立判定：模型路径非空即启用真实 VAD（见 `VadConfig::is_real`）。
    pub fn new_vad(&self) -> Box<dyn VadEngine> {
        #[cfg(feature = "sherpa")]
        {
            if self.config.vad.is_real() {
                if let Ok(v) = SherpaVad::new(&self.config.vad) {
                    return Box::new(v);
                }
            }
        }
        Box::new(MockVad::new(self.vad_threshold))
    }
}
