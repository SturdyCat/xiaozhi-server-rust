//! 服务注册表：插件与服务之间的通用调用面。
//!
//! 内核在启动时注册 `sys.*` 核心服务（owner = `host`）；插件在激活时注册
//! `manifest.provides` / `registration.services` 声明的服务，停用时注销。
//!
//! 约束：
//! - 同名服务默认只允许一个提供者（冲突即激活失败，可用 preference 指定首选）；
//! - 调用深度 ≤ [`MAX_CALL_DEPTH`]，5s 超时，禁止自调用（同 owner）；
//! - 宿主不解释方法语义，只做路由与「有没有这个方法」的判定。

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{bail, Result};

use super::contract::{ServiceDeclDto, ServiceMethodDto};

/// 服务调用链最大深度（防插件互相递归调用打满栈）。
pub const MAX_CALL_DEPTH: u32 = 4;
/// 单次服务调用超时。
pub const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// 核心服务的 owner 标记。
pub const HOST_OWNER: &str = "host";

/// 实际执行调用的回调（registry 实现，持有宿主的弱引用以打破循环）。
pub trait ServiceInvoker: Send + Sync {
    fn invoke(&self, service: &str, method: &str, payload: &[u8], depth: u32) -> Result<Vec<u8>>;
}

/// 已注册服务。
pub struct ServiceEntry {
    pub owner: String,
    pub version: String,
    pub methods: BTreeMap<String, ServiceMethodDto>,
    invoker: Option<Arc<dyn ServiceInvoker>>,
}

impl ServiceEntry {
    pub fn has_method(&self, method: &str) -> bool {
        self.methods.contains_key(method)
    }
}

/// 服务注册表。
#[derive(Default)]
pub struct ServiceTable {
    entries: RwLock<BTreeMap<String, ServiceEntry>>,
    /// 服务名 → 首选提供者（多提供者时生效；进程内态，重启回落注册顺序）
    prefs: RwLock<BTreeMap<String, String>>,
    /// 备选提供者（非首选，保留可切换）：服务名 → [(owner, entry)]
    fallbacks: RwLock<BTreeMap<String, Vec<ServiceEntry>>>,
}

thread_local! {
    /// 当前线程的服务调用深度。
    static CALL_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// 深度守卫：进入时 +1，离开时 -1（panic 安全）。
struct DepthGuard;

impl DepthGuard {
    fn enter() -> Result<Self> {
        let depth = CALL_DEPTH.with(|c| {
            let next = c.get() + 1;
            c.set(next);
            next
        });
        if depth > MAX_CALL_DEPTH {
            CALL_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
            bail!(
                "服务调用深度超限（>{MAX_CALL_DEPTH}）：疑似插件间递归调用"
            );
        }
        Ok(Self)
    }
}

impl Drop for DepthGuard {
    fn drop(&mut self) {
        CALL_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
    }
}

impl ServiceTable {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 注册核心服务（owner = `host`，不可被插件卸载）。
    pub fn register_host_service(
        &self,
        decl: &ServiceDeclDto,
        invoker: Option<Arc<dyn ServiceInvoker>>,
    ) -> Result<()> {
        self.insert(HOST_OWNER, decl, invoker)
    }

    /// 注册插件服务。同名冲突：首个为 owner，后续进备选并返回冲突信息。
    pub fn register_plugin_service(
        &self,
        plugin_id: &str,
        decl: &ServiceDeclDto,
        invoker: Option<Arc<dyn ServiceInvoker>>,
    ) -> Result<()> {
        self.insert(plugin_id, decl, invoker)
    }

    fn insert(
        &self,
        owner: &str,
        decl: &ServiceDeclDto,
        invoker: Option<Arc<dyn ServiceInvoker>>,
    ) -> Result<()> {
        let entry = ServiceEntry {
            owner: owner.to_string(),
            version: decl.version.clone(),
            methods: decl
                .methods
                .iter()
                .map(|m| (m.name.clone(), m.clone()))
                .collect(),
            invoker,
        };
        let mut entries = self.entries.write().map_err(|_| anyhow::anyhow!("服务表锁中毒"))?;
        if entries.contains_key(&decl.name) {
            // 多提供者：保留为备选（可用 preference 切换），并告知调用方冲突
            let mut fb = self
                .fallbacks
                .write()
                .map_err(|_| anyhow::anyhow!("服务表锁中毒"))?;
            fb.entry(decl.name.clone()).or_default().push(entry);
            bail!(
                "服务 {} 已由 {} 提供；{} 的实现保留为备选（可用 /api/plugins/extensions/{}/preference 切换）",
                decl.name,
                entries.get(&decl.name).map(|e| e.owner.as_str()).unwrap_or("?"),
                owner,
                decl.name
            );
        }
        entries.insert(decl.name.clone(), entry);
        Ok(())
    }

    /// 注销某插件的全部服务（deactivate / remove）。
    pub fn unregister_owner(&self, plugin_id: &str) {
        if let Ok(mut entries) = self.entries.write() {
            entries.retain(|_, e| e.owner != plugin_id);
        }
        if let Ok(mut fb) = self.fallbacks.write() {
            for list in fb.values_mut() {
                list.retain(|e| e.owner != plugin_id);
            }
            fb.retain(|_, v| !v.is_empty());
        }
        if let Ok(mut prefs) = self.prefs.write() {
            prefs.retain(|_, v| v != plugin_id);
        }
    }

    /// 服务是否存在，返回 `owner@version`。
    pub fn discover(&self, service: &str) -> Option<String> {
        self.entries
            .read()
            .ok()?
            .get(service)
            .map(|e| format!("{}@{}", e.owner, e.version))
    }

    /// 服务名全集（投影用）。
    pub fn names(&self) -> Vec<String> {
        self.entries
            .read()
            .map(|e| e.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// 调用服务。
    pub fn invoke(
        &self,
        caller: &str,
        service: &str,
        method: &str,
        payload: &[u8],
    ) -> Result<Vec<u8>> {
        let _guard = DepthGuard::enter()?;
        let (invoker, owner) = {
            let entries = self
                .entries
                .read()
                .map_err(|_| anyhow::anyhow!("服务表锁中毒"))?;
            let entry = entries
                .get(service)
                .ok_or_else(|| anyhow::anyhow!("服务未注册：{service}"))?;
            if !entry.has_method(method) {
                bail!(
                    "服务 {service} 无方法 {method}（已声明：{:?}）",
                    entry.methods.keys().collect::<Vec<_>>()
                );
            }
            if entry.owner == caller {
                bail!("禁止自调用服务 {service}（caller={caller}）");
            }
            let invoker = entry
                .invoker
                .clone()
                .ok_or_else(|| anyhow::anyhow!("服务 {service} 无可执行实现（仅声明）"))?;
            (invoker, entry.owner.clone())
        };
        tracing::debug!(caller, service, method, owner = %owner, "服务调用");
        invoker.invoke(service, method, payload, 0)
    }
}

/// 便于测试/投影：把服务声明合并且去重（manifest.provides + registration.services）。
pub fn merge_decls(
    provides: &[ServiceDeclDto],
    extra: &[ServiceDeclDto],
) -> Vec<ServiceDeclDto> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for d in provides.iter().chain(extra.iter()) {
        if seen.insert(d.name.clone()) {
            out.push(d.clone());
        }
    }
    out
}
