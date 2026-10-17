//! 配置存储后端抽象：`ConfigStore` trait + SQLite 唯一实现。
//!
//! ## 现状（2026-10：外置文件配置逻辑已移除）
//!
//! - [`sqlite::SqliteConfigStore`]：**唯一真源**。整份 Config 作为 JSON blob 存于 `cfg_documents`，
//!   设备 overlay 进 `cfg_device_overlays`、预设进 `cfg_presets`；并提供只读 SQL 查询与
//!   `VACUUM INTO` 备份。连接自持（[`plugins::memory::store`] 同款：单连接 + `Mutex` + WAL +
//!   `busy_timeout`），与 memory 共用同一 db 文件（`data.db`；`_cfg_migrations` 与 memory 的
//!   `_migrations` 命名空间**严格隔离**）。
//! - 旧 `FileConfigStore`（TOML 降级）与 `DefaultConfigStore`（内置默认、不可持久化）
//!   已删除：`data.db` 同时承载配置与 memory，SQLite 打不开则进程启动即失败（见
//!   `main.rs::load_config_inner`），不再有文件/默认降级路径。
//!
//! ## 为什么要有这一层（P0，设计论证见 `server/src/config/store/AGENTS.md`）
//!
//! 改动前配置的读写散落在 20+ 处：`Config::load(路径)` 直读盘、`persist_config(路径, cfg)`
//! 直写盘、`engines.config_path` 派生 overlay / presets / hot-audio 落点。把存储换成 SQLite，
//! 靠的是把「读、写、落点」三件事收进一个 trait。
//!
//! ## 与 memory 共库的约束（最高优先级）
//!
//! 配置表迁移**必须独占** `_cfg_migrations`，**绝不**写进 memory 的 `_migrations`
//! （后果矩阵见设计文档 §3.1：会把两个子系统的编号语义搅在一起，导致静默漏迁移）。
//! 配置 store **自持连接**：`[memory].enabled` 默认 false 时记忆库根本不会被打开，
//! 但配置库仍可读写（配置是必需能力）。

use anyhow::Result;
use serde_json::Value;

use super::Config;

/// SQLite 配置后端（主配置 + overlay + 预设进表，并提供只读 SQL 查询与备份）。
pub mod sqlite;

/// 配置存储后端。
///
/// 实现必须 `Send + Sync`：`Arc<dyn ConfigStore>` 存于 [`crate::engine::Engines`]，
/// 被 axum handler（多线程 runtime）共享。
///
/// **落点抽象的唯一入口**：主配置（`load`/`persist`）、每设备 overlay、配置预设、
/// 只读 SQL 查询、备份——全部经本 trait。不要绕过它直接读盘或 `std::fs` 操作，
/// 否则这些路径会绕过 SQLite 真源。
pub trait ConfigStore: Send + Sync {
    /// 持久化落点（SQLite 后端 = DB 文件路径）。
    ///
    /// `None` = 无持久化落点——各调用点用它区分「可持久化 / 不可持久化」，**不要**
    /// 用 `load()` 是否报错来判定（两者的日志语义不同）。
    fn location(&self) -> Option<&str>;

    /// 读取当前生效配置（读 `cfg_documents`，空库返回内置默认，便于首次保存即落库）。
    fn load(&self) -> Result<Config>;

    /// 写回配置（单事务写 `cfg_documents`）。
    fn persist(&self, cfg: &Config) -> Result<()>;

    /// 后端类型标识：`sqlite`——供 `/api/config/meta` 与启动日志区分。
    fn backend(&self) -> &'static str;

    // ---- 每设备 overlay（白名单字段增量）----
    /// 读设备 overlay；无 → `Ok(None)`。
    fn load_overlay(&self, device_id: &str) -> Result<Option<Value>>;
    /// 写/替换设备 overlay（调用方**已**校验白名单）。
    fn save_overlay(&self, device_id: &str, patch: &Value) -> Result<()>;
    /// 删除设备 overlay。
    fn delete_overlay(&self, device_id: &str) -> Result<()>;

    // ---- 配置预设（整份 Config 快照）----
    /// 列出全部预设名（排序）。
    fn list_presets(&self) -> Result<Vec<String>>;
    /// 保存预设（整份 Config 快照）。
    fn save_preset(&self, name: &str, doc: &Config) -> Result<()>;
    /// 读取预设为 `Config`。
    fn load_preset(&self, name: &str) -> Result<Option<Config>>;
    /// 删除预设。
    fn delete_preset(&self, name: &str) -> Result<()>;

    // ---- SQLite 专属 ----
    /// 只读 SQL 查询（返回行数组，每行是列名 → 值的 map）。详见 `sqlite.rs` 的只读护栏。
    fn query(&self, sql: &str) -> Result<Vec<serde_json::Map<String, Value>>>;
    /// 备份数据库（`VACUUM INTO`）。
    fn backup(&self, dest: &str) -> Result<()>;
    /// DB 是否为空（无任何 `cfg_documents` 行）——首次安装自举据此写入内置默认配置。
    fn is_empty(&self) -> bool;
}
