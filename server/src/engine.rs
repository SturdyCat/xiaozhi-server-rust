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
//! `cpus:"3.5"` 进一步限核。详见 [`crate::app::session`] 与各引擎模块注释。
//!
//! ## TTS 语言池与热切换
//! Kokoro 的 `lang` 在建模时固定，故 [`Engines::tts_for`] 按语言按需构建并缓存
//! 引擎（首次约数秒），配置默认语言直接返回默认引擎。
//! TTS 引擎存于 `RwLock`，[`Engines::refresh_tts_from_disk`] 在会话开始 / 测试台 tts_test
//! 时读取磁盘配置：`[tts]` 引擎签名（backend / 关键参数）变化即重建并热替换——
//! 管理页改配置保存后，无需重启即对**新会话**与测试台生效。

use crate::plugins::asr::{build_asr, AsrEngine};
use crate::config::Config;
use crate::plugins::llm::{build_llm, Llm};
use crate::plugins::tts::{build_tts, build_tts_with_lang, TtsEngine};
#[cfg(not(feature = "sherpa"))]
use crate::plugins::vad::MockVad;
use crate::plugins::vad::VadEngine;
#[cfg(feature = "sherpa")]
use {
    crate::plugins::vad::SherpaVad,
    anyhow::Context,
};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

/// 进程内共享的引擎（ASR / TTS / LLM）与每会话 VAD 工厂。
pub struct Engines {
    pub asr: Arc<dyn AsrEngine>,
    /// 当前生效的 TTS 引擎（可热替换；读多写少 → RwLock）。
    tts: RwLock<Arc<dyn TtsEngine>>,
    /// 当前 TTS 引擎的签名（`TtsConfig::engine_signature`），热切换判定用。
    tts_sig: RwLock<String>,
    /// 按语言缓存的 TTS 引擎（sherpa 的 Kokoro `lang` 在建模时固定，
    /// 测试台切换语言时按需构建对应引擎；配置默认语言直接返回默认引擎）。
    tts_pool: Mutex<HashMap<String, Arc<dyn TtsEngine>>>,
    pub llm: Llm,
    /// TTS 结果缓存（跨会话共享；`[tts].cache_entries`，0=关闭）。
    pub tts_cache: crate::app::tts_cache::TtsCache,
    pub config: Arc<Config>,
    /// server 启动时实际加载的配置文件路径；为 `None` 表示用的是内置默认配置，
    /// 此时 `PUT /api/config` 无法持久化（需以 `--config` 指定文件后重启）。
    pub config_path: Option<String>,
}

impl Engines {
    /// 依据配置构造引擎（无 mock：ASR/TTS 恒为真实实现，缺模型/缺 feature 直接报错）。
    pub fn new(config: &Config, config_path: Option<String>) -> Result<Arc<Self>> {
        // 启动期快速失败：sherpa 构建下 VAD 模型路径必填（切句依赖它）。
        #[cfg(feature = "sherpa")]
        if config.vad.model.is_empty() {
            anyhow::bail!("[vad].model 未配置（Silero VAD 模型路径，如 /data/models/silero_vad.onnx）");
        }
        // AIUI 全链路模式启用时三要素必填（缺一即快速失败，不静默回退级联）。
        if config.aiui.enabled
            && (config.aiui.appid.trim().is_empty()
                || config.aiui.api_key.trim().is_empty()
                || config.aiui.api_secret.trim().is_empty())
        {
            anyhow::bail!("[aiui].enabled 需要 appid/api_key/api_secret（AIUI 平台应用三要素）");
        }
        let asr = build_asr(&config.asr)?;
        let tts = build_tts(&config.tts)?;
        let tts_sig = config.tts.engine_signature();
        let llm = build_llm(&config.llm);
        let tts_cache = crate::app::tts_cache::TtsCache::new(config.tts.cache_entries);
        Ok(Arc::new(Self {
            asr,
            tts: RwLock::new(tts),
            tts_sig: RwLock::new(tts_sig),
            tts_pool: Mutex::new(HashMap::new()),
            llm,
            tts_cache,
            config: Arc::new(config.clone()),
            config_path,
        }))
    }

    /// 默认 TTS 引擎（服务器配置语言；设备流水线路径用）。
    pub fn tts(&self) -> Arc<dyn TtsEngine> {
        self.tts.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 按语言获取 TTS 引擎：已缓存直接返回；否则按需构建并缓存（首次约数秒）。
    /// 构建失败时回退默认引擎并告警。配置默认语言直接返回默认引擎。
    /// 讯飞在线引擎与语言无关（音色由 `[tts.xfyun].voice` 决定），直接返回默认引擎。
    pub fn tts_for(&self, lang: &str) -> Arc<dyn TtsEngine> {
        let lang = lang.trim();
        if lang.is_empty()
            || !cfg!(feature = "sherpa")
            || self.config.tts.backend_kind() == crate::config::TtsBackendKind::Xfyun
            || lang == self.config.tts.lang
        {
            return self.tts();
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
                return self.tts();
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

    /// 从磁盘配置热刷新 TTS 引擎（会话开始 / 测试台 tts_test 前调用）：
    /// 读盘解析 `[tts]`，引擎签名（backend / 关键参数）变化才重建并替换。
    /// 未指定配置文件或读盘失败时保持现状（日志 warn）。**阻塞调用**（sherpa 重建
    /// 可能加载模型数秒），调用方须包 `spawn_blocking`。
    pub fn refresh_tts_from_disk(&self) -> Result<(), String> {
        let Some(path) = &self.config_path else {
            return Ok(()); // 内置默认配置，无可刷新
        };
        let fresh = match Config::load(path) {
            Ok(c) => c,
            Err(e) => return Err(format!("读取配置失败（保持当前 TTS 引擎）: {e}")),
        };
        let sig = fresh.tts.engine_signature();
        if sig == *self.tts_sig.read().unwrap_or_else(|e| e.into_inner()) {
            return Ok(());
        }
        let kind = fresh.tts.backend_kind();
        match build_tts(&fresh.tts) {
            Ok(engine) => {
                *self.tts.write().unwrap_or_else(|e| e.into_inner()) = engine;
                *self.tts_sig.write().unwrap_or_else(|e| e.into_inner()) = sig;
                // 语言池属于旧引擎（如 sherpa 的按语言缓存），清掉避免串用
                self.tts_pool.lock().unwrap_or_else(|e| e.into_inner()).clear();
                tracing::info!("TTS 引擎已热切换：backend={}", kind.as_str());
                Ok(())
            }
            Err(e) => Err(format!(
                "TTS 引擎切换失败（保持当前引擎，backend={}）: {e:#}",
                kind.as_str()
            )),
        }
    }

    /// 当前 TTS 引擎标识（测试台结果回报用）。
    pub fn tts_name(&self) -> &'static str {
        self.tts().name()
    }

    /// 为每个会话创建一个独立的 VAD 实例（内部有状态）。
    ///
    /// sherpa 构建恒为 Silero VAD（`[vad].model` 已在启动期校验非空）；
    /// 未启用 feature 的编译返回占位实现（仅供检查/测试编译，生产不含）。
    pub fn new_vad(&self) -> Result<Box<dyn VadEngine>> {
        #[cfg(feature = "sherpa")]
        {
            Ok(Box::new(
                SherpaVad::new(&self.config.vad).context("创建 Silero VAD 失败")?,
            ))
        }
        #[cfg(not(feature = "sherpa"))]
        {
            Ok(Box::new(MockVad::new(self.config.vad.threshold)))
        }
    }
}
