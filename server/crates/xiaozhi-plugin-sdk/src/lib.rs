//! 小智 WASM 插件 guest SDK（契约唯一源的 guest 侧入口）。
//!
//! ## 职责（刻意收窄）
//!
//! - **契约绑定共享**：[`wit_bindgen::generate!`] 以 `server/wit/plugin.wit` 为唯一源在本 crate
//!   生成绑定；guest 不再各自指向 WIT 文件——引用本 crate 即与宿主同源（依赖宪法 R2）。
//! - **运行期分配纪律**（[`fallible`]）：处理事件/动作载荷等**运行期数据**时用可失败分配，
//!   分配失败 → 显式 `Err` 返回宿主（业务性失败，不降级）；panic/abort 才是陷阱（Degraded）。
//! - **静态注册构建器**（[`builder`]）：manifest/registration 的类型化构建，guest 侧先做与
//!   宿主 `contract::validate_*` 同源的形状校验，错误在构建期暴露而非装载期被拒。
//!
//! ## 分配纪律的分界（重要）
//!
//! | 数据 | 归属 | 失败语义 |
//! |---|---|---|
//! | 静态注册包（manifest/菜单/字段表） | [`builder`]，expect 语义 | 激活失败 → `Failed`（编程错误，静态可测） |
//! | 事件/动作载荷与派生数据 | [`fallible`]，显式 `Err` | 业务性失败，不降级、不卸载 |
//!
//! ## guest 骨架
//!
//! ```ignore
//! use xiaozhi_plugin_sdk::exports::lifecycle::Guest;
//! use xiaozhi_plugin_sdk::xiaozhi::plugin::types;
//!
//! struct Component;
//!
//! impl Guest for Component { /* manifest/register/handle-event/... */ }
//!
//! xiaozhi_plugin_sdk::export!(Component);
//! ```
//!
//! 宿主侧绑定在 `server/plugins/wasm/contract.rs`（同一份 WIT）；改契约必须重建
//! 本 crate 与全部 guest 夹具（`tools/wasm-fixtures/build.sh`）。

wit_bindgen::generate!({
    path: "../../wit",
    world: "plugin",
    // guest 跨 crate 使用 `xiaozhi_plugin_sdk::export!(Component)` 的两个前提
    // （wit-bindgen 原生选项，勿手工 pub use——会重名冲突）：
    // - pub_export_macro：export! 宏 pub 化（否则 crate 私有）；
    // - default_bindings_module：宏展开体内的路径默认 `self`（= 调用方 crate 根，
    //   guest 那没有绑定模块）——必须改指 SDK，让展开体解析到这里的生成模块。
    pub_export_macro: true,
    default_bindings_module: "xiaozhi_plugin_sdk",
});

pub mod builder;
pub mod error;
pub mod fallible;

pub use error::{Error, Result};

/// 契约版本（与 `server/wit/plugin.wit` 包版本一致；宿主校验主版本）。
pub const API_VERSION: &str = "0.1";
