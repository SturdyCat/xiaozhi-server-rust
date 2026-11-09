//! WASM 插件平台：契约 `xiaozhi:plugin@0.2.0`（唯一源 `server/wit/plugin.wit`）。
//! 当前维护说明见本级 [`AGENTS.md`](<AGENTS.md>)；`docs/designs/wasm-plugin-platform.md`
//! 是 v0.1 / 方案 v4 的历史长文，不作现行事实。
//!
//! ## 边界
//!
//! 宿主构建期不含任何业务，只提供通用能力：四个 Host trait（core/net/timer/system）、
//! 事件总线与服务注册表、宿主代监听的 socket、定时器、文件与配置沙箱、mDNS、artifact
//! 与生命周期。扩展点、topic、协议语义全在插件侧，宿主按字符串通用路由。
//!
//! ## 分层（文件即职责）
//!
//! - [`artifacts`] / [`sandbox`] / [`resolver`] / [`kv`]：产物钉版、沙箱预算、排他解析、隔离存储；
//! - [`contract`]（含 `contract/validate.rs`）：bindgen 绑定、DTO 与装载期强校验；
//! - [`host_api`]：`HostContext` 聚合根 + `SystemFacts` 抽象，平台不反向依赖会话层；
//! - [`instance`]：四个 Host trait 实现 + 类型化 `handle_event` / `handle_service`；
//! - [`registry`] / [`lifecycle`]：`WasmHost` 门面、每插件 Runner/入站队列/泵任务、三态生命周期；
//! - [`event_bus`] / [`services`] / [`net`] / [`timers`] / [`fs`] / [`mdns`] / [`config_store`]：
//!   订阅路由、服务调用、socket 监听、定时器、文件沙箱、广播与配置命名空间；
//! - [`effects`]：声明式效果 + 权限门（log/kv-put/kv-delete/http-request）。
//!
//! 阶段二收口：遗留策略桥 `bridge*` 与事件瀑布 `waterfall` 已删除——各能力/设备
//! 域的 prepare 派发归位到各自模块（asr/tts/vad 引擎适配器、weather、
//! workflow/session/hooks、device/{tuya,echomuse}/policy），宿主 crate 不再持有
//! 任何业务扩展点名。
//!
//! 静态注册表（`plugins/registry`）回答「内置能力现状」，本模块回答「动态插件供给」，
//! 两者在 `/api/plugins` 合并为同一张清单；静态表与 native 业务 feature 的剥离属阶段二。

#[cfg(feature = "wasm-plugins")]
pub mod artifacts;
#[cfg(feature = "wasm-plugins")]
pub mod config_store;
#[cfg(feature = "wasm-plugins")]
pub mod contract;
#[cfg(feature = "wasm-plugins")]
pub mod effects;
#[cfg(feature = "wasm-plugins")]
pub mod event_bus;
#[cfg(feature = "wasm-plugins")]
pub mod fs;
#[cfg(feature = "wasm-plugins")]
pub mod host_api;
#[cfg(feature = "wasm-plugins")]
pub mod instance;
#[cfg(feature = "wasm-plugins")]
pub mod kv;
#[cfg(feature = "wasm-plugins")]
pub mod lifecycle;
#[cfg(feature = "wasm-plugins")]
pub mod mdns;
#[cfg(feature = "wasm-plugins")]
pub mod net;
#[cfg(feature = "wasm-plugins")]
pub mod registry;
#[cfg(feature = "wasm-plugins")]
pub mod resolver;
#[cfg(feature = "wasm-plugins")]
pub mod sandbox;
#[cfg(feature = "wasm-plugins")]
pub mod services;
#[cfg(feature = "wasm-plugins")]
pub mod timers;

#[cfg(test)]
#[cfg(feature = "wasm-plugins")]
pub(crate) mod tests;

pub(crate) use contract::version_at_least;
pub use effects::{Effect, PermissionError, Permissions};
pub use lifecycle::{
    ActivateError, DeactivateError, DependencyGap, Lifecycle, RemoveError, StageError, StageOutcome,
};
pub use registry::{Phase, WasmHost};
pub use resolver::{Resolution, ResolutionTable};
pub use sandbox::{SandboxLimits, TrapKind};

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

/// 本宿主实现的契约次版本（`0.2.0` 的次段）：**只增不改**，插件声明次版本高于宿主即拒装。
pub const CONTRACT_MINOR_VERSION: u32 = 2;

/// 契约版本字符串（日志 / `/api/plugins/contract` 投影用）。
pub fn contract_version() -> String {
    format!("{CONTRACT_MAJOR_VERSION}.{CONTRACT_MINOR_VERSION}.0")
}
