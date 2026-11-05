//! Implementation validation and construction entry points.

use anyhow::Result;
#[cfg(feature = "wasm-plugins")]
use std::sync::Arc;

use crate::config::Config;
use crate::plugins::llm::build_llm;
#[cfg(feature = "wasm-plugins")]
use crate::plugins::tts::build_tts_wasm;
use crate::plugins::tts::{build_kokoro, build_xfyun};

use super::super::Built;

/// 远端 ASR 描述符的构建（P4b）：晚绑全局宿主槽（boot 期宿主尚未创建，
/// `assemble` 创建后经 `wasm::set_host_global` 注入；recognize 调用期解析）。
/// 未编入 wasm-plugins feature → boot 显式失败（配置错误，文案给修复指引）。
pub(super) fn build_asr_wasm_d(_cfg: &Config) -> Result<Built> {
    #[cfg(feature = "wasm-plugins")]
    {
        return Ok(Built::Asr(Arc::new(
            crate::plugins::asr::wasm_engine::WasmAsrEngine::for_extension("asr.provider"),
        )));
    }
    #[cfg(not(feature = "wasm-plugins"))]
    {
        anyhow::bail!(
            "[asr].engine = \"wasm-remote\" 需要以 wasm-plugins feature 构建的服务端（当前二进制未编入）"
        )
    }
}

/// 远端引擎无本地可校验项（模型/凭据都不需要）；运行期缺宿主/插件由
/// `WasmAsrEngine::recognize` 显式报错（不静默回退本地）。
pub(super) fn validate_asr_wasm(_cfg: &Config) -> Result<()> {
    Ok(())
}

// ---- 构建入口 ----

pub(super) fn build_asr_d(cfg: &Config) -> Result<Built> {
    Ok(Built::Asr(crate::plugins::asr::build_asr(&cfg.asr)?))
}
/// Kokoro 的构建入口**只读 `[tts.kokoro]`**（P4 私有段）：新增供应商时各写各的，无需中心分发。
pub(super) fn build_tts_kokoro_d(cfg: &Config) -> Result<Built> {
    Ok(Built::Tts(build_kokoro(&cfg.tts.kokoro)?))
}
/// 讯飞在线合成（`[tts.xfyun]`）。
pub(super) fn build_tts_xfyun_d(cfg: &Config) -> Result<Built> {
    Ok(Built::Tts(build_xfyun(&cfg.tts.xfyun)?))
}
pub(super) fn build_tts_wasm_policy_d(cfg: &Config) -> Result<Built> {
    #[cfg(feature = "wasm-plugins")]
    {
        return Ok(Built::Tts(build_tts_wasm(&cfg.tts)?));
    }
    #[cfg(not(feature = "wasm-plugins"))]
    {
        let _ = cfg;
        anyhow::bail!(
            "[tts].engine = \"wasm-policy\" 需要以 wasm-plugins feature 构建的服务端（且仍需底层 sherpa/Kokoro native 引擎）"
        )
    }
}
pub(super) fn build_llm_d(cfg: &Config) -> Result<Built> {
    Ok(Built::Llm(build_llm(&cfg.llm)))
}
pub(super) fn build_llm_wasm_policy_d(cfg: &Config) -> Result<Built> {
    #[cfg(feature = "wasm-plugins")]
    {
        return Ok(Built::Llm(crate::plugins::llm::build_llm_wasm(&cfg.llm)));
    }
    #[cfg(not(feature = "wasm-plugins"))]
    {
        let _ = cfg;
        anyhow::bail!(
            "[llm].engine = \"wasm-policy\" 需要以 wasm-plugins feature 构建的服务端（且需 llm.provider 策略插件）"
        )
    }
}
pub(super) fn build_memory_d(cfg: &Config) -> Result<Built> {
    Ok(Built::Memory(crate::plugins::memory::build_memory(
        &cfg.memory,
        &cfg.llm,
    )?))
}
