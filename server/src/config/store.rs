//! 配置存储后端抽象：`ConfigStore` trait + 两个实现（文件 / 内置默认）。
//!
//! ## 为什么要有这一层（P0，见 `server/src/config/store/AGENTS.md`）
//!
//! 改动前配置的读写散落在 20+ 处：`Config::load(路径)` 直读盘、`persist_config(路径, cfg)`
//! 直写盘、`engines.config_path` 派生 overlay / presets / hot-audio 落点。要把存储换成 SQLite，
//! 得先把「读、写、落点」三件事收进一个 trait——之后新增 `SqliteConfigStore` 是**加一个实现**，
//! 而不是改 20 个调用点。
//!
//! ## 本阶段的纪律：只做**行为不变**的搬移
//!
//! - [`FileConfigStore::persist`] 就是原 `app::ws::config::persist_config`（两级写入策略：
//!   临时文件 rename 原子写 → 单文件 bind mount 的 EBUSY 原地覆写回退），**错误文案一字不改**；
//! - [`DefaultConfigStore`]（内置 `Config::default()` 启动）`location() == None`，各调用点
//!   「无外部来源」的分支与改动前 `config_path: None` **完全相同**——包括日志差异
//!   （`refresh_from_disk` 静默返回、`current_config` 静默回退、`bench_api`/`voices` 静默回退）。
//!
//! ## 与 memory 共库的约束（P1 才会用到，此处先记录，避免后来者踩坑）
//!
//! 配置表迁移**必须独占** `_cfg_migrations`，**绝不**写进 memory 的 `_migrations`
//! （后果矩阵见设计文档 §3.1：会把两个子系统的编号语义搅在一起，导致静默漏迁移）。
//! 另外配置 store 必须**自持连接**：`[memory].enabled` 默认 false 时记忆库根本不会被打开。

use anyhow::{Context, Result};

use super::Config;

/// 配置存储后端。
///
/// 实现必须 `Send + Sync`：`Arc<dyn ConfigStore>` 存于 [`crate::engine::Engines`]，
/// 被 axum handler（多线程 runtime）共享。
pub trait ConfigStore: Send + Sync {
    /// 持久化落点（今天的 TOML 路径；将来 SQLite 后端的 DB 路径）。
    ///
    /// `None` = 无外部配置来源（内置默认启动）——各调用点用它区分
    /// 「可持久化 / 不可持久化」，**不要**用 `load()` 是否报错来判定（两者的日志语义不同）。
    fn location(&self) -> Option<&str>;

    /// 读取当前生效配置。文件后端 = 实时读盘（解析失败即 `Err`，由调用方决定回退）。
    fn load(&self) -> Result<Config>;

    /// 写回配置（文件后端：`normalize()` → 原子写 → 回退原地覆写）。
    fn persist(&self, cfg: &Config) -> Result<()>;
}

/// 文件后端（TOML）：改动前的全部行为。
pub struct FileConfigStore {
    path: String,
}

impl FileConfigStore {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

impl ConfigStore for FileConfigStore {
    fn location(&self) -> Option<&str> {
        Some(&self.path)
    }

    fn load(&self) -> Result<Config> {
        Config::load(&self.path)
    }

    fn persist(&self, cfg: &Config) -> Result<()> {
        persist_toml(&self.path, cfg)
    }
}

/// 无外部来源：内置 `Config::default()` 启动（`XIAOZHI_CONFIG` / `--config` 均未提供或读取失败）。
///
/// `load()` / `persist()` 都返回中文错误（含「怎么修」），但**正常路径不应触达**：
/// 调用点一律先用 [`ConfigStore::location`] 判空。
pub struct DefaultConfigStore;

impl ConfigStore for DefaultConfigStore {
    fn location(&self) -> Option<&str> {
        None
    }

    fn load(&self) -> Result<Config> {
        anyhow::bail!(
            "server 以内置默认配置启动，没有外部配置来源可读（用 XIAOZHI_CONFIG 或 --config 指定配置文件后重启）"
        )
    }

    fn persist(&self, _cfg: &Config) -> Result<()> {
        anyhow::bail!(
            "server 以内置默认配置启动，未指定配置文件，无法持久化。请用 --config 指定 config.toml 后重启。"
        )
    }
}

/// TOML 落盘（**原 `app::ws::config::persist_config` 原样搬移**，P4 起先 `normalize()` 再序列化：
/// 手搓/未规范化的内存配置若直接落盘，会把 `[tts].engine` 写成空值、旧位置字段写回旧结构
/// ——那正是我们要消除的「保存即漂移」）。
///
/// 写入策略两级：
/// 1. **原子写**（首选）：写同目录临时文件 → `rename` 覆盖，避免写一半被杀导致配置损坏；
/// 2. **原地写回退**：⚠️ compose 把 `config.toml` 以**单文件 bind mount** 挂进容器时，
///    `rename` 跨挂载点必然失败（`EBUSY`，os error 16，实测）——此时回退直接覆写原文件
///    （非原子，但挂载单文件的场景下这是唯一可行路径），并清理残留 tmp。
pub(crate) fn persist_toml(path: &str, cfg: &Config) -> Result<()> {
    let mut cfg = cfg.clone();
    cfg.normalize();
    let toml_str = toml::to_string_pretty(&cfg).context("序列化配置失败")?;
    let tmp = format!("{}.{}.tmp", path, uuid::Uuid::new_v4());
    let atomic = std::fs::write(&tmp, &toml_str).and_then(|()| std::fs::rename(&tmp, path));
    if atomic.is_ok() {
        return Ok(());
    }
    let atomic_err = atomic.unwrap_err();
    let fallback = std::fs::write(path, &toml_str);
    let _ = std::fs::remove_file(&tmp); // 清理 rename 失败残留的临时文件
    if let Err(e) = fallback {
        let hint = if e.kind() == std::io::ErrorKind::PermissionDenied {
            "（配置文件或所在目录为只读/属主不符。请检查宿主挂载是否误加 :ro，或修正属主与权限后重启 server。）"
        } else {
            ""
        };
        anyhow::bail!("写入配置失败: {e}{hint}（原子写失败原因: {atomic_err}）");
    }
    tracing::warn!(
        "配置已原地写入 {path}（挂载为单文件 bind mount，rename 原子替换不可用: {atomic_err}）"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
