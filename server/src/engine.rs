//! 全局共享引擎集合。ASR / TTS 模型重（数百 MB），进程内只加载一次。

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
    /// 仅 `sherpa` feature 下消费；mock 模式仅保留配置字段。
    #[allow(dead_code)]
    vad_model: String,
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
            vad_model: config.vad.model.clone(),
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
        let mut pool = self.tts_pool.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = pool.get(lang) {
            return e.clone();
        }
        let t0 = std::time::Instant::now();
        match build_tts_with_lang(&self.config.tts, lang) {
            Ok(e) => {
                tracing::info!(
                    "已构建 lang={lang} 的 TTS 引擎并缓存（耗时 {}ms，num_threads={}）",
                    t0.elapsed().as_millis(),
                    self.config.tts.num_threads
                );
                pool.insert(lang.to_string(), e.clone());
                e
            }
            Err(err) => {
                tracing::warn!("构建 lang={lang} 的 TTS 引擎失败（耗时 {}ms），回退默认引擎: {err:#}", t0.elapsed().as_millis());
                self.tts.clone()
            }
        }
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
