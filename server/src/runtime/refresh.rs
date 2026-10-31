//! 配置热刷新与非 TTS 能力的重建。读盘到发布串行，避免旧读盘快照晚发布。
use super::*;

/// 写入 `RwLock` 并吞掉毒化错误（全模块一致的处理口径；毒化意味此前已有 panic，
/// 此时返回内部值比整体崩溃更稳妥）。
fn write_locked<T>(lock: &RwLock<T>, value: T) {
    *lock.write().unwrap_or_else(|e| e.into_inner()) = value;
}

impl Engines {
    /// 从磁盘配置热刷新 TTS 引擎（**测试台 tts_test 专用**语义：只看 `[tts]`）：
    /// 读盘解析 `[tts]`，签名变化才重建并替换；失败保持当前引擎并回报原因。
    /// **阻塞调用**（sherpa 重建可能加载模型数秒），调用方须包 `spawn_blocking`。
    pub fn refresh_tts_from_disk(&self) -> Result<(), String> {
        if self.dev_mode {
            return Err(DEV_UNAVAILABLE.to_string());
        }
        let _refresh = self.refresh_lock.lock().unwrap_or_else(|e| e.into_inner());
        if self.config_path().is_none() {
            return Ok(()); // 内置默认配置，无可刷新
        }
        let fresh = match self.config_store.load() {
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
        if self.dev_mode {
            return vec![DEV_UNAVAILABLE.to_string()];
        }
        let _refresh = self.refresh_lock.lock().unwrap_or_else(|e| e.into_inner());
        if self.config_path().is_none() {
            return Vec::new(); // 内置默认配置，无可刷新
        }
        let fresh = match self.config_store.load() {
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
            write_locked(&self.memory, Arc::new(crate::plugins::memory::NoopMemory));
            write_locked(&self.memory_sig, String::new());
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
                write_locked(&self.memory, engine);
                write_locked(&self.memory_sig, sig);
                let warn = d.warn_if.and_then(|f| f(fresh));
                self.mark_capability(
                    Capability::Memory,
                    if warn.is_some() {
                        Phase::Degraded
                    } else {
                        Phase::Active
                    },
                    warn,
                );
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
        write_locked(&self.llm, engine);
        write_locked(&self.llm_sig, sig);
        self.mark_capability(Capability::Llm, Phase::Active, None);
        tracing::info!("LLM 引擎已热切换：model={}（新会话生效）", fresh.llm.model);
        Ok(())
    }

    /// 把某能力的当前选中实现标记为指定阶段（状态由 `/api/plugins` 上报）。
    pub(super) fn mark_capability(&self, cap: Capability, phase: Phase, err: Option<String>) {
        self.host.mark_capability(cap, phase, err);
    }
}

/// 热切换决策（**纯函数**，可在无引擎 / 无 `sherpa` feature 的 `cargo test` 中直接断言）：
///
/// - `Ok(Some(sig))` → 需按新签名重建；
/// - `Ok(None)` → 签名未变，无需重建（会话开始调用是零开销的）；
/// - `Err(reason)` → 配置不通过校验或能力受限：**保留旧实例**，`reason` 由调用方写入插件状态。
///
/// 把决策从"重建"里抽出来，是为了让"改 model 生效 / 坏配置不替换"这条 P2 验收
/// 能在没有真实引擎（也就没有 sherpa feature）的单元测试里被证明。
pub(super) fn decide_refresh(
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
        return Err(format!(
            "{} 配置未通过校验（保持当前引擎）: {e:#}",
            d.display
        ));
    }
    if let Some(reason) = d.warn_if.and_then(|f| f(fresh)) {
        return Err(format!("{} 能力受限（保持当前引擎）: {reason}", d.display));
    }
    let sig = (d.signature)(fresh);
    let old = old_sig.read().unwrap_or_else(|e| e.into_inner()).clone();
    if sig == old {
        Ok(None)
    } else {
        Ok(Some(sig))
    }
}
