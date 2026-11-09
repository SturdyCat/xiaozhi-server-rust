//! `WasmHost`：WASM 插件平台的门面——装载/激活/phase/快照/事件分发/扩展点解析。
//!
//! ## 装载语义
//!
//! - **先建新、后替换**：热装载先完整走「实例化 → manifest 校验 → 权限解析 →
//!   register 校验」，全过才替换旧实例；失败保留旧实例并记 `Failed`；
//! - **phase 与 enabled 分离**：enabled 是配置意图，phase 是运行事实；
//! - **陷阱 → Degraded**：调用期 fuel/epoch/内存/trap 不 panic、不卸载，置 Degraded
//!   并保留实例（下次调用重置预算重试）；连续失败由扩展点解析自动回退官方默认；
//! - **注册-回收**：unload 清 kv 命名空间、事件订阅、端点、服务、定时器、mDNS 广播
//!   与配置命名空间（DSH「注册随上下文回收」）。
//!
//! ## 并发模型
//!
//! - 条目表锁只做**查找/增删**，绝不跨 guest 调用持有；
//! - 每插件一个 [`Runner`]（实例 Arc + 权限 + phase 单元 + 有界入站队列），
//!   互不阻塞 = wasmtime 单 Store 串行的正确粒度；
//! - `publish` / socket / 定时器一律 `try_send`，永不阻塞发布方（依赖环上安全）；
//! - guest 调用在 `spawn_blocking` 线程上跑（wasmtime 是同步 API）。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tokio::sync::mpsc;

use super::config_store::{MemoryConfigStore, PluginConfigStore};
use super::contract::{validate_manifest, validate_registration, ManifestDto, RegistrationDto};
use super::effects::{self, Effect, HttpFetcher, Permissions, ReqwestFetcher};
use super::event_bus::{Envelope, EventBus};
use super::fs::FsSandbox;
use super::host_api::{EmptySystemFacts, HostContext, SystemFacts};
use super::instance::LoadedPlugin;
use super::kv::KvStore;
use super::lifecycle::Lifecycle;
use super::mdns::MdnsAdvertiser;
use super::net::{NetManager, NioSink};
use super::resolver::{FlatWorld, Resolution, ResolutionTable};
use super::sandbox::{build_engine, classify_trap, EpochTicker, SandboxLimits, TrapKind};
use super::services::{ServiceInvoker, ServiceTable};
use super::timers::{TimerSink, TimerWheel};

/// http 回灌深度上限（初始事件 + 最多 N 次 `http.responses` 批量续步）。
/// 防插件「回灌再发请求」自我循环；超限停止回灌、记错误，不判插件死刑。
const MAX_HTTP_FEEDBACK_DEPTH: usize = 4;

/// 事件队列默认深度（manifest.resources.max-queued-events 为 0 时）。
const DEFAULT_QUEUE: usize = 256;

/// 插件运行 phase（enabled 是配置意图，由配置管线持有，不在这）。
/// 只在 lifecycle == Active 的条目上有意义；非 Active 条目统一 [`super::lifecycle::inactive_phase`]。
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// 已装载并激活
    Active,
    /// 激活失败（原因；不替换旧实例、不影响其他插件）
    Failed(String),
    /// 运行期陷阱/效果拒绝（保留实例，预算下次重置重试）
    Degraded { kind: TrapKind, detail: String },
}

impl Phase {
    fn as_str(&self) -> &str {
        match self {
            Phase::Active => "active",
            Phase::Failed(_) => "failed",
            Phase::Degraded { .. } => "degraded",
        }
    }
}

/// phase 单元：条目表投影与事件泵共享同一份运行事实（不复制状态）。
pub(super) type PhaseRef = Arc<Mutex<Phase>>;

pub(super) fn phase_ref(p: Phase) -> PhaseRef {
    Arc::new(Mutex::new(p))
}

/// 注册表条目（生命周期字段见 [`Lifecycle`]；phase 仅 Active 态有意义）。
pub(super) struct PluginEntry {
    pub(super) manifest: ManifestDto,
    pub(super) registration: RegistrationDto,
    pub(super) permissions: Permissions,
    pub(super) phase: PhaseRef,
    /// 单实例串行：锁粒度是插件实例，不是整张表。
    pub(super) instance: Option<Arc<Mutex<LoadedPlugin>>>,
    pub(super) lifecycle: Lifecycle,
    /// 事件入站队列（激活且有运行时时建立；drop 即泵退出）。
    pub(super) inbox: Option<mpsc::Sender<Envelope>>,
}

impl PluginEntry {
    pub(super) fn phase_snapshot(&self) -> Phase {
        self.phase
            .lock()
            .map(|p| p.clone())
            .unwrap_or_else(|_| Phase::Failed("phase 锁中毒".to_string()))
    }

    pub(super) fn set_phase(&self, p: Phase) {
        if let Ok(mut guard) = self.phase.lock() {
            *guard = p;
        }
    }

    /// 运行期句柄（事件泵/服务调用/管理动作用的稳定入口）。
    pub(super) fn runner(&self, plugin_id: &str) -> Option<Runner> {
        let instance = self.instance.clone()?;
        Some(Runner {
            plugin_id: plugin_id.to_string(),
            instance,
            permissions: self.permissions.clone(),
            phase: Arc::clone(&self.phase),
        })
    }
}

/// 单插件的运行期束：跨锁边界传递 guest 调用所需的一切。
#[derive(Clone)]
pub(super) struct Runner {
    pub(super) plugin_id: String,
    pub(super) instance: Arc<Mutex<LoadedPlugin>>,
    pub(super) permissions: Permissions,
    pub(super) phase: PhaseRef,
}

impl Runner {
    fn set_phase(&self, p: Phase) {
        if let Ok(mut guard) = self.phase.lock() {
            *guard = p;
        }
    }

    fn mark_degraded(&self, kind: TrapKind, detail: String) -> anyhow::Error {
        tracing::warn!(plugin = %self.plugin_id, ?kind, "{detail}");
        self.set_phase(Phase::Degraded {
            kind,
            detail: detail.clone(),
        });
        anyhow::anyhow!("插件 {} 调用陷阱（{kind:?}）：{detail}", self.plugin_id)
    }
}

/// 每插件事件入站表：socket 层 / 定时器层只认识它，不认识注册表（无环依赖）。
#[derive(Default)]
pub struct InboxTable {
    map: Mutex<BTreeMap<String, mpsc::Sender<Envelope>>>,
}

impl InboxTable {
    fn set(&self, plugin_id: &str, tx: mpsc::Sender<Envelope>) {
        if let Ok(mut m) = self.map.lock() {
            m.insert(plugin_id.to_string(), tx);
        }
    }

    fn remove(&self, plugin_id: &str) {
        if let Ok(mut m) = self.map.lock() {
            m.remove(plugin_id);
        }
    }

    /// 投递（非阻塞；插件未激活 / 队列满 → false）。
    pub fn deliver(&self, plugin_id: &str, env: Envelope) -> bool {
        let Ok(m) = self.map.lock() else {
            return false;
        };
        match m.get(plugin_id) {
            Some(tx) => tx.try_send(env).is_ok(),
            None => false,
        }
    }

    pub fn len(&self) -> usize {
        self.map.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// 网络/定时器事件汇入入站表。
struct InboxSink {
    inbox: Arc<InboxTable>,
}

impl NioSink for InboxSink {
    fn deliver(&self, plugin_id: &str, env: Envelope) {
        if !self.inbox.deliver(plugin_id, env) {
            tracing::debug!(plugin = plugin_id, "插件入站队列不可用，网络事件丢弃");
        }
    }
}

impl TimerSink for InboxSink {
    fn fired(&self, plugin_id: &str, timer_id: u64, kind: &str) {
        let env = Envelope::json(
            "host.timer.fired",
            &serde_json::json!({ "timer_id": timer_id, "kind": kind }),
        );
        if !self.inbox.deliver(plugin_id, env) {
            tracing::debug!(
                plugin = plugin_id,
                timer_id,
                "插件入站队列不可用，定时事件丢弃"
            );
        }
    }
}

/// 效果执行所需的宿主件（克隆成本极低；泵任务各持一份）。
#[derive(Clone)]
pub(super) struct ExecEnv {
    pub(super) ctx: Arc<HostContext>,
    pub(super) fetcher: Arc<dyn HttpFetcher>,
}

/// 宿主装配选项（缺省为「内存配置 + 临时目录 + 空事实 + 当前运行时」）。
pub struct HostOptions {
    pub runtime: Option<tokio::runtime::Handle>,
    pub data_dir: std::path::PathBuf,
    pub config: Option<Arc<dyn PluginConfigStore>>,
    pub facts: Option<Arc<dyn SystemFacts>>,
}

impl Default for HostOptions {
    fn default() -> Self {
        Self {
            runtime: tokio::runtime::Handle::try_current().ok(),
            data_dir: std::env::temp_dir(),
            config: None,
            facts: None,
        }
    }
}

impl HostOptions {
    /// 生产装配：注入运行时、数据目录、配置后端与事实来源。
    pub fn production(
        runtime: tokio::runtime::Handle,
        data_dir: impl Into<std::path::PathBuf>,
        config: Arc<dyn PluginConfigStore>,
        facts: Arc<dyn SystemFacts>,
    ) -> Self {
        Self {
            runtime: Some(runtime),
            data_dir: data_dir.into(),
            config: Some(config),
            facts: Some(facts),
        }
    }
}

/// 宿主：引擎 + 注册表 + 通用服务上下文 + 解析表 + http 执行器。
pub struct WasmHost {
    pub(super) engine: wasmtime::Engine,
    pub(super) limits: SandboxLimits,
    pub(super) entries: Mutex<BTreeMap<String, PluginEntry>>,
    pub(super) ctx: Arc<HostContext>,
    inbox: Arc<InboxTable>,
    fetcher: Arc<dyn HttpFetcher>,
    resolver: Mutex<ResolutionTable>,
    /// 受限高层 LAN 命令队列；native connector 可按批 drain。
    runtime: Option<tokio::runtime::Handle>,
    /// epoch 递增线程句柄（Drop 停止；字段持有即活着）
    _ticker: EpochTicker,
}

impl WasmHost {
    pub fn new(limits: SandboxLimits) -> Result<Self> {
        Self::with_options(
            limits,
            Arc::new(ReqwestFetcher::default()),
            HostOptions::default(),
        )
    }

    /// 测试/嵌入方注入 fetcher（脚本化假实现，零真实网络）。
    pub fn with_fetcher(limits: SandboxLimits, fetcher: Arc<dyn HttpFetcher>) -> Result<Self> {
        Self::with_options(limits, fetcher, HostOptions::default())
    }

    pub fn with_options(
        limits: SandboxLimits,
        fetcher: Arc<dyn HttpFetcher>,
        opts: HostOptions,
    ) -> Result<Self> {
        let engine = build_engine().context("构建 wasm 引擎失败")?;
        let ticker = EpochTicker::start(&engine);

        let inbox = Arc::new(InboxTable::default());
        let sink = Arc::new(InboxSink {
            inbox: Arc::clone(&inbox),
        });
        let net = NetManager::new(opts.runtime.clone(), sink.clone());
        let timers = TimerWheel::new(opts.runtime.clone(), sink);
        let mdns = {
            let (m, started) = MdnsAdvertiser::new();
            if let Err(e) = started {
                tracing::warn!("{e}（插件 mDNS 广播不可用，其余能力不受影响）");
            }
            m
        };
        let facts: Arc<dyn SystemFacts> = opts.facts.unwrap_or_else(|| {
            Arc::new(EmptySystemFacts {
                data_dir: opts.data_dir.clone(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            })
        });
        let config: Arc<dyn PluginConfigStore> = opts
            .config
            .unwrap_or_else(|| MemoryConfigStore::new() as Arc<dyn PluginConfigStore>);

        let ctx = HostContext::new(
            Arc::new(KvStore::new()),
            config,
            EventBus::new(),
            ServiceTable::new(),
            net,
            timers,
            Arc::new(FsSandbox::new(opts.data_dir.join("plugins"))),
            mdns,
            facts,
        );

        Ok(Self {
            engine,
            entries: Mutex::new(BTreeMap::new()),
            ctx,
            inbox,
            fetcher,
            resolver: Mutex::new(ResolutionTable::new()),
            runtime: opts.runtime,
            limits,
            _ticker: ticker,
        })
    }

    fn exec_env(&self) -> ExecEnv {
        ExecEnv {
            ctx: Arc::clone(&self.ctx),
            fetcher: Arc::clone(&self.fetcher),
        }
    }

    /// 装载并激活：全链路成功才落表（新实例替换旧实例）。
    pub fn load(&self, expected_id: &str, wasm: &[u8]) -> Result<()> {
        let mut entry = self.build_entry(expected_id, wasm)?;
        entry.lifecycle = Lifecycle::Active;
        let previous = {
            let mut entries = self.entries.lock().expect("entries lock poisoned");
            entries.insert(expected_id.to_string(), entry)
        };
        if let Some(old) = previous.as_ref() {
            self.release_entry(old);
        }
        if let Err(e) = self.arm(expected_id) {
            self.unload(expected_id);
            return Err(e);
        }
        tracing::info!(plugin = expected_id, "插件装载并激活");
        Ok(())
    }

    pub(super) fn build_entry(&self, expected_id: &str, wasm: &[u8]) -> Result<PluginEntry> {
        let mut instance = LoadedPlugin::instantiate(
            &self.engine,
            wasm,
            expected_id,
            self.limits,
            Arc::clone(&self.ctx),
        )
        .with_context(|| format!("插件 {expected_id} 实例化失败"))?;
        let manifest = instance
            .manifest()
            .with_context(|| format!("插件 {expected_id} manifest() 失败"))?;
        if manifest.id != expected_id {
            anyhow::bail!(
                "manifest.id ({}) 与装载请求的 id ({expected_id}) 不一致（拒绝装载）",
                manifest.id
            );
        }
        validate_manifest(&manifest)
            .with_context(|| format!("插件 {expected_id} manifest 校验失败"))?;
        let permissions = Permissions::parse(&manifest.permissions)
            .with_context(|| format!("插件 {expected_id} 权限声明非法"))?;
        // 权限注入须在 register 之前：注册期调用的宿主导入（kv-get）已按声明放行
        instance.set_permissions(permissions.clone());
        let registration = instance
            .register()
            .with_context(|| format!("插件 {expected_id} register() 失败"))?;
        validate_registration(&registration)
            .with_context(|| format!("插件 {expected_id} 注册包校验失败"))?;
        Ok(PluginEntry {
            manifest,
            registration,
            permissions,
            phase: phase_ref(Phase::Active),
            instance: Some(Arc::new(Mutex::new(instance))),
            lifecycle: Lifecycle::Active,
            inbox: None,
        })
    }

    /// 激活期装配：事件订阅 + 端点监听 + 服务注册。
    pub(super) fn arm(&self, plugin_id: &str) -> Result<()> {
        let (runner, subs, endpoints, resources, provides, extra) = {
            let entries = self.entries.lock().expect("entries lock poisoned");
            let e = entries
                .get(plugin_id)
                .ok_or_else(|| anyhow::anyhow!("插件 {plugin_id} 未装载"))?;
            let Some(runner) = e.runner(plugin_id) else {
                anyhow::bail!(
                    "插件 {plugin_id} 无运行实例（lifecycle={}）",
                    e.lifecycle.as_str()
                );
            };
            (
                runner,
                e.manifest.subscriptions.clone(),
                e.manifest.endpoints.clone(),
                e.manifest.resources.clone(),
                e.manifest.provides.clone(),
                e.registration.services.clone(),
            )
        };

        // ① 事件订阅：一插件一条有界队列 + 一个泵任务
        if !subs.is_empty() {
            match &self.runtime {
                Some(rt) => {
                    let cap = if resources.max_queued_events == 0 {
                        DEFAULT_QUEUE
                    } else {
                        resources.max_queued_events as usize
                    };
                    let (tx, rx) = mpsc::channel::<Envelope>(cap);
                    if let Ok(mut entries) = self.entries.lock() {
                        if let Some(e) = entries.get_mut(plugin_id) {
                            e.inbox = Some(tx.clone());
                        }
                    }
                    self.inbox.set(plugin_id, tx.clone());
                    for sub in &subs {
                        let filter = sub
                            .filter_json
                            .as_deref()
                            .and_then(|f| serde_json::from_str(f).ok());
                        self.ctx
                            .bus
                            .subscribe(plugin_id, &sub.topic, filter, tx.clone());
                    }
                    spawn_pump(rt.clone(), runner.clone(), self.exec_env(), rx);
                    tracing::debug!(
                        plugin = plugin_id,
                        topics = subs.len(),
                        cap,
                        "事件订阅已装配"
                    );
                }
                None => tracing::warn!(
                    plugin = plugin_id,
                    "无 tokio 运行时，事件订阅未生效（仅同步派发可用）"
                ),
            }
        }

        // ② 端点：宿主代监听，连接事件按订阅路由
        for spec in &endpoints {
            match self.ctx.net.listen(plugin_id, spec) {
                Ok((id, port)) => tracing::info!(
                    plugin = plugin_id,
                    endpoint = id,
                    protocol = spec.protocol.as_str(),
                    port,
                    name = %spec.name,
                    "插件端点已监听"
                ),
                Err(e) => {
                    let msg = format!("端点 {} 监听失败：{e:#}", spec.name);
                    if spec.required {
                        anyhow::bail!("{msg}（required=true，激活失败）");
                    }
                    tracing::warn!(plugin = plugin_id, "{msg}（optional，跳过）");
                }
            }
        }

        // ③ 服务：manifest.provides + registration.services 合并去重
        for decl in super::services::merge_decls(&provides, &extra) {
            let invoker: Arc<dyn ServiceInvoker> = Arc::new(EntryInvoker {
                runner: runner.clone(),
            });
            if let Err(e) =
                self.ctx
                    .services
                    .register_plugin_service(plugin_id, &decl, Some(invoker))
            {
                tracing::warn!(plugin = plugin_id, service = %decl.name, "{e:#}");
            }
        }

        Ok(())
    }

    /// 回收一个条目的全部注册（订阅/端点/服务/定时器/mDNS/入站队列）。
    pub(super) fn release_entry(&self, entry: &PluginEntry) {
        let id = entry.manifest.id.clone();
        self.ctx.bus.unsubscribe_all(&id);
        self.ctx.services.unregister_owner(&id);
        self.ctx.net.clear_plugin(&id);
        self.ctx.timers.clear_plugin(&id);
        self.ctx.mdns.clear_plugin(&id);
        self.inbox.remove(&id);
    }

    /// 卸载：移除条目并清空插件全部注册与命名空间（注册-回收）。
    pub fn unload(&self, plugin_id: &str) -> bool {
        let removed = {
            let mut entries = self.entries.lock().expect("entries lock poisoned");
            entries.remove(plugin_id)
        };
        match removed {
            Some(e) => {
                self.release_entry(&e);
                self.ctx.kv.clear_plugin(plugin_id);
                self.ctx.config.clear(plugin_id);
                tracing::info!(plugin = plugin_id, "插件卸载（注册与命名空间已回收）");
                true
            }
            None => false,
        }
    }

    /// 取运行期束（不做 guest 调用；锁表时间 O(log n)）。
    pub(super) fn runner(&self, plugin_id: &str) -> Result<Runner> {
        let entries = self.entries.lock().expect("entries lock poisoned");
        let e = entries
            .get(plugin_id)
            .ok_or_else(|| anyhow::anyhow!("插件 {plugin_id} 未装载"))?;
        e.runner(plugin_id)
            .ok_or_else(|| anyhow::anyhow!("插件 {plugin_id} 无运行实例"))
    }

    /// 事件步进（同步；测试与管理动作路径）。
    ///
    /// `payload` 为 JSON/文本载荷；二进制载荷（socket 帧）走事件泵的 [`Envelope`] 通道。
    pub fn handle_event(
        &self,
        plugin_id: &str,
        topic: &str,
        payload: &str,
    ) -> Result<Vec<effects::EffectError>> {
        let runner = self.runner(plugin_id)?;
        dispatch_event(
            &runner,
            &self.exec_env(),
            Envelope {
                topic: topic.to_string(),
                correlation_id: None,
                payload: payload.as_bytes().to_vec(),
            },
        )
    }

    /// 管理动作（管理台按钮与语音工具共用）。
    pub fn handle_action(&self, plugin_id: &str, action_id: &str, payload: &str) -> Result<String> {
        let runner = self.runner(plugin_id)?;
        {
            let entries = self.entries.lock().expect("entries lock poisoned");
            let entry = entries
                .get(plugin_id)
                .ok_or_else(|| anyhow::anyhow!("插件 {plugin_id} 未装载"))?;
            if !entry.registration.actions.iter().any(|a| a.id == action_id) {
                anyhow::bail!("插件 {plugin_id} 未声明动作 {action_id}");
            }
        }
        let called = {
            let mut inst = runner
                .instance
                .lock()
                .map_err(|_| anyhow::anyhow!("插件实例锁中毒"))?;
            inst.handle_action(action_id, payload)
        };
        match called {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(plugin_msg)) => anyhow::bail!("动作 {action_id} 失败：{plugin_msg}"),
            Err(call_err) => {
                let kind = classify_trap(&call_err);
                Err(runner.mark_degraded(kind, call_err.to_string()))
            }
        }
    }

    /// 候选验证：装载进临时实例跑 self-test（不落表、不影响现有实例）。
    pub fn validate(
        &self,
        expected_id: &str,
        wasm: &[u8],
    ) -> Result<Vec<super::contract::TestResultDto>> {
        let mut candidate = self.build_entry(expected_id, wasm)?;
        let instance = candidate.instance.take().expect("build_entry 成功必有实例");
        let mut guard = instance
            .lock()
            .map_err(|_| anyhow::anyhow!("候选实例锁中毒"))?;
        guard.self_test()
    }

    /// 解析扩展点（对当前注册表快照求值）。
    pub fn resolve(&self, point: &str) -> Resolution {
        let (table, world) = self.views();
        table.resolve(point, &world)
    }

    /// 设置扩展点首选实现。
    pub fn set_extension_preference(&self, point: &str, plugin_id: Option<String>) {
        self.resolver
            .lock()
            .expect("resolver lock poisoned")
            .set_preference(point, plugin_id);
    }

    pub(super) fn views(&self) -> (ResolutionTable, FlatWorld) {
        let entries = self.entries.lock().expect("entries lock poisoned");
        let mut world = FlatWorld::default();
        for (id, e) in entries.iter() {
            let live = e.lifecycle == Lifecycle::Active && e.phase_snapshot() == Phase::Active;
            world.insert(
                id,
                live,
                e.manifest.implements.iter().map(String::as_str).collect(),
                e.manifest.official,
            );
        }
        let table = self
            .resolver
            .lock()
            .expect("resolver lock poisoned")
            .clone();
        (table, world)
    }

    /// kv 只读访问（调试/测试）
    pub fn kv(&self) -> &KvStore {
        &self.ctx.kv
    }

    /// 配置后端（管理端点投影/写入用）。
    pub fn config_store(&self) -> &Arc<dyn PluginConfigStore> {
        &self.ctx.config
    }

    /// 事件总线统计（`/api/plugins` 投影）。
    pub fn bus_stats(&self) -> serde_json::Value {
        self.ctx.bus.stats_json()
    }

    /// **测试用**：订阅指定 topic（验证 guest 发布面；宿主侧无生产消费方）。
    #[cfg(test)]
    pub(crate) fn subscribe_topic(
        &self,
        topic: &str,
    ) -> tokio::sync::mpsc::Receiver<super::event_bus::Envelope> {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        self.ctx.bus.subscribe("__test__", topic, None, tx);
        rx
    }

    /// 已注册服务名全集。
    pub fn services(&self) -> Vec<String> {
        self.ctx.services.names()
    }

    /// 某插件的端点投影。
    pub fn endpoints_of(&self, plugin_id: &str) -> Vec<serde_json::Value> {
        self.ctx.net.endpoints_of(plugin_id)
    }

    /// 宿主 → 插件的事件发布（`sys.*` / `io.*` / `host.*`）。
    pub fn publish(&self, env: Envelope) -> bool {
        self.ctx.bus.publish(&env)
    }

    /// **探测**未知组件的 manifest id（管理台上传 id 缺省时用）。
    pub fn probe_id(&self, wasm: &[u8]) -> Result<String> {
        let mut probe = LoadedPlugin::instantiate(
            &self.engine,
            wasm,
            "__probe__",
            self.limits,
            Arc::clone(&self.ctx),
        )
        .context("组件实例化失败（不是合法的插件组件或激活期陷阱）")?;
        probe.manifest().map(|m| m.id)
    }

    /// 已装载（Active）插件目录 `(id, 标题)`。
    pub fn observers(&self) -> Vec<(String, String)> {
        self.entries
            .lock()
            .expect("entries lock poisoned")
            .values()
            .filter(|e| e.lifecycle == Lifecycle::Active)
            .map(|e| (e.manifest.id.clone(), e.manifest.title.clone()))
            .collect()
    }

    /// 单个已注册插件的 JSON UI 描述只读投影（`GET /api/plugins/{id}/ui` 数据源）。
    pub fn plugin_ui_json(&self, plugin_id: &str) -> Option<serde_json::Value> {
        let entries = self.entries.lock().expect("entries lock poisoned");
        let e = entries.get(plugin_id)?;
        let phase = e.phase_snapshot();
        if e.lifecycle != Lifecycle::Active || phase != Phase::Active {
            return None;
        }
        Some(serde_json::json!({
            "id": plugin_id,
            "title": e.manifest.title,
            "lifecycle": e.lifecycle.as_str(),
            "phase": phase.as_str(),
            "ui": super::contract::ui_json_value(e.registration.ui_json.as_deref()),
        }))
    }

    /// 单步派发（**不执行**效果、不回灌）：调插件 → 权限门 → 返回待执行效果。
    pub fn try_dispatch(
        &self,
        plugin_id: &str,
        topic: &str,
        payload: &str,
    ) -> Result<Option<Vec<Effect>>> {
        let runner = self.runner(plugin_id)?;
        // try_dispatch 是测试/同步面：无关联键（socket 编排走泵路径）。
        let Some(wit_effects) = call_guest_event(&runner, topic, payload.as_bytes(), None)? else {
            return Ok(None);
        };
        let domain: Vec<Effect> = wit_effects.into_iter().map(Into::into).collect();
        match effects::enforce_permissions(&runner.permissions, domain) {
            Ok(allowed) => {
                runner.set_phase(Phase::Active);
                Ok((!allowed.is_empty()).then_some(allowed))
            }
            Err(e) => Err(runner.mark_degraded(TrapKind::Trap, e.to_string())),
        }
    }

    /// 异步执行一批已放行的效果（fire-and-forget，tokio 池）。
    pub fn spawn_deferred(&self, plugin_id: &str, effects: Vec<Effect>) {
        let kv = Arc::clone(&self.ctx.kv);
        let fetcher = Arc::clone(&self.fetcher);
        let id = plugin_id.to_string();
        tokio::task::spawn_blocking(move || {
            let (errs, _feedback) = effects::execute_effects(&id, &effects, &kv, &fetcher);
            for e in errs {
                tracing::warn!(plugin = %id, "观察者效果执行失败：{e}");
            }
        });
    }
}

// ============================================================
// 条目级执行原语（事件泵与公开 API 共用）
// ============================================================

/// 调 guest 的 `handle-event`：返回效果（业务性拒绝 → `None`），陷阱 → Err（已置 Degraded）。
fn call_guest_event(
    runner: &Runner,
    topic: &str,
    payload: &[u8],
    correlation_id: Option<String>,
) -> Result<Option<Vec<super::contract::xiaozhi::plugin::types::Effect>>> {
    let ev = super::contract::xiaozhi::plugin::types::Event {
        topic: topic.to_string(),
        // S3m 勘误：此前硬编码 None——socket 事件（io.tcp.data/closed）的
        // correlation-id = connection-id 是 guest 编排定位连接的唯一键，
        // 泵路径丢失会让所有回灌事件无法关联（文档承诺的关联键语义）。
        correlation_id,
        payload: payload.to_vec(),
    };
    let called = {
        let mut inst = runner
            .instance
            .lock()
            .map_err(|_| anyhow::anyhow!("插件实例锁中毒"))?;
        inst.handle_event(ev)
    };
    match called {
        Ok(Ok(wit_effects)) => Ok(Some(wit_effects)),
        Ok(Err(plugin_msg)) => {
            tracing::info!(plugin = %runner.plugin_id, topic, "{plugin_msg}");
            Ok(None)
        }
        Err(call_err) => {
            let kind = classify_trap(&call_err);
            Err(runner.mark_degraded(kind, call_err.to_string()))
        }
    }
}

/// 完整派发：调 guest → 权限门 → 执行效果 → http 回灌（深度受限）。
pub(super) fn dispatch_event(
    runner: &Runner,
    exec: &ExecEnv,
    env: Envelope,
) -> Result<Vec<effects::EffectError>> {
    let mut all_errs = Vec::new();
    let mut pending: std::collections::VecDeque<Envelope> = std::collections::VecDeque::new();
    pending.push_back(env);
    let mut feedback_count = 0usize;

    while let Some(ev) = pending.pop_front() {
        let Some(wit_effects) =
            call_guest_event(runner, &ev.topic, &ev.payload, ev.correlation_id.clone())?
        else {
            return Ok(all_errs);
        };
        let domain: Vec<Effect> = wit_effects.into_iter().map(Into::into).collect();
        let allowed = match effects::enforce_permissions(&runner.permissions, domain) {
            Ok(a) => a,
            Err(e) => return Err(runner.mark_degraded(TrapKind::Trap, e.to_string())),
        };
        runner.set_phase(Phase::Active);
        let (errs, feedback) =
            effects::execute_effects(&runner.plugin_id, &allowed, &exec.ctx.kv, &exec.fetcher);
        all_errs.extend(errs);
        if !feedback.is_empty() {
            if feedback_count >= MAX_HTTP_FEEDBACK_DEPTH {
                tracing::warn!(
                    plugin = %runner.plugin_id,
                    "http 回灌深度达上限 {MAX_HTTP_FEEDBACK_DEPTH}，停止回灌"
                );
                all_errs.push(effects::EffectError::Failed(anyhow::anyhow!(
                    "http 回灌深度达上限 {MAX_HTTP_FEEDBACK_DEPTH}"
                )));
            } else {
                feedback_count += 1;
                pending.push_back(Envelope {
                    topic: "http.responses".to_string(),
                    correlation_id: None,
                    payload: effects::feedback_batch_payload(&feedback).into_bytes(),
                });
            }
        }
    }
    Ok(all_errs)
}

/// 每插件事件泵：从有界队列取事件，在阻塞线程池执行完整派发。
fn spawn_pump(
    rt: tokio::runtime::Handle,
    runner: Runner,
    exec: ExecEnv,
    mut rx: mpsc::Receiver<Envelope>,
) {
    rt.spawn(async move {
        while let Some(env) = rx.recv().await {
            eprintln!(
                "[pump] dispatch topic={} corr={:?} len={}",
                env.topic,
                env.correlation_id,
                env.payload.len()
            );
            let runner = runner.clone();
            let exec = exec.clone();
            let topic = env.topic.clone();
            let pid = runner.plugin_id.clone();
            let joined =
                tokio::task::spawn_blocking(move || dispatch_event(&runner, &exec, env)).await;
            match joined {
                Ok(Ok(errs)) => {
                    for e in errs {
                        tracing::warn!(plugin = %pid, topic = %topic, "事件效果执行失败：{e}");
                    }
                }
                Ok(Err(e)) => {
                    tracing::warn!(plugin = %pid, topic = %topic, "事件派发失败：{e:#}")
                }
                Err(e) => tracing::warn!(plugin = %pid, topic = %topic, "事件派发任务异常：{e}"),
            }
        }
        tracing::debug!("插件事件泵退出");
    });
}

/// 服务调用入口：把 `call-service` 转发给目标插件实例。
struct EntryInvoker {
    runner: Runner,
}

impl ServiceInvoker for EntryInvoker {
    fn invoke(&self, service: &str, method: &str, payload: &[u8], _depth: u32) -> Result<Vec<u8>> {
        let called = {
            let mut inst = self
                .runner
                .instance
                .lock()
                .map_err(|_| anyhow::anyhow!("插件实例锁中毒"))?;
            inst.handle_service(service, method, payload)
        };
        match called {
            Ok(Ok(bytes)) => Ok(bytes),
            Ok(Err(msg)) => anyhow::bail!("服务 {service}.{method} 由插件侧拒绝：{msg}"),
            Err(call_err) => {
                let kind = classify_trap(&call_err);
                Err(self.runner.mark_degraded(kind, call_err.to_string()))
            }
        }
    }
}
