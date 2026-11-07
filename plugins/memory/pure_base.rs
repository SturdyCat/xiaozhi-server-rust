//! pure-base 下的记忆兼容接口。
//!
//! 图记忆 / 纯存储 / RAG 后端与辅助 LLM 抽取是业务能力，不属于纯底座核心
//! ASR/VAD/TTS/LLM 数据面；pure-base 不编译任何后端实现与 SQLite schema。
//!
//! 单源共享（`#[path]` 引用，与原生实现物理同一文件）：
//! - [`provider`]：`MemoryProvider` trait + `NoopMemory`（接口一行不改的承诺）；
//! - [`types`]：召回/写入/抽取/维护的数据类型；
//! - [`config`]：`[memory]` 配置段（旧配置库无损解析，注册表描述符零改动）。
//!
//! 行为契约：`[memory].enabled = false`（默认）→ `NoopMemory`（零成本，与原生一致）；
//! 业务后端被选中时**明确报错**（可选能力降级为 Degraded，不伪造"记忆已启用"）——
//! 记忆业务只能由运行期 WASM 插件或 legacy 构建提供。

use std::sync::Arc;

#[path = "config.rs"]
pub mod config;
#[path = "provider.rs"]
mod provider;
#[path = "types.rs"]
pub mod types;

pub use config::{ExtractorConfig, MemoryBackend, MemoryConfig, RagConfig, RetentionConfig};
pub use provider::{BoxFut, MemoryProvider, NoopMemory};
pub use types::{
    CompletedTurn, ExtractOutcome, MaintainOpts, MaintainOutcome, MemoryStats, RecallResult,
};

/// 依据配置构造记忆引擎（pure-base 收口点，签名与原生一致）。
///
/// 关闭（`active() == false`）或显式 `engine = "none"` → [`NoopMemory`]；
/// graph / store / rag 属于 legacy-business 业务后端，选中即报错（不静默回退）。
pub fn build_memory(
    cfg: &MemoryConfig,
    _llm: &crate::config::LlmConfig,
) -> anyhow::Result<Arc<dyn MemoryProvider>> {
    if !cfg.active() {
        return Ok(Arc::new(NoopMemory));
    }
    let engine = cfg.engine_id();
    anyhow::bail!(
        "[memory].engine = \"{engine}\" 的原生后端属于 legacy-business，pure-base 未编译：\
         请改用 engine = \"none\"（或关闭 [memory].enabled），\
         或把记忆业务交由已装载的 WASM 插件提供。"
    );
}
