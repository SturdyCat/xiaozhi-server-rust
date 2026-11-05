//! 单个已装载插件：Store + bindgen 实例 + 带预算的调用包装。
//!
//! 每次导出调用前重置 fuel/epoch 预算（[`super::sandbox::arm_call_budget`]），
//! 陷阱经 [`super::sandbox::classify_trap`] 归类后浮出给 registry 决定 phase。
//! v0 单实例串行（调用方持锁）；P1 按性能预算引入无状态实例池。

use anyhow::Result;
use wasmtime::component::{Component, Linker};
use wasmtime::Store;

use super::contract::xiaozhi;
use super::contract::{ManifestDto, Plugin, RegistrationDto, TestResultDto};
use super::effects::Permissions;
use super::kv::KvStore;
use super::sandbox::{arm_call_budget, SandboxLimits};
use wasmtime::component::HasSelf;

/// wasmtime::Error（非 std::error::Error）→ anyhow 的转换点。
/// 用 `msg` 装箱而非字符串化：保留类型与内部链，sandbox::classify_trap
/// 才能解出 `wasmtime::Trap` 变体（Fuel/Epoch），否则只剩 backtrace 文本。
fn call_err(e: wasmtime::Error) -> anyhow::Error {
    anyhow::Error::msg(e)
}

/// Store 数据：宿主服务 trait 的实现载体 + 插件上下文。
///
/// `kv`/`permissions` 是宿主导入（kv-get）的执行依据：权限在 manifest 解析后
/// 由 registry 注入（见 [`LoadedPlugin::set_permissions`]），此前 kv-get 一律拒绝。
pub struct StoreData {
    pub plugin_id: String,
    budget: super::sandbox::MemoryBudget,
    kv: std::sync::Arc<KvStore>,
    permissions: Permissions,
}

// types 接口无宿主函数，但 bindgen 仍生成空 Host trait：需要显式空 impl
impl xiaozhi::plugin::types::Host for StoreData {}

impl xiaozhi::plugin::host::Host for StoreData {
    fn log(&mut self, level: String, message: String) {
        match level.as_str() {
            "warn" => tracing::warn!(plugin = %self.plugin_id, "{message}"),
            "error" => tracing::error!(plugin = %self.plugin_id, "{message}"),
            "debug" => tracing::debug!(plugin = %self.plugin_id, "{message}"),
            _ => tracing::info!(plugin = %self.plugin_id, "{message}"),
        }
    }

    fn now_ms(&mut self) -> u64 {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64
    }

    /// 读插件隔离 kv。未声明 `kv` 权限 → **deny-as-absent**（none + 审计告警）：
    /// 插件把它当「没有缓存」处理，而不是装载失败或运行期报错（温和拒绝形态）。
    fn kv_get(&mut self, key: String) -> Option<Vec<u8>> {
        if !self.permissions.kv {
            tracing::warn!(
                plugin = %self.plugin_id, key = %key,
                "kv-get 未声明 kv 权限（deny-as-absent）"
            );
            return None;
        }
        self.kv.get(&self.plugin_id, &key)
    }
}

/// 已实例化的插件（Store 与实例一一绑定，不可 Clone）。
pub struct LoadedPlugin {
    store: Store<StoreData>,
    plugin: Plugin,
    limits: SandboxLimits,
}

impl LoadedPlugin {
    /// 从组件二进制实例化。激活预算（fuel/epoch）给得比单次调用宽。
    /// `kv` 为宿主全局 kv（kv-get 导入按插件命名空间隔离访问）。
    pub fn instantiate(
        engine: &wasmtime::Engine,
        wasm: &[u8],
        plugin_id: &str,
        limits: SandboxLimits,
        kv: std::sync::Arc<KvStore>,
    ) -> Result<Self> {
        let component = Component::new(engine, wasm)
            .map_err(|e| anyhow::anyhow!("组件解析失败（{plugin_id}）：{e}"))?;
        let mut linker: Linker<StoreData> = Linker::new(engine);
        Plugin::add_to_linker::<_, HasSelf<StoreData>>(&mut linker, |data| data)?;
        let mut store = Store::new(
            engine,
            StoreData {
                plugin_id: plugin_id.to_string(),
                budget: super::sandbox::MemoryBudget::new(limits.max_memory_bytes),
                kv,
                // manifest 解析前为空权限：kv-get 一律 deny-as-absent
                permissions: Permissions::default(),
            },
        );
        store.limiter(|data| &mut data.budget as &mut dyn wasmtime::ResourceLimiter);
        arm_call_budget(
            &mut store,
            limits.activation_fuel,
            limits.activation_epoch_ticks,
        )?;
        let plugin = Plugin::instantiate(&mut store, &component, &linker).map_err(|e| {
            anyhow::anyhow!("实例化失败（{plugin_id}，world 不匹配或激活期陷阱）：{e}")
        })?;
        Ok(Self {
            store,
            plugin,
            limits,
        })
    }

    /// 注入 manifest 解析出的权限（registry 在 manifest 校验后、register 调用前执行；
    /// 宿主导入 kv-get 自此按声明放行）。
    pub fn set_permissions(&mut self, permissions: Permissions) {
        self.store.data_mut().permissions = permissions;
    }

    fn arm_call(&mut self) -> Result<()> {
        arm_call_budget(
            &mut self.store,
            self.limits.call_fuel,
            self.limits.call_epoch_ticks,
        )
    }

    /// manifest（激活期一次性）
    pub fn manifest(&mut self) -> Result<ManifestDto> {
        self.arm_call()?;
        let m = self
            .plugin
            .lifecycle()
            .call_manifest(&mut self.store)
            .map_err(call_err)?;
        Ok(m.into())
    }

    /// 注册包（激活期一次性）
    pub fn register(&mut self) -> Result<RegistrationDto> {
        self.arm_call()?;
        let r = self
            .plugin
            .lifecycle()
            .call_register(&mut self.store)
            .map_err(call_err)?;
        Ok(r.into())
    }

    /// 事件步进：ok = WIT 效果列表（由 registry 转 [`super::effects::Effect`]）
    pub fn handle_event(
        &mut self,
        event: &str,
        payload: &str,
    ) -> Result<Result<Vec<xiaozhi::plugin::types::Effect>, String>> {
        self.arm_call()?;
        self.plugin
            .lifecycle()
            .call_handle_event(&mut self.store, event, payload)
            .map_err(call_err)
    }

    /// 管理动作（管理台/语音工具双调用方）
    pub fn handle_action(
        &mut self,
        action_id: &str,
        payload: &str,
    ) -> Result<Result<String, String>> {
        self.arm_call()?;
        self.plugin
            .lifecycle()
            .call_handle_action(&mut self.store, action_id, payload)
            .map_err(call_err)
    }

    /// 候选验证自检
    pub fn self_test(&mut self) -> Result<Vec<TestResultDto>> {
        self.arm_call()?;
        match self
            .plugin
            .lifecycle()
            .call_self_test(&mut self.store)
            .map_err(call_err)?
        {
            Ok(results) => Ok(results.into_iter().map(Into::into).collect()),
            Err(e) => anyhow::bail!("self-test 整体失败：{e}"),
        }
    }

    /// 本次调用剩余 fuel（指标/调试；fuel 已耗尽时为 0）
    pub fn remaining_fuel(&mut self) -> u64 {
        self.store.get_fuel().unwrap_or(0)
    }

    pub fn plugin_id(&self) -> &str {
        &self.store.data().plugin_id
    }
}
