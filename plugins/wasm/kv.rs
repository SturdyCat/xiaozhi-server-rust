//! 插件隔离命名空间的键值存储（P0 内存实现）。
//!
//! 语义（方案 v4 §状态纪律）：插件 kv 是**派生缓存**，不是事实源——
//! 会话事件流才是事实源；kv 可随时清空重建。后续阶段换 SQLite 落盘，
//! 接口不变（put/get/delete/命名空间强隔离 + 容量上限）。

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{bail, Result};

/// 容量上限（P0）：防插件无限膨胀宿主内存。
const MAX_KEY_LEN: usize = 256;
const MAX_VALUE_LEN: usize = 64 * 1024;
const MAX_ENTRIES_PER_PLUGIN: usize = 1024;

#[derive(Default)]
struct Inner {
    map: HashMap<String, Vec<u8>>,
    /// 每插件条目数（含带前缀键名计数）
    counts: HashMap<String, usize>,
}

/// 全局 KV（所有插件共享一个实例，键强制带 `<plugin-id>:` 前缀）。
#[derive(Default)]
pub struct KvStore {
    inner: Mutex<Inner>,
}

impl KvStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn namespaced(plugin_id: &str, key: &str) -> String {
        format!("{plugin_id}:{key}")
    }

    pub fn put(&self, plugin_id: &str, key: &str, value: &[u8]) -> Result<()> {
        if key.len() > MAX_KEY_LEN {
            bail!("kv key 超长（{} > {MAX_KEY_LEN}）", key.len());
        }
        if value.len() > MAX_VALUE_LEN {
            bail!("kv value 超长（{} > {MAX_VALUE_LEN}）", value.len());
        }
        let mut inner = self.inner.lock().expect("kv lock poisoned");
        let full = Self::namespaced(plugin_id, key);
        if !inner.map.contains_key(&full) {
            let count = inner.counts.entry(plugin_id.to_string()).or_insert(0);
            if *count >= MAX_ENTRIES_PER_PLUGIN {
                bail!("插件 {plugin_id} 的 kv 条目数已达上限 {MAX_ENTRIES_PER_PLUGIN}");
            }
            *count += 1;
        }
        inner.map.insert(full, value.to_vec());
        Ok(())
    }

    pub fn get(&self, plugin_id: &str, key: &str) -> Option<Vec<u8>> {
        let inner = self.inner.lock().expect("kv lock poisoned");
        inner.map.get(&Self::namespaced(plugin_id, key)).cloned()
    }

    pub fn delete(&self, plugin_id: &str, key: &str) -> bool {
        let mut inner = self.inner.lock().expect("kv lock poisoned");
        let full = Self::namespaced(plugin_id, key);
        if inner.map.remove(&full).is_some() {
            if let Some(c) = inner.counts.get_mut(plugin_id) {
                *c = c.saturating_sub(1);
            }
            true
        } else {
            false
        }
    }

    /// 卸载/禁用插件时清空其命名空间（注册-回收语义的一部分）。
    pub fn clear_plugin(&self, plugin_id: &str) {
        let mut inner = self.inner.lock().expect("kv lock poisoned");
        let prefix = format!("{plugin_id}:");
        inner.map.retain(|k, _| !k.starts_with(&prefix));
        inner.counts.remove(plugin_id);
    }

    /// 调试/测试：插件命名空间条目数
    pub fn len_for(&self, plugin_id: &str) -> usize {
        self.inner
            .lock()
            .expect("kv lock poisoned")
            .counts
            .get(plugin_id)
            .copied()
            .unwrap_or(0)
    }
}
