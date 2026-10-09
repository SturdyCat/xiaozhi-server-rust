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
//! TTS 引擎存于 `RwLock`，[`Engines::refresh_from_disk`] 在**会话开始**时（以及测试台
//! `tts_test` 前）读取磁盘配置：`[tts]` 引擎签名（engine / 关键参数）变化即重建并热替换
//! ——管理页改配置保存后，无需重启即对**新会话**与测试台生效。

use crate::plugins::asr::AsrEngine;
use crate::config::Config;
use crate::plugins::host::{Phase, PluginHost};
use crate::plugins::llm::{build_llm, Llm};
use crate::plugins::memory::MemoryProvider;
use crate::plugins::registry::{self, Capability};
use crate::plugins::tts::{build_kokoro_with_lang, TtsEngine};
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
    /// 当前 TTS 引擎的签名（注册表 `signature`，见 `registry::signature_of`），热切换判定用。
    tts_sig: RwLock<String>,
    /// 按语言缓存的 TTS 引擎（sherpa 的 Kokoro `lang` 在建模时固定，
    /// 测试台切换语言时按需构建对应引擎；配置默认语言直接返回默认引擎）。
    tts_pool: Mutex<HashMap<String, Arc<dyn TtsEngine>>>,
    /// 当前生效的 LLM 引擎（与 TTS 同模式的热替换；读多写少 → RwLock）。
    llm: RwLock<Llm>,
    /// 当前 LLM 引擎签名（注册表 `signature`），热切换判定用。**勿打日志**（含密钥）。
    llm_sig: RwLock<String>,
    /// 当前生效的记忆引擎（`[memory].enabled=false` 时为 `NoopMemory`；可热替换）。
    memory: RwLock<Arc<dyn MemoryProvider>>,
    /// 当前记忆引擎签名（注册表 `signature`），热切换判定用。**勿打日志**（含密钥）。
    memory_sig: RwLock<String>,
    /// 插件宿主：能力/实现的可枚举清单 + 状态上报（`GET /api/plugins`）。
    /// 装配与前置校验都经它（见 [`crate::plugins::registry`]）。
    pub host: Arc<PluginHost>,
    /// TTS 结果缓存（跨会话共享；`[tts].cache_entries`，0=关闭）。
    pub tts_cache: crate::app::tts_cache::TtsCache,
    /// 令牌用量计量（全局 + 按会话；`GET /api/usage`）。回答"记忆注入让 prompt 长了多少"。
    pub usage: crate::app::usage::UsageMeter,
    pub config: Arc<Config>,
    /// server 启动时实际加载的配置文件路径；为 `None` 表示用的是内置默认配置，
    /// 此时 `PUT /api/config` 无法持久化（需以 `--config` 指定文件后重启）。
    pub config_path: Option<String>,
}

impl Engines {
    /// 依据配置构造引擎（无 mock：ASR/TTS 恒为真实实现，缺模型/缺 feature 直接报错）。
    ///
    /// 装配经 [`PluginHost::boot`]：能力清单/前置校验/错误文案统一由注册表声明
    /// （原先 `engine.rs` 里手写的 VAD 路径、AIUI 三要素两处硬编码校验已迁入
    /// `plugins/registry/`（`impls.rs` 的 `validate_*`）声明，并支持**一次汇总多个问题**）。
    pub fn new(config: &Config, config_path: Option<String>) -> Result<Arc<Self>> {
        let (host, booted) = PluginHost::boot(config)?;
        let tts_sig = capability_signature(config, Capability::Tts);
        let llm_sig = capability_signature(config, Capability::Llm);
        let memory_sig = capability_signature(config, Capability::Memory);
        let tts_cache = crate::app::tts_cache::TtsCache::new(config.tts.cache_entries);
        Ok(Arc::new(Self {
            asr: booted.asr,
            tts: RwLock::new(booted.tts),
            tts_sig: RwLock::new(tts_sig),
            tts_pool: Mutex::new(HashMap::new()),
            llm: RwLock::new(booted.llm),
            llm_sig: RwLock::new(llm_sig),
            memory: RwLock::new(booted.memory),
            memory_sig: RwLock::new(memory_sig),
            host,
            tts_cache,
            usage: crate::app::usage::UsageMeter::new(),
            config: Arc::new(config.clone()),
            config_path,
        }))
    }

    /// 当前生效的 LLM 引擎（每次调用取一次读锁后克隆 `Arc`，开销可忽略）。
    pub fn llm(&self) -> Llm {
        self.llm.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 当前生效的记忆引擎（未启用时是 `NoopMemory`，调用零开销）。
    pub fn memory(&self) -> Arc<dyn MemoryProvider> {
        self.memory.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// **会话级最新配置快照**：由 [`Self::refresh_from_disk`] 在会话开始时刷新。
    ///
    /// 与 [`Self::config`]（启动快照）的区别：结构型参数（`[server]` 端口/token、
    /// `[audio]` 下行协商、`[aiui]` 全链路）以启动快照为准（改动需重启，见注册表 `hot`）；
    /// 而**会话内读取**的参数（`[vad]` 阈值、`[llm].max_history`、`[tts]` 语言/发音人、
    /// 音色探测用的凭据）走这里，从而"保存 → 新会话生效"成立。
    ///
    /// ⚠️ 不要把它用于握手/OTA（那些在会话建立前就要一致，且属 restart-only）。
    pub fn live_config(&self) -> Arc<Config> {
        self.host.config()
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
        // 用会话级最新快照：改 [tts.kokoro].lang / [tts].engine 后，新会话按新配置选引擎
        let cfg = self.live_config();
        if lang.is_empty()
            || !cfg!(feature = "sherpa")
            || cfg.tts.engine_id() == "xfyun"
            || lang == cfg.tts.kokoro.lang
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
        let built = match build_kokoro_with_lang(&cfg.tts.kokoro, lang) {
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
                cfg.tts.kokoro.num_threads
            );
        }
        built
    }

    /// 从磁盘配置热刷新 TTS 引擎（**测试台 tts_test 专用**语义：只看 `[tts]`）：
    /// 读盘解析 `[tts]`，签名变化才重建并替换；失败保持当前引擎并回报原因。
    /// **阻塞调用**（sherpa 重建可能加载模型数秒），调用方须包 `spawn_blocking`。
    pub fn refresh_tts_from_disk(&self) -> Result<(), String> {
        let Some(path) = self.config_path.clone() else {
            return Ok(()); // 内置默认配置，无可刷新
        };
        let fresh = match Config::load(&path) {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("读取配置失败（保持当前 TTS 引擎）: {e}");
                self.mark_capability(Capability::Tts, Phase::Degraded, Some(msg.clone()));
                return Err(msg);
            }
        };
        self.refresh_tts(&fresh)
    }

    /// **会话开始时的统一热刷新**：按磁盘最新配置重建全部可变引擎。
    ///
    /// 覆盖 `[tts]` 与 `[llm]`（记忆接入后同处登记）。语义：
    /// - 签名（注册表 `signature`）未变 → 不重建（零开销，会话开始调用是安全的）；
    /// - 重建失败 / 配置未通过校验 → **保留旧实例继续服务**，并把原因写入插件状态
    ///   （`GET /api/plugins` 的 `phase` + `last_error`），绝不因为一次坏配置让语音不可用；
    /// - 读盘成功后同步 [`Self::live_config`] 快照，使非引擎参数（`[vad]` 阈值、
    ///   `[llm].max_history`…）对新会话生效。
    ///
    /// 返回值 = 面向日志/调用方的告警文本（空 = 一切正常）。
    /// **阻塞调用**（sherpa 重建可能加载模型数秒），调用方须包 `spawn_blocking`。
    pub fn refresh_from_disk(&self) -> Vec<String> {
        let Some(path) = self.config_path.clone() else {
            return Vec::new(); // 内置默认配置，无可刷新
        };
        let fresh = match Config::load(&path) {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("读取配置失败（保持当前引擎）: {e}");
                for cap in [Capability::Tts, Capability::Llm, Capability::Memory] {
                    self.mark_capability(cap, Phase::Degraded, Some(msg.clone()));
                }
                return vec![msg];
            }
        };
        // 配置本身可读 → 更新会话级快照（引擎重建失败也不影响这一步：
        // 用户改了 vad 阈值/历史长度，不该因为 LLM 密钥写错而一起作废）
        self.host.set_config(Arc::new(fresh.clone()));
        let mut notes = Vec::new();
        if let Err(e) = self.refresh_tts(&fresh) {
            notes.push(e);
        }
        if let Err(e) = self.refresh_llm(&fresh) {
            notes.push(e);
        }
        if let Err(e) = self.refresh_memory(&fresh) {
            notes.push(e);
        }
        notes
    }

    /// 记忆热切换（与 TTS/LLM 同模式）：签名比较 → 成功替换 / 失败保留旧实例。
    ///
    /// ⚠️ 与 `decide_refresh` 的差别：记忆是**可选能力**，`warn_if`（如"无 embedding"）
    /// 只应体现在状态上，**不能**当成"受限"而拒绝重建——否则用户改了 `recall_max_nodes`
    /// 却永远不生效。因此这里只做"硬校验 + 签名比较"。
    fn refresh_memory(&self, fresh: &Config) -> Result<(), String> {
        let Some(d) = registry::active_impl(fresh, Capability::Memory).and_then(registry::find)
        else {
            // 记忆被关掉（或 engine=none）：换回 Noop，状态由 plan 侧的标记决定
            *self.memory.write().unwrap_or_else(|e| e.into_inner()) =
                Arc::new(crate::plugins::memory::NoopMemory);
            *self
                .memory_sig
                .write()
                .unwrap_or_else(|e| e.into_inner()) = String::new();
            return Ok(());
        };
        if let Err(e) = (d.validate)(fresh) {
            let msg = format!("记忆配置未通过校验（保持当前记忆引擎）: {e:#}");
            self.mark_capability(Capability::Memory, Phase::Degraded, Some(msg.clone()));
            return Err(msg);
        }
        let sig = (d.signature)(fresh);
        let old = self
            .memory_sig
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if sig == old {
            return Ok(()); // 签名未变：零开销
        }
        match crate::plugins::memory::build_memory(&fresh.memory, &fresh.llm) {
            Ok(engine) => {
                *self.memory.write().unwrap_or_else(|e| e.into_inner()) = engine;
                *self
                    .memory_sig
                    .write()
                    .unwrap_or_else(|e| e.into_inner()) = sig;
                let warn = d.warn_if.and_then(|f| f(fresh));
                self.mark_capability(Capability::Memory, if warn.is_some() { Phase::Degraded } else { Phase::Active }, warn);
                tracing::info!("记忆引擎已热切换（新会话生效）");
                Ok(())
            }
            Err(e) => {
                let msg = format!("记忆引擎切换失败（保持当前引擎）: {e:#}");
                self.mark_capability(Capability::Memory, Phase::Degraded, Some(msg.clone()));
                Err(msg)
            }
        }
    }

    /// TTS 热切换（签名比较 → 重建 → 成功替换 / 失败保留）。
    fn refresh_tts(&self, fresh: &Config) -> Result<(), String> {
        let sig = match decide_refresh(Capability::Tts, &self.tts_sig, fresh) {
            Ok(Some(sig)) => sig,
            Ok(None) => return Ok(()), // 签名未变：零开销
            Err(msg) => {
                self.mark_capability(Capability::Tts, Phase::Failed, Some(msg.clone()));
                return Err(msg);
            }
        };
        let engine_id = fresh.tts.engine_id();
        // 构建入口**由注册表描述符提供**（新增供应商只要加 1 条描述符，本函数不再分发）。
        let Some(descriptor) = registry::active_impl(fresh, Capability::Tts).and_then(registry::find)
        else {
            let msg = format!(
                "TTS 引擎切换失败：没有匹配 [tts].engine=\"{engine_id}\" 的实现（保持当前引擎）"
            );
            self.mark_capability(Capability::Tts, Phase::Failed, Some(msg.clone()));
            return Err(msg);
        };
        let Some(build) = descriptor.build else {
            let msg = format!("TTS 实现 {} 未提供构建入口（注册表配置错误）", descriptor.id);
            self.mark_capability(Capability::Tts, Phase::Failed, Some(msg.clone()));
            return Err(msg);
        };
        let engine = match build(fresh) {
            Ok(registry::Built::Tts(engine)) => engine,
            Ok(_) => {
                let msg = format!(
                    "TTS 实现 {} 的构建结果不是 TTS 引擎（注册表配置错误）",
                    descriptor.id
                );
                self.mark_capability(Capability::Tts, Phase::Failed, Some(msg.clone()));
                return Err(msg);
            }
            Err(e) => {
                let msg = format!("TTS 引擎切换失败（保持当前引擎，engine={engine_id}）: {e:#}");
                self.mark_capability(Capability::Tts, Phase::Failed, Some(msg.clone()));
                return Err(msg);
            }
        };
        *self.tts.write().unwrap_or_else(|e| e.into_inner()) = engine;
        *self.tts_sig.write().unwrap_or_else(|e| e.into_inner()) = sig;
        // 语言池属于旧引擎（如 sherpa 的按语言缓存），清掉避免串用
        self.tts_pool.lock().unwrap_or_else(|e| e.into_inner()).clear();
        // ⚠️ 缓存里是**旧引擎**产出的下行音频：换了引擎（音色/采样率不同）后
        // 若继续命中，会播出上一个引擎的声音。故一并清空。
        self.tts_cache.clear();
        self.mark_capability(Capability::Tts, Phase::Active, None);
        tracing::info!("TTS 引擎已热切换：engine={engine_id}（新会话生效）");
        Ok(())
    }

    /// LLM 热切换：先按注册表声明的前置条件校验，**坏配置不替换**（宁可继续用旧引擎，
    /// 也不把服务切成"能跑但每次对话都失败"）。密钥为空视为坏配置（见 P3 的
    /// "空 = 保留原值"约定），只降级上报、不覆盖正在工作的引擎。
    fn refresh_llm(&self, fresh: &Config) -> Result<(), String> {
        let sig = match decide_refresh(Capability::Llm, &self.llm_sig, fresh) {
            Ok(Some(sig)) => sig,
            Ok(None) => {
                self.mark_capability(Capability::Llm, Phase::Active, None);
                return Ok(()); // 签名未变：配置健康，只是无需重建
            }
            Err(msg) => {
                // 坏配置 / 受限：**保留旧引擎**，只降级上报（服务不受影响）
                self.mark_capability(Capability::Llm, Phase::Degraded, Some(msg.clone()));
                return Err(msg);
            }
        };
        let engine = build_llm(&fresh.llm);
        *self.llm.write().unwrap_or_else(|e| e.into_inner()) = engine;
        *self.llm_sig.write().unwrap_or_else(|e| e.into_inner()) = sig;
        self.mark_capability(Capability::Llm, Phase::Active, None);
        tracing::info!("LLM 引擎已热切换：model={}（新会话生效）", fresh.llm.model);
        Ok(())
    }

    /// 把某能力的当前选中实现标记为指定阶段（状态由 `/api/plugins` 上报）。
    fn mark_capability(&self, cap: Capability, phase: Phase, err: Option<String>) {
        self.host.mark_capability(cap, phase, err);
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
        // 会话级最新快照：改 [vad] 阈值/时长后，新会话即生效（当前会话保持旧参数）
        let cfg = self.live_config();
        #[cfg(feature = "sherpa")]
        {
            Ok(Box::new(
                SherpaVad::new(&cfg.vad).context("创建 Silero VAD 失败")?,
            ))
        }
        #[cfg(not(feature = "sherpa"))]
        {
            Ok(Box::new(MockVad::new(cfg.vad.threshold)))
        }
    }
}

/// 当前配置下某能力**选中实现**的签名（由注册表 [`registry`] 声明）。
///
/// 仅用于内存内比较（热切换判定），**绝不打印**——签名可能包含密钥字段。
fn capability_signature(cfg: &Config, cap: Capability) -> String {
    // 签名口径的唯一权威在注册表（`registry::signature_of`）；会话级缓存键也走它。
    registry::signature_of(cfg, cap)
}

/// 热切换决策（**纯函数**，可在无引擎 / 无 `sherpa` feature 的 `cargo test` 中直接断言）：
///
/// - `Ok(Some(sig))` → 需按新签名重建；
/// - `Ok(None)` → 签名未变，无需重建（会话开始调用是零开销的）；
/// - `Err(reason)` → 配置不通过校验或能力受限：**保留旧实例**，`reason` 由调用方写入插件状态。
///
/// 把决策从"重建"里抽出来，是为了让"改 model 生效 / 坏配置不替换"这条 P2 验收
/// 能在没有真实引擎（也就没有 sherpa feature）的单元测试里被证明。
fn decide_refresh(
    cap: Capability,
    old_sig: &RwLock<String>,
    fresh: &Config,
) -> Result<Option<String>, String> {
    // P4：`engine` 写错 → 没有实现被选中。**必须报错**而不是 `Ok(None)`：
    // 否则保存一个新值后热刷新会静默保留旧引擎，用户以为切过去了（最糟的失败模式）。
    if registry::unknown_engine_of(fresh, cap).is_some() {
        return Err(format!(
            "{}（保持当前引擎）",
            registry::unknown_engine_hint(fresh, cap)
        ));
    }
    let Some(d) = registry::active_impl(fresh, cap).and_then(registry::find) else {
        return Ok(None);
    };
    if let Err(e) = (d.validate)(fresh) {
        return Err(format!("{} 配置未通过校验（保持当前引擎）: {e:#}", d.display));
    }
    if let Some(reason) = d.warn_if.and_then(|f| f(fresh)) {
        return Err(format!("{} 能力受限（保持当前引擎）: {reason}", d.display));
    }
    let sig = (d.signature)(fresh);
    let old = old_sig.read().unwrap_or_else(|e| e.into_inner()).clone();
    Ok(if sig == old { None } else { Some(sig) })
}

#[cfg(test)]
mod tests;
