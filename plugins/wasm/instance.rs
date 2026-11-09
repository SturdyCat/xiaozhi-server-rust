//! 单个已装载插件：Store + bindgen 实例 + 带预算的调用包装。
//!
//! 每次导出调用前重置 fuel/epoch 预算（[`super::sandbox::arm_call_budget`]），
//! 陷阱经 [`super::sandbox::classify_trap`] 归类后浮出给 registry 决定 phase。
//!
//! 四个宿主服务（`host-core` / `host-net` / `host-timer` / `host-system`）全部在
//! [`StoreData`] 上实现，一律**先过权限门，再委托 [`HostContext`]**——本文件不含任何
//! 网络/文件/存储逻辑，也不认识任何业务概念。

use anyhow::Result;
use wasmtime::component::{Component, Linker};
use wasmtime::Store;

use super::contract::xiaozhi;
use super::contract::{ManifestDto, Plugin, ProtocolDto, RegistrationDto, TestResultDto};
use super::effects::Permissions;
use super::host_api::HostContext;
use super::sandbox::{arm_call_budget, SandboxLimits};

/// `host-system` 实现（会话/设备事实 + 文件沙箱）拆到 [子模块](self::host_system)。
mod host_system;
use wasmtime::component::HasSelf;

/// wasmtime::Error（非 std::error::Error）→ anyhow 的转换点。
/// 用 `msg` 装箱而非字符串化：保留类型与内部链，sandbox::classify_trap
/// 才能解出 `wasmtime::Trap` 变体（Fuel/Epoch），否则只剩 backtrace 文本。
fn call_err(e: wasmtime::Error) -> anyhow::Error {
    anyhow::Error::msg(e)
}

/// `host-core.random` 单次上限（宿主侧第二道防线，guest SDK 侧无预检）。
const MAX_RANDOM_BYTES: u32 = 1024;

fn denied(plugin_id: &str, what: &str, perm: &str) -> String {
    format!("插件 {plugin_id} 未声明 `{perm}` 权限，拒绝 {what}")
}

/// Store 数据：宿主服务 trait 的实现载体 + 插件上下文。
///
/// 权限在 manifest 解析后由 registry 注入（见 [`LoadedPlugin::set_permissions`]），
/// 此前所有受限能力一律拒绝（默认拒绝，不做「温和降级」）。
pub struct StoreData {
    pub plugin_id: String,
    budget: super::sandbox::MemoryBudget,
    ctx: std::sync::Arc<HostContext>,
    permissions: Permissions,
}

impl StoreData {
    fn pid(&self) -> &str {
        &self.plugin_id
    }
}

// types 接口无宿主函数，但 bindgen 仍生成空 Host trait：需要显式空 impl
impl xiaozhi::plugin::types::Host for StoreData {}

// ============================================================
// host-core：日志 / 时钟 / kv / 配置 / 事件 / 服务
// ============================================================

impl xiaozhi::plugin::host_core::Host for StoreData {
    fn log(&mut self, level: String, message: String) {
        match level.as_str() {
            "warn" => tracing::warn!(plugin = %self.plugin_id, "{message}"),
            "error" => tracing::error!(plugin = %self.plugin_id, "{message}"),
            "debug" => tracing::debug!(plugin = %self.plugin_id, "{message}"),
            _ => tracing::info!(plugin = %self.plugin_id, "{message}"),
        }
    }

    fn now_ms(&mut self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn monotonic_ms(&mut self) -> u64 {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64
    }

    /// 密码学随机字节：单次 1..=[`MAX_RANDOM_BYTES`]（防插件用超大分配放大），
    /// 委托宿主 OS 熵源；熵源不可用显式 Err（插件决定降级或失败）。
    fn random(&mut self, len: u32) -> Result<Vec<u8>, String> {
        if len == 0 || len > MAX_RANDOM_BYTES {
            return Err(format!(
                "random 长度越界（{len}，允许 1..={MAX_RANDOM_BYTES}）"
            ));
        }
        self.ctx.rng.fill(len as usize)
    }

    /// 读插件隔离 kv。未声明 `kv` 权限 → **deny-as-absent**（none + 审计告警）：
    /// 插件把它当「没有缓存」处理，而不是装载失败或运行期报错。
    fn kv_get(&mut self, key: String) -> Option<Vec<u8>> {
        if !self.permissions.kv {
            tracing::warn!(
                plugin = %self.plugin_id, key = %key,
                "kv-get 未声明 kv 权限（deny-as-absent）"
            );
            return None;
        }
        self.ctx.kv.get(&self.plugin_id, &key)
    }

    fn kv_put(&mut self, key: String, value: Vec<u8>) -> std::result::Result<(), String> {
        if !self.permissions.kv {
            return Err(denied(&self.plugin_id, "kv-put", "kv"));
        }
        self.ctx
            .kv
            .put(&self.plugin_id, &key, &value)
            .map_err(|e| e.to_string())
    }

    fn kv_delete(&mut self, key: String) -> bool {
        if !self.permissions.kv {
            tracing::warn!(plugin = %self.plugin_id, key = %key, "kv-delete 未声明 kv 权限（deny）");
            return false;
        }
        self.ctx.kv.delete(&self.plugin_id, &key)
    }

    fn kv_keys(&mut self, prefix: String) -> Vec<String> {
        if !self.permissions.kv {
            return Vec::new();
        }
        self.ctx.kv.keys(&self.plugin_id, &prefix)
    }

    fn config_get(&mut self, key: String) -> Option<String> {
        if !self.permissions.config {
            tracing::warn!(plugin = %self.plugin_id, key = %key, "config-get 未声明 config 权限");
            return None;
        }
        self.ctx.config.get(&self.plugin_id, &key)
    }

    fn config_secret(&mut self, key: String) -> Option<String> {
        if !self.permissions.config {
            tracing::warn!(plugin = %self.plugin_id, key = %key, "config-secret 未声明 config 权限");
            return None;
        }
        tracing::info!(plugin = %self.plugin_id, key = %key, "config-secret 读取（审计）");
        self.ctx.config.get_secret(&self.plugin_id, &key)
    }

    fn config_set(&mut self, key: String, value_json: String) -> std::result::Result<(), String> {
        if !self.permissions.config {
            return Err(denied(&self.plugin_id, "config-set", "config"));
        }
        self.ctx
            .config
            .set(&self.plugin_id, &key, &value_json)
            .map_err(|e| e.to_string())
    }

    fn publish(&mut self, topic: String, payload: Vec<u8>) -> std::result::Result<bool, String> {
        if !self.permissions.publish {
            return Err(denied(&self.plugin_id, "publish", "pub"));
        }
        if !super::event_bus::publish_allowed(&self.plugin_id, &topic) {
            return Err(format!(
                "插件 {} 只能发布 `{}.*` 命名空间下的 topic（请求：{topic}）",
                self.plugin_id, self.plugin_id
            ));
        }
        Ok(self.ctx.bus.publish(&super::event_bus::Envelope {
            topic,
            correlation_id: None,
            payload,
        }))
    }

    fn call_service(
        &mut self,
        service: String,
        method: String,
        payload: Vec<u8>,
    ) -> std::result::Result<Vec<u8>, String> {
        if !self.permissions.services {
            return Err(denied(&self.plugin_id, "call-service", "services"));
        }
        self.ctx
            .services
            .invoke(&self.plugin_id, &service, &method, &payload)
            .map_err(|e| format!("{e:#}"))
    }

    fn discover_service(&mut self, service: String) -> Option<String> {
        self.ctx.services.discover(&service)
    }
}

// ============================================================
// host-net：socket 监听 / 出站 / HTTP 应答 / mDNS
// ============================================================

impl xiaozhi::plugin::host_net::Host for StoreData {
    fn listen(
        &mut self,
        spec: xiaozhi::plugin::types::EndpointSpec,
    ) -> std::result::Result<u64, String> {
        if !self.permissions.net {
            return Err(denied(&self.plugin_id, "listen", "net"));
        }
        let dto = super::contract::EndpointSpecDto {
            name: spec.name,
            protocol: match spec.protocol {
                xiaozhi::plugin::types::Protocol::Http => ProtocolDto::Http,
                xiaozhi::plugin::types::Protocol::Tcp => ProtocolDto::Tcp,
                xiaozhi::plugin::types::Protocol::Udp => ProtocolDto::Udp,
                xiaozhi::plugin::types::Protocol::Ws => ProtocolDto::Ws,
            },
            bind: spec.bind,
            port: spec.port,
            path: spec.path,
            required: spec.required,
        };
        self.ctx
            .net
            .listen(&self.plugin_id, &dto)
            .map(|(id, port)| {
                tracing::info!(
                    plugin = %self.plugin_id, endpoint = id, port,
                    protocol = dto.protocol.as_str(), "插件端点已绑定"
                );
                id
            })
            .map_err(|e| format!("{e:#}"))
    }

    fn unlisten(&mut self, endpoint_handle: u64) -> std::result::Result<(), String> {
        self.ctx
            .net
            .unlisten(&self.plugin_id, endpoint_handle)
            .map_err(|e| format!("{e:#}"))
    }

    fn net_send(
        &mut self,
        connection_id: u64,
        payload: Vec<u8>,
    ) -> std::result::Result<u64, String> {
        self.ctx
            .net
            .send(&self.plugin_id, connection_id, &payload)
            .map_err(|e| format!("{e:#}"))
    }

    fn net_close(&mut self, connection_id: u64, reason: String) -> std::result::Result<(), String> {
        self.ctx
            .net
            .close(&self.plugin_id, connection_id, &reason)
            .map_err(|e| format!("{e:#}"))
    }

    fn net_peer(&mut self, connection_id: u64) -> Option<String> {
        self.ctx.net.peer(connection_id)
    }

    /// S3l：出站 TCP 连接。`net` 权限门（与 udp-send 同口径）；协议语义
    /// （分帧/握手/加密）在插件侧，宿主只搬运字节并回灌 `io.tcp.*` 事件。
    fn tcp_connect(&mut self, host: String, port: u16) -> std::result::Result<u64, String> {
        if !self.permissions.net {
            return Err(denied(&self.plugin_id, "tcp-connect", "net"));
        }
        self.ctx
            .net
            .tcp_connect(&self.plugin_id, &host, port)
            .map_err(|e| format!("{e:#}"))
    }

    fn udp_send(
        &mut self,
        socket_handle: u64,
        peer: String,
        payload: Vec<u8>,
    ) -> std::result::Result<u64, String> {
        if !self.permissions.net {
            return Err(denied(&self.plugin_id, "udp-send", "net"));
        }
        self.ctx
            .net
            .udp_send(&self.plugin_id, socket_handle, &peer, &payload)
            .map_err(|e| format!("{e:#}"))
    }

    fn udp_join_multicast(
        &mut self,
        socket_handle: u64,
        group: String,
    ) -> std::result::Result<(), String> {
        if !self.permissions.net {
            return Err(denied(&self.plugin_id, "udp-join-multicast", "net"));
        }
        self.ctx
            .net
            .udp_join_multicast(&self.plugin_id, socket_handle, &group)
            .map_err(|e| format!("{e:#}"))
    }

    fn http_respond(
        &mut self,
        request_id: String,
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> std::result::Result<(), String> {
        self.ctx
            .net
            .respond_http(&self.plugin_id, &request_id, status, headers, body)
            .map_err(|e| format!("{e:#}"))
    }

    fn mdns_advertise(
        &mut self,
        service_type: String,
        instance: String,
        port: u16,
        txt: Vec<(String, String)>,
    ) -> std::result::Result<u64, String> {
        if !self.permissions.mdns {
            return Err(denied(&self.plugin_id, "mdns-advertise", "mdns"));
        }
        self.ctx
            .mdns
            .advertise(&self.plugin_id, &service_type, &instance, port, &txt)
            .map_err(|e| format!("{e:#}"))
    }

    fn mdns_unadvertise(&mut self, handle: u64) -> std::result::Result<(), String> {
        self.ctx
            .mdns
            .unadvertise(&self.plugin_id, handle)
            .map_err(|e| format!("{e:#}"))
    }
}

// ============================================================
// host-timer
// ============================================================

impl xiaozhi::plugin::host_timer::Host for StoreData {
    fn set_timeout(&mut self, delay_ms: u64) -> std::result::Result<u64, String> {
        if !self.permissions.timer {
            return Err(denied(&self.plugin_id, "set-timeout", "timer"));
        }
        self.ctx
            .timers
            .set_timeout(&self.plugin_id, delay_ms)
            .map_err(|e| format!("{e:#}"))
    }

    fn set_interval(&mut self, interval_ms: u64) -> std::result::Result<u64, String> {
        if !self.permissions.timer {
            return Err(denied(&self.plugin_id, "set-interval", "timer"));
        }
        self.ctx
            .timers
            .set_interval(&self.plugin_id, interval_ms)
            .map_err(|e| format!("{e:#}"))
    }

    fn clear_timer(&mut self, timer_id: u64) -> std::result::Result<(), String> {
        self.ctx
            .timers
            .clear(&self.plugin_id, timer_id)
            .map_err(|e| format!("{e:#}"))
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
    pub fn instantiate(
        engine: &wasmtime::Engine,
        wasm: &[u8],
        plugin_id: &str,
        limits: SandboxLimits,
        ctx: std::sync::Arc<HostContext>,
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
                ctx,
                // manifest 解析前为空权限：所有受限能力默认拒绝
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

    /// 注入 manifest 解析出的权限（registry 在 manifest 校验后、register 调用前执行）。
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
        event: xiaozhi::plugin::types::Event,
    ) -> Result<Result<Vec<xiaozhi::plugin::types::Effect>, String>> {
        self.arm_call()?;
        self.plugin
            .lifecycle()
            .call_handle_event(&mut self.store, &event)
            .map_err(call_err)
    }

    /// 服务调用入口（其他插件 `call-service` 的目标）
    pub fn handle_service(
        &mut self,
        service: &str,
        method: &str,
        payload: &[u8],
    ) -> Result<Result<Vec<u8>, String>> {
        self.arm_call()?;
        self.plugin
            .lifecycle()
            .call_handle_service(&mut self.store, service, method, payload)
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
