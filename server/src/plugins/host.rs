//! 插件宿主：装配、校验前移、状态上报（`docs/plugin-architecture-unification.md` §3.2）。
//!
//! ## 职责边界（P1/M0+M1）
//!
//! - **做**：按 [`crate::plugins::registry::REGISTRY`] 枚举能力 → 校验前置条件（错误文案含「怎么修」）
//!   → 构建必需能力 → 维护 `enabled` / `phase` / `last_error` 状态快照。
//! - **不做**（P2 才接管）：热切换。P1 只如实上报"改了配置但还没重启"这件事，
//!   不改变任何运行期行为——这是本步能安全落地的前提。
//!
//! ## 两条硬规则
//!
//! 1. **必需 vs 可选分级**：`Asr/Vad/Tts/Llm` 失败即启动失败（对齐现状）；
//!    其余能力坏掉只降级（[`Phase::Degraded`]），**不能拖垮语音服务**。
//! 2. **状态与选择分离**：`enabled`（用户配置里选中的实现）与 `phase`（实际激活状态）
//!    分开上报，UI 才能显示「已启用但加载失败」——只报一个布尔值必然误导
//!    （对齐 DSH 的 `enabled` vs `fiberPhase`）。

use std::sync::{Arc, RwLock};

use anyhow::Result;

use crate::config::Config;
use crate::plugins::registry::{
    active_impl, impl_id, unknown_engine_hint, unknown_engine_of, Built, Capability, HotReload,
    Requirement, ALL_CAPABILITIES, REGISTRY,
};
use crate::plugins::asr::AsrEngine;
use crate::plugins::llm::Llm;
use crate::plugins::tts::TtsEngine;

/// 插件生命周期状态（对齐 DSH 的 `FiberState`，只保留对用户有意义的几档）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// 配置里没选中该实现（正常情况，不是错误）。
    Disabled,
    /// 已构建并可用。
    Active,
    /// 可用但能力受限/依赖缺失（可选能力）；语音服务继续工作。
    Degraded,
    /// 构建失败或必需条件不满足（必需能力出现该状态 = 启动已失败）。
    Failed,
}

impl Phase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Phase::Disabled => "disabled",
            Phase::Active => "active",
            Phase::Degraded => "degraded",
            Phase::Failed => "failed",
        }
    }
    /// 是否属于"偏离正常"（UI 只在这些状态打标，避免一片绿色噪音）。
    pub const fn is_deviation(self) -> bool {
        matches!(self, Phase::Degraded | Phase::Failed)
    }
}

/// 单个实现的状态快照。
#[derive(Clone, Debug)]
pub struct PluginStatus {
    pub id: &'static str,
    pub capability: Capability,
    pub display: &'static str,
    pub local: bool,
    pub hot: HotReload,
    /// 用户配置里是否选中了它（与 `phase` 分开，见模块文档规则 2）。
    pub enabled: bool,
    pub phase: Phase,
    pub requires: &'static [Requirement],
    /// 失败/降级原因（面向用户，含「怎么修」）。**不含签名，避免密钥经状态接口外泄**。
    pub last_error: Option<String>,
}

impl PluginStatus {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "capability": self.capability.id(),
            "capability_display": self.capability.display(),
            "display": self.display,
            "local": self.local,
            "hot": self.hot.as_str(),
            "hot_hint": self.hot.hint(),
            "enabled": self.enabled,
            "phase": self.phase.as_str(),
            "deviation": self.phase.is_deviation(),
            "requires": self.requires.iter().map(|r| r.as_str()).collect::<Vec<_>>(),
            "last_error": self.last_error,
        })
    }
}

/// 装配产物：必需能力的共享实例 + 可选能力的共享实例。
pub struct Booted {
    pub asr: Arc<dyn AsrEngine>,
    pub tts: Arc<dyn TtsEngine>,
    pub llm: Llm,
    /// 图记忆（未启用时为 [`crate::plugins::memory::NoopMemory`]）。
    pub memory: Arc<dyn crate::plugins::memory::MemoryProvider>,
}

/// 插件宿主：注册表的状态侧（构建/校验/上报）。
pub struct PluginHost {
    /// **当前生效配置快照**：启动时为 boot 配置，[`Self::set_config`] 在会话热刷新后更新。
    /// 会话内读取的参数（VAD 阈值、LLM 历史长度、TTS 语言/发音人、探测凭据）走它，
    /// 结构型参数（server/audio/aiui）仍以启动快照为准。
    cfg: RwLock<Arc<Config>>,
    statuses: RwLock<Vec<PluginStatus>>,
}

impl PluginHost {
    /// **只规划不构建**：算出每个实现的 `enabled`/`phase`（依据注册表的 `validate`）。
    ///
    /// 不触碰任何引擎 ctor ⇒ 可在无 `sherpa` feature 的 `cargo test` 里断言全部状态逻辑，
    /// 也用于"能力清单"端点。必需能力校验失败时**不**在此报错（由 [`Self::boot`] 统一报）。
    pub fn plan(cfg: &Config) -> Arc<Self> {
        let mut statuses: Vec<PluginStatus> = REGISTRY
            .iter()
            .map(|d| {
                let enabled = d.is_selected(cfg);
                let (phase, last_error) = if !enabled {
                    (Phase::Disabled, None)
                } else {
                    match (d.validate)(cfg) {
                        Ok(()) => {
                            // 软性问题（能构建但能力受限）→ Degraded + 原因，而不是静默 Active
                            match d.warn_if.and_then(|f| f(cfg)) {
                                Some(reason) => (Phase::Degraded, Some(reason)),
                                None => (Phase::Active, None),
                            }
                        }
                        Err(e) => {
                            let msg = format!("{e:#}");
                            if d.capability.fatal_if_enabled() {
                                (Phase::Failed, Some(msg))
                            } else {
                                // 可选能力：坏掉只降级，语音服务照常
                                (Phase::Degraded, Some(msg))
                            }
                        }
                    }
                };
                PluginStatus {
                    id: d.id,
                    capability: d.capability,
                    display: d.display,
                    local: d.local,
                    hot: d.hot,
                    enabled,
                    phase,
                    requires: d.requires,
                    last_error,
                }
            })
            .collect();
        // P4：`engine` 写成未知值时**没有任何实现会被选中**——显式标 Failed 并给出可选值，
        // 否则 UI 上表现为"该能力整体禁用"，用户会误以为是自己关掉的。
        for &cap in ALL_CAPABILITIES {
            if unknown_engine_of(cfg, cap).is_none() {
                continue;
            }
            let hint = unknown_engine_hint(cfg, cap);
            for s in statuses.iter_mut().filter(|s| s.capability == cap) {
                s.phase = Phase::Failed;
                s.last_error = Some(hint.clone());
            }
        }
        Arc::new(Self {
            cfg: RwLock::new(Arc::new(cfg.clone())),
            statuses: RwLock::new(statuses),
        })
    }

    /// **装配**：先 [`Self::plan`]，再构建必需能力。
    ///
    /// - 必需能力的前置校验失败 → 汇总**全部**问题一次性报出（避免"修一个报一个"）；
    /// - 必需能力构建失败 → 直接失败（对齐既有 `Engines::new` 语义）；
    /// - **有共享实例的可选能力**（当前只有 `memory`）也参与构建，但失败只降级：
    ///   记原因 → `Phase::Degraded`，服务照常启动（记忆坏了不能让语音不可用）；
    /// - 其余可选能力不参与构建（`voices`/`firmware` 是 app 层服务；`aiui` 每会话构造）。
    pub fn boot(cfg: &Config) -> Result<(Arc<Self>, Booted)> {
        let host = Self::plan(cfg);

        // 1) 前置校验汇总（把散落的硬编码校验统一到这里，文案含「怎么修」）
        let mut problems: Vec<String> = Vec::new();
        // 0. 先查 `engine` 写错：此时**没有任何实现被选中**，下面逐描述符的校验一条都不会跑，
        //    必须单独报（否则只会得到"没有可用的 X 实现（注册表/配置异常）"这种不可行动的报错）。
        for &cap in ALL_CAPABILITIES {
            if cap.fatal_if_enabled() && unknown_engine_of(cfg, cap).is_some() {
                problems.push(unknown_engine_hint(cfg, cap));
            }
        }
        for d in REGISTRY {
            if !d.capability.fatal_if_enabled() || !d.is_selected(cfg) {
                continue;
            }
            if let Err(e) = (d.validate)(cfg) {
                problems.push(format!("[{}] {e:#}", d.id));
            }
        }
        if !problems.is_empty() {
            anyhow::bail!(
                "必需能力配置校验未通过（{} 项）：\n  - {}",
                problems.len(),
                problems.join("\n  - ")
            );
        }

        // 2) 构建必需能力（按能力槽位收集；VAD 无共享实例，每会话构造）
        let mut asr: Option<Arc<dyn AsrEngine>> = None;
        let mut tts: Option<Arc<dyn TtsEngine>> = None;
        let mut llm: Option<Llm> = None;
        for d in REGISTRY {
            if !d.capability.required() || !d.is_selected(cfg) {
                continue;
            }
            let Some(build) = d.build else { continue };
            match build(cfg) {
                Ok(Built::Asr(e)) => {
                    if asr.is_some() {
                        anyhow::bail!("注册表错误：能力 asr 有多个被选中的实现（{}）", d.id);
                    }
                    asr = Some(e);
                }
                Ok(Built::Tts(e)) => {
                    if tts.is_some() {
                        anyhow::bail!("注册表错误：能力 tts 有多个被选中的实现（{}）", d.id);
                    }
                    tts = Some(e);
                }
                Ok(Built::Llm(e)) => {
                    if llm.is_some() {
                        anyhow::bail!("注册表错误：能力 llm 有多个被选中的实现（{}）", d.id);
                    }
                    llm = Some(e);
                }
                Ok(Built::FullChain(_)) => { /* 每会话构造，不占共享槽位 */ }
                // 可选能力的共享实例不在这里构建（见 `Self::build_optional`）
                Ok(Built::Memory(_)) => {}
                Err(e) => {
                    let msg = format!("{e:#}");
                    host.mark(d.id, Phase::Failed, Some(msg.clone()));
                    anyhow::bail!("构建 {} 失败: {msg}", d.id);
                }
            }
        }

        // 3) 可选能力的共享实例（记忆）：失败只降级，并把原因写进状态供 /api/plugins 上报
        let memory: Arc<dyn crate::plugins::memory::MemoryProvider> = match Self::build_optional(
            &host,
            cfg,
            Capability::Memory,
        ) {
            Some(Built::Memory(m)) => {
                tracing::info!("记忆引擎已装配：{}", m.name());
                m
            }
            _ => Arc::new(crate::plugins::memory::NoopMemory),
        };

        let booted = Booted {
            asr: asr.ok_or_else(|| no_impl_err(Capability::Asr))?,
            tts: tts.ok_or_else(|| no_impl_err(Capability::Tts))?,
            llm: llm.ok_or_else(|| no_impl_err(Capability::Llm))?,
            memory,
        };
        Ok((host, booted))
    }

    /// 构建某**可选**能力的共享实例：失败不致命，只把原因写进状态（`Degraded`）。
    ///
    /// 与必需能力的分工：必需能力走 [`Self::boot`] 的"校验汇总 → 构建 → 失败即退出"；
    /// 可选能力只要能降级就不该阻止启动（记忆配错一个路径不该让音箱不能用）。
    pub fn build_optional(
        host: &Arc<Self>,
        cfg: &Config,
        cap: Capability,
    ) -> Option<Built> {
        let d = REGISTRY
            .iter()
            .find(|d| d.capability == cap && d.is_selected(cfg))?;
        let build = d.build?;
        if let Err(e) = (d.validate)(cfg) {
            let msg = format!("{e:#}");
            tracing::warn!("可选能力 {} 配置不通过（已降级）: {msg}", d.id);
            host.mark(d.id, Phase::Degraded, Some(msg));
            return None;
        }
        match build(cfg) {
            Ok(built) => Some(built),
            Err(e) => {
                let msg = format!("{e:#}");
                tracing::warn!("可选能力 {} 构建失败（已降级）: {msg}", d.id);
                host.mark(d.id, Phase::Degraded, Some(msg));
                None
            }
        }
    }

    /// 当前生效的配置快照（启动快照，或最近一次 [`Self::set_config`]）。
    pub fn config(&self) -> Arc<Config> {
        self.cfg.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 更新配置快照（会话热刷新时由 `Engines::refresh_from_disk` 调用）。
    ///
    /// 只换快照，**不重建引擎**：引擎重建由 `Engines` 按签名决定，两者职责分开。
    pub fn set_config(&self, cfg: Arc<Config>) {
        *self.cfg.write().unwrap_or_else(|e| e.into_inner()) = cfg;
    }

    /// 更新某实现的阶段与原因（构建失败、降级等）。
    pub fn mark(&self, id: &str, phase: Phase, last_error: Option<String>) {
        let mut guard = self.statuses.write().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = guard.iter_mut().find(|s| s.id == id) {
            s.phase = phase;
            s.last_error = last_error;
        }
    }

    /// 全部实现的状态快照（顺序同注册表）。
    pub fn snapshot(&self) -> Vec<PluginStatus> {
        self.statuses
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 某能力当前被选中的实现 id。
    pub fn active_impl(&self, cap: Capability) -> Option<&'static str> {
        active_impl(&self.config(), cap)
    }

    /// 把某能力**当前选中的实现**标记为指定阶段（热切换成功/失败、降级等统一入口）。
    pub fn mark_capability(&self, cap: Capability, phase: Phase, err: Option<String>) {
        if let Some(id) = self.active_impl(cap) {
            self.mark(id, phase, err);
        }
    }

    /// 按能力聚合的状态（`GET /api/plugins` 的 `capabilities[]`）。
    ///
    /// 能力级 `phase` 取该能力下"最坏"的实现状态：Active < Degraded < Failed；
    /// 全部 Disabled 时为 Disabled。
    pub fn capabilities_json(&self) -> serde_json::Value {
        let statuses = self.snapshot();
        let caps: Vec<serde_json::Value> = ALL_CAPABILITIES
            .iter()
            .map(|&cap| {
                let impls: Vec<&PluginStatus> =
                    statuses.iter().filter(|s| s.capability == cap).collect();
                let phase = impls
                    .iter()
                    .map(|s| s.phase)
                    .fold(None::<Phase>, |acc, p| match (acc, p) {
                        (None, p) => Some(p),
                        (Some(Phase::Failed), _) | (_, Phase::Failed) => Some(Phase::Failed),
                        (Some(Phase::Active), Phase::Degraded)
                        | (Some(Phase::Degraded), Phase::Active) => Some(Phase::Degraded),
                        (Some(a), Phase::Disabled) => Some(a),
                        (Some(_), p) => Some(p),
                    })
                    .unwrap_or(Phase::Disabled);
                let active = impls.iter().find(|s| s.phase == Phase::Active).map(|s| s.id);
                let enabled = impls.iter().any(|s| s.enabled);
                let last_error = impls
                    .iter()
                    .find_map(|s| s.last_error.as_ref())
                    .cloned();
                serde_json::json!({
                    "id": cap.id(),
                    "display": cap.display(),
                    "required": cap.required(),
                    "implemented": cap.implemented(),
                    "enabled": enabled,
                    "phase": phase.as_str(),
                    "deviation": phase.is_deviation(),
                    "active_impl": active,
                    "hot": impls.first().map(|s| s.hot.as_str()).unwrap_or("restart_only"),
                    "last_error": last_error,
                    "implementations": impls.iter().map(|s| s.to_json()).collect::<Vec<_>>(),
                })
            })
            .collect();

        let count = |p: Phase| {
            statuses
                .iter()
                .filter(|s| s.phase == p)
                .count()
        };
        serde_json::json!({
            "capabilities": caps,
            "counts": {
                "active": count(Phase::Active),
                "degraded": count(Phase::Degraded),
                "failed": count(Phase::Failed),
                "disabled": count(Phase::Disabled),
                "total": statuses.len(),
            },
        })
    }
}

/// 「没有可用实现」的可行动报错：列出该能力**全部可用实现 id**（= `engine` 的合法取值）。
fn no_impl_err(cap: Capability) -> anyhow::Error {
    let known: Vec<&str> = REGISTRY
        .iter()
        .filter(|d| d.capability == cap)
        .map(|d| impl_id(d.id))
        .collect();
    anyhow::anyhow!(
        "能力 {} 没有可用实现（注册表/配置异常）：可选 engine = {}。检查对应配置段是否被清空。",
        cap.id(),
        known.join(" / ")
    )
}

/// 供端点复用的实现数上限断言辅助（避免注册表被误改成某能力多实现同时可选）。
#[cfg(test)]
mod tests;
