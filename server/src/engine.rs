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
//! **推理闸门**（缺口①，[`Self::inference`]）：上述预算只在"每能力同时 **1 路**推理"时成立；
//! `spawn_blocking` 本身无并发上限，多设备同时说话即超订。ASR/TTS 各一把
//! `Semaphore`（`[asr]/[tts].max_concurrent`，默认 1）包在全部推理调用点外，
//! 忙时排队而不是压垮 CPU——见 [`crate::app::inference`] 模块注释。
//!
//! ## TTS 语言池与热切换
//! Kokoro 的 `lang` 在建模时固定，故 [`Engines::tts_for`] 按语言按需构建并缓存
//! 引擎（首次约数秒），配置默认语言直接返回默认引擎。
//! TTS 引擎存于 `RwLock`，[`Engines::refresh_from_disk`] 在**会话开始**时（以及测试台
//! `tts_test` 前）读取磁盘配置：`[tts]` 引擎签名（engine / 关键参数）变化即重建并热替换
//! ——管理页改配置保存后，无需重启即对**新会话**与测试台生效。

use crate::config::store::ConfigStore;
use crate::config::Config;
use crate::plugins::asr::AsrEngine;
use crate::plugins::host::{Phase, PluginHost};
use crate::plugins::llm::{build_llm, Llm};
use crate::plugins::memory::MemoryProvider;
use crate::plugins::nlu::{bert_signature, build_bert_checked, BertClassify, NluConfig};
use crate::plugins::registry::{self, Capability};
#[cfg(not(feature = "sherpa"))]
use crate::plugins::vad::MockVad;
use crate::plugins::vad::VadEngine;
use anyhow::Result;
use std::sync::{Arc, Mutex, RwLock};
#[cfg(feature = "sherpa")]
use {crate::plugins::vad::SherpaVad, anyhow::Context};

/// 进程内共享的引擎（ASR / TTS / LLM）与每会话 VAD 工厂。
pub struct Engines {
    pub asr: Arc<dyn AsrEngine>,
    /// 实例与实际身份的原子快照（生产会话固定持有租约）。
    tts: tts::TtsSlot,
    /// 整个读盘刷新串行化；必须在读配置之前加锁。
    refresh_lock: Mutex<()>,
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
    ///
    /// `Arc`：预取 worker（独立 task）也要读写它，见 [`crate::app::prefetch`]。
    pub tts_cache: Arc<crate::app::tts_cache::TtsCache>,
    /// 持久化**热音频**库（可复现合成的完整句成品音频，跨进程复用）。
    ///
    /// 与 `tts_cache` 的分工见 [`crate::app::hot_audio`]：内存缓存"什么都能装"且每进程
    /// 一份；这里落盘的是**生产预取路径逐句登记身份**的完整句——配置固定文本（唤醒问候语 /
    /// 告别语 / 技能应答 / 兜底话术）复用率最高，普通回复同样入缓存。AIUI 云闭环与测试台
    /// 直连合成不可复现，不落盘。有界（文件数 / 总量 / 单句上限）、可预期、容器重建后仍命中。
    pub hot_audio: Arc<crate::app::hot_audio::HotAudioStore>,
    /// 令牌用量计量（全局 + 按会话；`GET /api/usage`）。回答"记忆注入让 prompt 长了多少"。
    pub usage: crate::app::usage::UsageMeter,
    /// 设备 / 会话 / 最近对话的进程内注册表
    /// （`GET /api/devices`、`/api/sessions`、`/api/conversations`、`/api/stats`）。
    ///
    /// 与 [`Self::usage`]、[`Self::mcp`] 同性质：**只读观测面**，不参与任何流水线决策。
    /// 全部内存态，进程重启即清空（设备重连后自动回填）——理由见 `app/registry` 模块注释。
    pub registry: crate::app::registry::Registry,
    /// 轻量事件总线（P0-5）：会话在关键节点 `emit` 进程级事件（会话生命周期 / 消息入对话 /
    /// TTS 轮次 / 异常）。**叠加观测层**，与 `registry` 的 `note_*` 埋点并存，不是替换；
    /// 订阅者（大盘 / 审计，属 P1）独立消费，会话发布时**完全不感知**谁在听。
    pub event_bus: crate::app::events::EventBus,
    /// VAD 实例池（P0-3）：复用昂贵的 Silero ONNX 实例（每会话借还，`acquire_vad`）。
    /// 配置签名变了自动失效；详情见 `plugins/vad/pool` 模块注释。
    pub vad_pool: crate::plugins::vad::pool::VadPool,
    /// 延迟/事件聚合（P1-1）：事件总线的常驻订阅者写入，`GET /api/metrics` 只读投影。
    /// 全内存态、重启清零；见 `app/metrics` 模块注释。
    pub metrics: crate::app::metrics::Metrics,
    /// 设备 MCP 工具注册表（按 `device_id` 存最近一次能力发现的结果；`GET /api/mcp/tools`）。
    /// 工具清单只存在于固件里且**只有设备连着时才问得到**，故必须落下来给管理界面看。
    pub mcp: crate::app::mcp::registry::McpRegistry,
    /// 会话注入注册表（P1-2）：活跃会话注册 `mpsc::Sender`，HTTP 端点据此投递文本
    /// （`POST /api/devices/{id}/inject`）；见 `app/inject` 模块注释。
    pub inject: crate::app::inject::Injector,
    /// 推理并发闸门（缺口①）：ASR/TTS 各一把 Semaphore，包在全部 `spawn_blocking`
    /// 推理调用点外（会话 / 预取 worker / 测试台共用）；见 `app/inference` 模块注释。
    pub inference: crate::app::inference::InferenceGates,
    pub config: Arc<Config>,
    /// 配置存储后端（读、写、落点三件事的唯一入口，见 [`crate::config::store`]）。
    /// 读取/写回配置**一律**经它——不要直接 `Config::load(路径)`，否则换成 SQLite 后端时会被绕过。
    pub config_store: Arc<dyn ConfigStore>,
    /// TinyBERT 分类器跨会话缓存（`[nlu].bert_*`）：键 = 配置签名（[`bert_signature`]），
    /// 变了即重建；构建失败缓存 `None`（每会话不重试加载大文件，测试台 `nlu_test`
    /// 直调 [`build_bert_checked`] 以拿到完整错误）。加载是同步 IO（会话开始路径），
    /// 但仅在首会话/配置变化时发生一次，稳态零开销。
    bert: Mutex<Option<(String, Option<Arc<dyn BertClassify>>)>>,
}

mod refresh;
mod tts;
use refresh::decide_refresh;
pub use tts::TtsLease;

impl Engines {
    /// 依据配置构造引擎（无 mock：ASR/TTS 恒为真实实现，缺模型/缺 feature 直接报错）。
    ///
    /// 装配经 [`PluginHost::boot`]：能力清单/前置校验/错误文案统一由注册表声明
    /// （原先 `engine.rs` 里手写的 VAD 路径、AIUI 三要素两处硬编码校验已迁入
    /// `plugins/registry/`（`impls.rs` 的 `validate_*`）声明，并支持**一次汇总多个问题**）。
    pub fn new(config: &Config, config_store: Arc<dyn ConfigStore>) -> Result<Arc<Self>> {
        let (host, booted) = PluginHost::boot(config)?;
        let llm_sig = capability_signature(config, Capability::Llm);
        let memory_sig = capability_signature(config, Capability::Memory);
        // 跟随配置来源的持久卷；无来源时复用 memory.db_path 的父目录，
        // 不把生产音频写到硬编码的容器层路径。与 wakeup 共用旧 hot-audio/ 格式。
        let hot_audio_dir =
            crate::app::hot_audio::cache_dir(config_store.location(), &config.memory.db_path);
        let hot_audio = Arc::new(crate::app::hot_audio::HotAudioStore::new(Some(
            hot_audio_dir,
        )));
        let tts_cache = Arc::new(crate::app::tts_cache::TtsCache::with_store(
            config.tts.cache_entries,
            hot_audio.clone(),
        ));
        Ok(Arc::new(Self {
            asr: booted.asr,
            tts: tts::TtsSlot::new(config, booted.tts),
            refresh_lock: Mutex::new(()),
            llm: RwLock::new(booted.llm),
            llm_sig: RwLock::new(llm_sig),
            memory: RwLock::new(booted.memory),
            memory_sig: RwLock::new(memory_sig),
            host,
            tts_cache,
            hot_audio,
            usage: crate::app::usage::UsageMeter::new(),
            registry: crate::app::registry::Registry::new(),
            event_bus: crate::app::events::EventBus::new(1024),
            vad_pool: crate::plugins::vad::pool::VadPool::new(),
            metrics: crate::app::metrics::Metrics::new(),
            mcp: crate::app::mcp::registry::McpRegistry::new(),
            inject: crate::app::inject::Injector::new(),
            inference: crate::app::inference::InferenceGates::new(config),
            config: Arc::new(config.clone()),
            config_store,
            bert: Mutex::new(None),
        }))
    }

    /// 配置持久化落点（今天的 TOML 路径；将来 SQLite 后端为 DB 路径）。
    ///
    /// `None` = 内置默认配置启动 → `PUT /api/config` 与设备 overlay / 预设都无法持久化。
    /// overlay / presets / hot-audio 的落点派生与"能否持久化"判定都读这里。
    pub fn config_path(&self) -> Option<&str> {
        self.config_store.location()
    }

    /// 取（或按需构建）跨会话共享的 TinyBERT 分类器；未配置 / 构建失败返回 `None`
    /// （失败只 warn 一次并缓存，避免每会话重复加载大文件的延迟与日志噪音）。
    pub fn bert_engine(&self, nlu: &NluConfig) -> Option<Arc<dyn BertClassify>> {
        if !nlu.bert_configured() {
            return None;
        }
        let sig = bert_signature(nlu);
        let mut guard = self.bert.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached_sig, engine)) = guard.as_ref() {
            if *cached_sig == sig {
                return engine.clone();
            }
        }
        let built = match build_bert_checked(nlu) {
            Ok(engine) => Some(engine),
            Err(e) => {
                tracing::warn!(
                    "TinyBERT 引擎构建失败（技能级联回落启发式打分；测试台 nlu_test 可见完整错误）: {e:#}"
                );
                None
            }
        };
        *guard = Some((sig, built.clone()));
        built
    }

    /// 当前生效的 LLM 引擎（每次调用取一次读锁后克隆 `Arc`，开销可忽略）。
    pub fn llm(&self) -> Llm {
        self.llm.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 当前生效的记忆引擎（未启用时是 `NoopMemory`，调用零开销）。
    pub fn memory(&self) -> Arc<dyn MemoryProvider> {
        self.memory
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
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

    /// **每设备生效配置** = 全局 `live_config()` ⊕ 该设备的 overlay（仅白名单字段）。
    ///
    /// overlay 只含会话内参数（tts 音色/语速、llm 提示词与历史长度、soul 人设、
    /// wakeup/command 话术、memory 召回参数），**永不触碰引擎签名**——叠加后各引擎的
    /// `signature_of` 不变，故共享 TTS/LLM/Memory 引擎仍按全局配置构建、不重建。
    ///
    /// 无 `Device-Id`（unknown）/ 无 overlay / 内置默认配置 → 直接返回全局快照，
    /// 与现状完全一致（零成本契约）。读 overlay 失败（坏数据 / 越界字段）防御性回退全局。
    ///
    /// ⚠️ 会话开始时算一次存入 `Session.eff_config` 即可；overlay 是会话级生效，
    /// 运行中改 overlay 只对**下一次**会话生效（与全局部分更新同源）。
    ///
    /// overlay 存储经 [`crate::config::store::ConfigStore`]：SQLite 后端进 `cfg_device_overlays`
    /// 表，文件后端落 `overlays/devices/<id>.toml`（两种后端语义一致）。
    pub fn effective_config(&self, device_id: &str) -> Arc<Config> {
        let global = self.host.config();
        if device_id.is_empty() || device_id == "unknown" {
            return global;
        }
        let overlay = match self.config_store.load_overlay(device_id) {
            Ok(Some(mut overlay)) => {
                crate::config::overlay::prune_disallowed(&mut overlay);
                overlay
            }
            Ok(None) => return global,
            Err(e) => {
                tracing::warn!("读取设备 {device_id} overlay 失败，用全局配置: {e}");
                return global;
            }
        };
        match crate::config::overlay::merge_overlay(&global, &overlay) {
            Ok(c) => Arc::new(c),
            Err(e) => {
                tracing::warn!("合并设备 {device_id} overlay 失败，用全局配置: {e}");
                global
            }
        }
    }

    /// 从 VAD 池借出实例（P0-3）：空闲复用 / 池满等待 / `[vad]` 配置变更自动失效。
    /// [`plugins/vad/pool`] 的 `VadGuard` Drop 时归还（RAII，含提前返回与 panic 展开）。
    pub async fn acquire_vad(&self) -> Result<crate::plugins::vad::pool::VadGuard> {
        let cfg = self.live_config();
        // 配置签名：参与 `SherpaVad::new` 的全部字段（变了即池内旧实例失效）。
        let sig = format!(
            "{}|{}|{}|{}|{}",
            cfg.vad.engine,
            cfg.vad.model,
            cfg.vad.threshold,
            cfg.vad.min_silence_duration,
            cfg.vad.min_speech_duration,
        );
        #[cfg(feature = "sherpa")]
        let factory = || -> Result<Box<dyn VadEngine>> {
            Ok(Box::new(
                SherpaVad::new(&cfg.vad).context("创建 Silero VAD 失败")?,
            ))
        };
        #[cfg(not(feature = "sherpa"))]
        let factory =
            || -> Result<Box<dyn VadEngine>> { Ok(Box::new(MockVad::new(cfg.vad.threshold))) };
        self.vad_pool.acquire(sig, factory).await
    }
}

/// 当前配置下某能力**选中实现**的签名（由注册表 [`registry`] 声明）。
///
/// 仅用于内存内比较（热切换判定），**绝不打印**——签名可能包含密钥字段。
fn capability_signature(cfg: &Config, cap: Capability) -> String {
    // 签名口径的唯一权威在注册表（`registry::signature_of`）；会话级缓存键也走它。
    registry::signature_of(cfg, cap)
}

#[cfg(test)]
mod tests;
