//! 配置存储后端抽象：`ConfigStore` trait + 三个实现（SQLite / 文件 / 内置默认）。
//!
//! ## 现状（已落地，2026-10-14）
//!
//! - [`sqlite::SqliteConfigStore`]：**默认真源**。整份 Config 作为 JSON blob 存于 `cfg_documents`，
//!   设备 overlay 进 `cfg_device_overlays`、预设进 `cfg_presets`；并提供只读 SQL 查询与
//!   `VACUUM INTO` 备份。连接自持（[`plugins::memory::store`] 同款：单连接 + `Mutex` + WAL +
//!   `busy_timeout`），与 memory 可共用同一 db 文件（`_cfg_migrations` 与 memory 的 `_migrations`
//!   命名空间**严格隔离**）。
//! - [`FileConfigStore`]：降级路径（SQLite 打开失败时使用），overlay/预设落文件，行为与历史一致。
//! - [`DefaultConfigStore`]：内置 `Config::default()` 启动，`location() == None` → `PUT /api/config` 400。
//!
//! 装配见 `main.rs::load_config_inner`：解析 DB 路径（`XIAOZHI_CONFIG_DB` → `--config-db` →
//! `[memory].db_path` → `/data/xiaozhi.db`）→ 打开 SQLite → 空库且有旧 TOML 则**兼容导入**
//! （`source=bootstrap`，DB 优先）→ 失败降级文件/默认。
//!
//! ## 为什么要有这一层（P0，设计论证见 `server/src/config/store/AGENTS.md`）
//!
//! 改动前配置的读写散落在 20+ 处：`Config::load(路径)` 直读盘、`persist_config(路径, cfg)`
//! 直写盘、`engines.config_path` 派生 overlay / presets / hot-audio 落点。把存储换成 SQLite，
//! 靠的是把「读、写、落点」三件事收进一个 trait——新增 `SqliteConfigStore` 是**加一个实现**，
//! 而不是改 20 个调用点。
//!
//! ## 与 memory 共库的约束（最高优先级）
//!
//! 配置表迁移**必须独占** `_cfg_migrations`，**绝不**写进 memory 的 `_migrations`
//! （后果矩阵见设计文档 §3.1：会把两个子系统的编号语义搅在一起，导致静默漏迁移）。
//! 配置 store **自持连接**：`[memory].enabled` 默认 false 时记忆库根本不会被打开，
//! 但配置库仍可读写（配置是必需能力）。

use anyhow::{Context, Result};
use serde_json::Value;

use super::Config;

/// 该存储后端不支持某操作的统一错误（默认实现用；SQLite / 文件后端各自覆盖）。
fn unsupported(op: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{op} 仅 SQLite 配置后端支持；当前后端不支持该操作（请用 SQLite 配置后端，或由管理页配置接口操作）。"
    )
}

/// SQLite 配置后端（主配置 + overlay + 预设进表，并提供只读 SQL 查询与备份）。
pub mod sqlite;

/// 配置存储后端。
///
/// 实现必须 `Send + Sync`：`Arc<dyn ConfigStore>` 存于 [`crate::engine::Engines`]，
/// 被 axum handler（多线程 runtime）共享。
///
/// **落点抽象的唯一入口**：主配置（`load`/`persist`）、每设备 overlay、配置预设、
/// 只读 SQL 查询、备份、首启引导导入——全部经本 trait。不要绕过它直接 `Config::load(路径)`
/// 或 `std::fs` 操作，否则换成 SQLite 后端时这些路径会被绕过。
pub trait ConfigStore: Send + Sync {
    /// 持久化落点（文件后端 = TOML 路径；SQLite 后端 = DB 文件路径）。
    ///
    /// `None` = 无外部配置来源（内置默认启动）——各调用点用它区分
    /// 「可持久化 / 不可持久化」，**不要**用 `load()` 是否报错来判定（两者的日志语义不同）。
    fn location(&self) -> Option<&str>;

    /// 读取当前生效配置。文件后端 = 实时读盘（解析失败即 `Err`，由调用方决定回退）；
    /// SQLite 后端 = 读 `cfg_documents`（空库返回内置默认，便于首次保存即落库）。
    fn load(&self) -> Result<Config>;

    /// 写回配置（文件后端：`normalize()` → 原子写 → 回退原地覆写；SQLite 后端：单事务写 `cfg_documents`）。
    fn persist(&self, cfg: &Config) -> Result<()>;

    /// 后端类型标识：`file` / `sqlite` / `default`——供 `/api/config/meta` 与启动日志区分。
    fn backend(&self) -> &'static str {
        "default"
    }

    // ---- 每设备 overlay（白名单字段增量）----
    /// 读设备 overlay；无 → `Ok(None)`。
    fn load_overlay(&self, _device_id: &str) -> Result<Option<Value>> {
        Err(unsupported("设备 overlay"))
    }
    /// 写/替换设备 overlay（调用方**已**校验白名单）。
    fn save_overlay(&self, _device_id: &str, _patch: &Value) -> Result<()> {
        Err(unsupported("设备 overlay"))
    }
    /// 删除设备 overlay。
    fn delete_overlay(&self, _device_id: &str) -> Result<()> {
        Err(unsupported("设备 overlay"))
    }

    // ---- 配置预设（整份 Config 快照）----
    /// 列出全部预设名（排序）。
    fn list_presets(&self) -> Result<Vec<String>> {
        Err(unsupported("配置预设"))
    }
    /// 保存预设（整份 Config 快照）。
    fn save_preset(&self, _name: &str, _doc: &Config) -> Result<()> {
        Err(unsupported("配置预设"))
    }
    /// 读取预设为 `Config`。
    fn load_preset(&self, _name: &str) -> Result<Option<Config>> {
        Err(unsupported("配置预设"))
    }
    /// 删除预设。
    fn delete_preset(&self, _name: &str) -> Result<()> {
        Err(unsupported("配置预设"))
    }

    // ---- SQLite 专属（文件 / 默认后端返回不支持）----
    /// 只读 SQL 查询（返回行数组，每行是列名 → 值的 map）。详见 `sqlite.rs` 的只读护栏。
    fn query(&self, _sql: &str) -> Result<Vec<serde_json::Map<String, Value>>> {
        Err(unsupported("SQL 查询"))
    }
    /// 备份数据库（`VACUUM INTO`），仅 SQLite。
    fn backup(&self, _dest: &str) -> Result<()> {
        Err(unsupported("数据库备份"))
    }
    /// 首启引导导入：DB 为空且有旧 TOML 时把整份 Config 写入（`source=bootstrap`）。
    /// 文件后端无需导入（数据已在文件里），默认空操作成功。
    fn bootstrap_import(&self, _cfg: &Config) -> Result<()> {
        Ok(())
    }
    /// DB 是否为空（无任何 `cfg_documents` 行）。非 DB 后端返回 `false`（视为"已有数据"以跳过导入）。
    fn is_empty(&self) -> bool {
        false
    }
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

    fn backend(&self) -> &'static str {
        "file"
    }

    // 文件后端：overlay / 预设落盘到 config 同目录（与 SQLite 后端语义一致，保持兼容）。

    fn load_overlay(&self, device_id: &str) -> Result<Option<Value>> {
        let Some(path) = crate::config::overlay::device_overlay_path(&self.path, device_id) else {
            anyhow::bail!("设备 ID 非法（仅允许字母数字与 - _ : 字符）。");
        };
        Ok(crate::config::overlay::read_device_overlay_value(&path))
    }

    fn save_overlay(&self, device_id: &str, patch: &Value) -> Result<()> {
        let path = crate::config::overlay::device_overlay_path(&self.path, device_id)
            .ok_or_else(|| anyhow::anyhow!("设备 ID 非法（仅允许字母数字与 - _ : 字符）。"))?;
        let toml_str = crate::config::overlay::json_to_toml(patch)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建 overlay 目录 {} 失败", parent.display()))?;
        }
        std::fs::write(&path, &toml_str)
            .with_context(|| format!("写入设备 overlay {} 失败", path.display()))
    }

    fn delete_overlay(&self, device_id: &str) -> Result<()> {
        let Some(path) = crate::config::overlay::device_overlay_path(&self.path, device_id) else {
            anyhow::bail!("设备 ID 非法（仅允许字母数字与 - _ : 字符）。");
        };
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("删除设备 overlay {} 失败", path.display())),
        }
    }

    fn list_presets(&self) -> Result<Vec<String>> {
        let Some(dir) = crate::config::overlay::presets_dir(&self.path) else {
            anyhow::bail!("无法确定预设目录。");
        };
        let mut names = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) == Some("toml") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        names.push(stem.to_string());
                    }
                }
            }
        }
        names.sort();
        Ok(names)
    }

    fn save_preset(&self, name: &str, doc: &Config) -> Result<()> {
        let preset_file = crate::config::overlay::preset_path(&self.path, name)
            .ok_or_else(|| anyhow::anyhow!("预设名非法（仅允许字母数字与 - _ . 字符）。"))?;
        let toml_str = toml::to_string_pretty(doc).context("序列化预设失败")?;
        if let Some(parent) = preset_file.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建预设目录 {} 失败", parent.display()))?;
        }
        std::fs::write(&preset_file, &toml_str)
            .with_context(|| format!("保存预设 {} 失败", preset_file.display()))
    }

    fn load_preset(&self, name: &str) -> Result<Option<Config>> {
        let preset_file = crate::config::overlay::preset_path(&self.path, name)
            .ok_or_else(|| anyhow::anyhow!("预设名非法（仅允许字母数字与 - _ . 字符）。"))?;
        if !preset_file.exists() {
            return Ok(None);
        }
        Ok(Some(Config::load(preset_file.to_str().unwrap_or(""))?))
    }

    fn delete_preset(&self, name: &str) -> Result<()> {
        let preset_file = crate::config::overlay::preset_path(&self.path, name)
            .ok_or_else(|| anyhow::anyhow!("预设名非法（仅允许字母数字与 - _ . 字符）。"))?;
        match std::fs::remove_file(&preset_file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("删除预设 {} 失败", preset_file.display())),
        }
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
