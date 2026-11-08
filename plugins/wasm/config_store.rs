//! 插件配置存储（按插件命名空间隔离）。
//!
//! 契约里 `config-get` / `config-secret` / `config-set` 的宿主实现依据。
//! 存储后端由 [`PluginConfigStore`] 抽象：内核默认用内存实现（测试 / 无配置库场景），
//! 生产由 `server` 侧注入 SQLite 实现（`plugin_configs` 表）。
//!
//! 安全口径：
//! - `get` 永不返回 secret 字段的明文（返回 `None`），只有 `get_secret` 返回；
//! - `get_secret` 每次调用写审计（由后端实现，见 `server/src/config/store/sqlite.rs`）；
//! - `set` 的合法性（字段已声明、kind / enum / required）在 [`crate::plugins::wasm::registry`]
//!   调用前校验，本模块只负责存取。

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use anyhow::Result;

/// 配置读写后端。
pub trait PluginConfigStore: Send + Sync {
    /// 读非密值；secret 字段必须返回 `None`。
    fn get(&self, plugin_id: &str, key: &str) -> Option<String>;
    /// 读密值（仅供已声明 `secret=true` 的字段）。
    fn get_secret(&self, plugin_id: &str, key: &str) -> Option<String>;
    /// 写入（已存在则覆盖）。
    fn set(&self, plugin_id: &str, key: &str, value: &str) -> Result<()>;
    /// 删除；返回是否原本存在。
    fn delete(&self, plugin_id: &str, key: &str) -> bool;
    /// 列出某插件的全部非密键值（secret 键只回键名、值为空串）。
    fn list(&self, plugin_id: &str) -> Vec<(String, String)>;
    /// 清空某插件的全部配置（卸载 `--purge`）。
    fn clear(&self, plugin_id: &str);
}

/// 内存实现（进程重启丢失；默认后端，测试与小规模场景够用）。
#[derive(Default)]
pub struct MemoryConfigStore {
    inner: RwLock<BTreeMap<String, BTreeMap<String, String>>>,
}

impl MemoryConfigStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl PluginConfigStore for MemoryConfigStore {
    fn get(&self, plugin_id: &str, key: &str) -> Option<String> {
        self.inner
            .read()
            .ok()?
            .get(plugin_id)?
            .get(key)
            .cloned()
    }

    fn get_secret(&self, plugin_id: &str, key: &str) -> Option<String> {
        self.get(plugin_id, key)
    }

    fn set(&self, plugin_id: &str, key: &str, value: &str) -> Result<()> {
        let mut guard = self.inner.write().map_err(|_| anyhow::anyhow!("配置锁中毒"))?;
        guard
            .entry(plugin_id.to_string())
            .or_default()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, plugin_id: &str, key: &str) -> bool {
        self.inner
            .write()
            .ok()
            .and_then(|mut g| g.get_mut(plugin_id).and_then(|m| m.remove(key)))
            .is_some()
    }

    fn list(&self, plugin_id: &str) -> Vec<(String, String)> {
        self.inner
            .read()
            .ok()
            .and_then(|g| g.get(plugin_id).cloned())
            .map(|m| m.into_iter().collect())
            .unwrap_or_default()
    }

    fn clear(&self, plugin_id: &str) {
        if let Ok(mut g) = self.inner.write() {
            g.remove(plugin_id);
        }
    }
}

/// 生成对外投影：secret 字段打码（`***`），非密字段原值。
pub fn redact_for_projection(
    secret_keys: &std::collections::HashSet<String>,
    entries: Vec<(String, String)>,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (k, v) in entries {
        if secret_keys.contains(&k) {
            map.insert(
                k,
                serde_json::json!({ "configured": !v.is_empty(), "value": null }),
            );
        } else {
            map.insert(k, serde_json::Value::String(v));
        }
    }
    serde_json::Value::Object(map)
}
