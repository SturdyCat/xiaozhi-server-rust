//! WASM 插件平台（方案 v4 / `docs/designs/wasm-plugin-platform.md`，P0）。
//!
//! ## 分层（文件即职责）
//!
//! - [`artifacts`]：产物落盘（上传即启用：save/remove/scan，重启重装）；
//! - [`contract`]：WIT 契约（`server/wit/plugin.wit`）的宿主绑定 + 投影 DTO；
//! - [`sandbox`]：沙箱三重上限（fuel / epoch / 内存）与陷阱分类；
//! - [`instance`]：单个已装载插件（Store + bindgen 实例 + 带预算的调用包装）；
//! - [`effects`]：效果模型 + 权限门 + 执行器（log / kv-put / http-get 两段式回灌；
//!   其余变体显式 Unsupported，随宿主服务落地扩员）；
//! - [`kv`]：插件隔离命名空间的键值存储（P0 内存实现，后续换 SQLite）；
//! - [`resolver`]：扩展点解析表（排他扩展点 → 唯一实现，用户插件失败回退官方默认）;
//! - [`waterfall`]：有序事件瀑布（`next()` 透传纪律 + 强度分级），内核通用件，P2 移交会话层；
//! - [`registry`]：`WasmHost`——装载/激活/phase/快照/事件分发的门面。
//!
//! ## 刻意不做（P0 边界）
//!
//! - 不接管静态注册表（`plugins/registry`）的装配：两套体系并存，P4+ 才逐域绞杀迁移；
//! - 无并发实例池：v0 单实例串行调用（Mutex），池化在 P1 按性能预算引入；
//! - 效果执行器只实现 log/kv-put；http-out/speak 等宿主服务按阶段扩。
//!
//! ## 与静态注册表的关系
//!
//! 静态注册表回答「内置能力现在什么状态」；本模块回答「动态插件提供什么」。
//! 两者在 P2 的 `/api/plugins` 投影里合并为同一张清单。

#[cfg(feature = "wasm-plugins")]
pub mod artifacts;
#[cfg(feature = "wasm-plugins")]
pub mod bridge;
#[cfg(feature = "wasm-plugins")]
pub mod contract;
#[cfg(feature = "wasm-plugins")]
pub mod effects;
#[cfg(feature = "wasm-plugins")]
pub mod instance;
#[cfg(feature = "wasm-plugins")]
pub mod kv;
#[cfg(feature = "wasm-plugins")]
pub mod lifecycle;
#[cfg(feature = "wasm-plugins")]
pub mod registry;
#[cfg(feature = "wasm-plugins")]
pub mod resolver;
#[cfg(feature = "wasm-plugins")]
pub mod sandbox;
#[cfg(feature = "wasm-plugins")]
pub mod waterfall;

#[cfg(test)]
#[cfg(feature = "wasm-plugins")]
pub(crate) mod tests;

pub use effects::{Effect, LanSocketCommand, PermissionError, Permissions};
pub use lifecycle::{
    ActivateError, DeactivateError, DependencyGap, Lifecycle, RemoveError, StageError,
    StageOutcome,
};
pub use registry::{Phase, WasmHost};
pub use resolver::{Resolution, ResolutionTable};
pub(crate) use contract::version_at_least;
pub use sandbox::{SandboxLimits, TrapKind};
pub use waterfall::{Strength, Waterfall};

// ============================================================
// 进程级宿主槽（晚绑）：boot 期构建的引擎（如 WasmAsrEngine）在宿主创建后
// 才能拿到实例——镜像 weather/geo 的进程级全局惯例（OnceLock 一次性注入）。
// ============================================================

static HOST_GLOBAL: std::sync::OnceLock<std::sync::Arc<WasmHost>> = std::sync::OnceLock::new();

/// 注入进程级宿主（`Engines::assemble` 创建宿主后调用一次；重复注入静默忽略）。
pub fn set_host_global(host: std::sync::Arc<WasmHost>) {
    let _ = HOST_GLOBAL.set(host);
}

/// 取进程级宿主（未创建 = None；调用方显式报错，不静默降级）。
pub fn host_global() -> Option<std::sync::Arc<WasmHost>> {
    HOST_GLOBAL.get().cloned()
}

/// 本宿主实现的契约主版本（与 `server/wit/plugin.wit` 的包版本主段一致）。
/// manifest.api-version 主段不等于它 → 拒绝激活（见 [`registry::WasmHost::load`]）。
pub const CONTRACT_MAJOR_VERSION: u32 = 0;
