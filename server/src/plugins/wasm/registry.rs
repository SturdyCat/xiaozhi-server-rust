//! `WasmHost`：WASM 插件平台的门面——装载/激活/phase/快照/事件分发/扩展点解析。
//!
//! 装载语义（对齐静态注册表与 NLU 构建铁律）：
//!
//! - **先建新、后替换**：热装载先完整走「实例化 → manifest 校验 → 权限解析 →
//!   register 校验」，全过才替换旧实例；失败保留旧实例并记 `Failed`；
//! - **phase 与 enabled 分离**：enabled 是配置意图（P2 接配置管线），phase 是运行事实；
//! - **陷阱 → Degraded**：调用期 fuel/epoch/内存/trap 不 panic、不卸载，置 Degraded
//!   并保留实例（下次调用重置预算重试）；连续失败由扩展点解析自动回退官方默认；
//! - **注册-回收**：unload 清 kv 命名空间与注册表条目（DSH「注册随上下文回收」）。
//!
//! v0 单锁串行（P1 按插件分锁 + 无状态池）。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

use super::contract::{validate_manifest, validate_registration, ManifestDto, RegistrationDto};
use super::effects::{
    self, Effect, HttpFetcher, LanSocketCommand, LanSocketQueue, Permissions, ReqwestFetcher,
};
use super::instance::LoadedPlugin;
use super::kv::KvStore;
use super::resolver::{FlatWorld, Resolution, ResolutionTable};
use super::sandbox::{build_engine, classify_trap, EpochTicker, SandboxLimits, TrapKind};

/// http-get 回灌深度上限（初始事件 + 最多 N 次 `http.responses` 批量续步）。
/// 防插件「回灌再发请求」自我循环；超限停止回灌、记错误，不判插件死刑。
const MAX_HTTP_FEEDBACK_DEPTH: usize = 4;

/// 插件运行 phase（enabled 是配置意图，由配置管线持有，不在这）
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

struct PluginEntry {
    manifest: ManifestDto,
    registration: RegistrationDto,
    permissions: Permissions,
    phase: Phase,
    instance: Option<LoadedPlugin>,
}

/// 宿主：引擎 + 注册表 + kv + 解析表 + http 执行器。
pub struct WasmHost {
    engine: wasmtime::Engine,
    limits: SandboxLimits,
    entries: Mutex<BTreeMap<String, PluginEntry>>,
    kv: Arc<KvStore>,
    resolver: Mutex<ResolutionTable>,
    fetcher: Arc<dyn HttpFetcher>,
    /// 受限高层 LAN 命令队列；native Tuya runtime 可按批 drain。
    lan_queue: Arc<LanSocketQueue>,
    /// epoch 递增线程句柄（Drop 停止；字段持有即活着）
    _ticker: EpochTicker,
}

impl WasmHost {
    pub fn new(limits: SandboxLimits) -> Result<Self> {
        Self::with_fetcher(limits, Arc::new(ReqwestFetcher::default()))
    }

    /// 测试/嵌入方注入 fetcher（脚本化假实现，零真实网络）。
    pub fn with_fetcher(limits: SandboxLimits, fetcher: Arc<dyn HttpFetcher>) -> Result<Self> {
        let engine = build_engine().context("构建 wasm 引擎失败")?;
        let ticker = EpochTicker::start(&engine);
        Ok(Self {
            engine,
            entries: Mutex::new(BTreeMap::new()),
            kv: Arc::new(KvStore::new()),
            resolver: Mutex::new(ResolutionTable::new()),
            fetcher,
            lan_queue: Arc::new(LanSocketQueue::default()),
            limits,
            _ticker: ticker,
        })
    }

    /// 装载并激活：全链路成功才落表（新实例替换旧实例）。
    /// `wasm` 为组件二进制；`expected_id` 为装载请求声明的插件 id（须与 manifest 一致）。
    pub fn load(&self, expected_id: &str, wasm: &[u8]) -> Result<()> {
        let entry = self.build_entry(expected_id, wasm)?;
        let mut entries = self.entries.lock().expect("entries lock poisoned");
        entries.insert(expected_id.to_string(), entry);
        tracing::info!(plugin = expected_id, "插件装载并激活");
        Ok(())
    }

    fn build_entry(&self, expected_id: &str, wasm: &[u8]) -> Result<PluginEntry> {
        let mut instance = LoadedPlugin::instantiate(
            &self.engine,
            wasm,
            expected_id,
            self.limits,
            Arc::clone(&self.kv),
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
            phase: Phase::Active,
            instance: Some(instance),
        })
    }

    /// 卸载：移除条目并清空 kv 命名空间（注册-回收）。
    pub fn unload(&self, plugin_id: &str) -> bool {
        let removed = {
            let mut entries = self.entries.lock().expect("entries lock poisoned");
            entries.remove(plugin_id).is_some()
        };
        if removed {
            self.kv.clear_plugin(plugin_id);
            tracing::info!(plugin = plugin_id, "插件卸载（kv 命名空间已回收）");
        }
        removed
    }

    /// 事件步进：调用插件 → 权限门 → 执行效果；http-get 结果以 `http.responses`
    /// 事件回灌（瀑布循环，深度上限 [`MAX_HTTP_FEEDBACK_DEPTH`]）。
    /// 返回聚合的执行期错误（非致命，供上报）。
    pub fn handle_event(
        &self,
        plugin_id: &str,
        event: &str,
        payload: &str,
    ) -> Result<Vec<effects::EffectError>> {
        let mut all_errs = Vec::new();
        // 反馈队列：初始事件优先，反馈事件 FIFO
        let mut pending: std::collections::VecDeque<(String, String)> =
            std::collections::VecDeque::new();
        pending.push_back((event.to_string(), payload.to_string()));
        let mut feedback_count = 0usize;

        while let Some((ev, pl)) = pending.pop_front() {
            // ① 调用插件（持 entries 锁；效果执行刻意移出锁外——HTTP 可达秒级）
            let called: Result<(
                Permissions,
                Vec<super::contract::xiaozhi::plugin::types::Effect>,
            )> = {
                let mut entries = self.entries.lock().expect("entries lock poisoned");
                let entry = entries
                    .get_mut(plugin_id)
                    .ok_or_else(|| anyhow::anyhow!("插件 {plugin_id} 未装载"))?;
                let phase = entry.phase.as_str();
                let Some(instance) = entry.instance.as_mut() else {
                    anyhow::bail!("插件 {plugin_id} 无运行实例（phase={phase}）");
                };
                match instance.handle_event(&ev, &pl) {
                    Ok(Ok(wit_effects)) => Ok((entry.permissions.clone(), wit_effects)),
                    // 插件自身返回 Err：业务性失败，契约允许，不降级
                    Ok(Err(plugin_msg)) => {
                        tracing::info!(plugin = plugin_id, event = %ev, "{plugin_msg}");
                        return Ok(all_errs);
                    }
                    Err(call_err) => {
                        let kind = classify_trap(&call_err);
                        return Err(self.mark_degraded(
                            &mut entries,
                            plugin_id,
                            kind,
                            call_err.to_string(),
                        ));
                    }
                }
            };
            let (permissions, wit_effects) = called?;
            let domain: Vec<Effect> = wit_effects.into_iter().map(Into::into).collect();
            let allowed = match effects::enforce_permissions(&permissions, domain) {
                Ok(allowed) => allowed,
                Err(e) => {
                    let mut entries = self.entries.lock().expect("entries lock poisoned");
                    return Err(self.mark_degraded(
                        &mut entries,
                        plugin_id,
                        TrapKind::Trap,
                        e.to_string(),
                    ));
                }
            };
            // 成功调用恢复 Active
            {
                let mut entries = self.entries.lock().expect("entries lock poisoned");
                if let Some(e) = entries.get_mut(plugin_id) {
                    e.phase = Phase::Active;
                }
            }
            // ② 执行效果（不持 entries 锁）+ ③ http 回灌：一步的全部请求合成一个
            //    `http.responses` 批量事件（深度按**步**计，见 effects::feedback_batch_payload）
            let (errs, feedback) = effects::execute_effects_with_lan_queue(
                plugin_id,
                &allowed,
                &self.kv,
                &self.fetcher,
                Some(&self.lan_queue),
            );
            all_errs.extend(errs);
            if !feedback.is_empty() {
                if feedback_count >= MAX_HTTP_FEEDBACK_DEPTH {
                    tracing::warn!(
                        plugin = plugin_id,
                        "http 回灌深度达上限 {MAX_HTTP_FEEDBACK_DEPTH}，停止回灌"
                    );
                    all_errs.push(effects::EffectError::Failed(anyhow::anyhow!(
                        "http 回灌深度达上限 {MAX_HTTP_FEEDBACK_DEPTH}"
                    )));
                } else {
                    feedback_count += 1;
                    pending.push_back((
                        "http.responses".to_string(),
                        effects::feedback_batch_payload(&feedback),
                    ));
                }
            }
        }
        Ok(all_errs)
    }

    /// 管理动作（管理台按钮与语音工具共用）。
    pub fn handle_action(&self, plugin_id: &str, action_id: &str, payload: &str) -> Result<String> {
        let mut entries = self.entries.lock().expect("entries lock poisoned");
        let result = {
            let entry = entries
                .get_mut(plugin_id)
                .ok_or_else(|| anyhow::anyhow!("插件 {plugin_id} 未装载"))?;
            // 动作声明白名单：未注册的动作直接拒绝（模型不能任意指定动作名）
            if !entry.registration.actions.iter().any(|a| a.id == action_id) {
                anyhow::bail!("插件 {plugin_id} 未声明动作 {action_id}");
            }
            let phase = entry.phase.as_str();
            let Some(instance) = entry.instance.as_mut() else {
                anyhow::bail!("插件 {plugin_id} 无运行实例（phase={phase}）");
            };
            instance.handle_action(action_id, payload)
        };
        match result {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(plugin_msg)) => anyhow::bail!("动作 {action_id} 失败：{plugin_msg}"),
            Err(call_err) => {
                let kind = classify_trap(&call_err);
                Err(self.mark_degraded(&mut *entries, plugin_id, kind, call_err.to_string()))
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
        let results = candidate
            .instance
            .as_mut()
            .expect("build_entry 成功必有实例")
            .self_test()?;
        Ok(results)
    }

    fn mark_degraded(
        &self,
        entries: &mut BTreeMap<String, PluginEntry>,
        plugin_id: &str,
        kind: TrapKind,
        detail: String,
    ) -> anyhow::Error {
        tracing::warn!(plugin = plugin_id, ?kind, "{detail}");
        let degraded = Phase::Degraded {
            kind,
            detail: detail.clone(),
        };
        if let Some(e) = entries.get_mut(plugin_id) {
            e.phase = degraded;
        }
        anyhow::anyhow!("插件 {plugin_id} 调用陷阱（{kind:?}）：{detail}")
    }

    /// 解析扩展点（对当前注册表快照求值）。
    pub fn resolve(&self, point: &str) -> Resolution {
        let (table, world) = self.views();
        table.resolve(point, &world)
    }

    /// 设置扩展点首选实现（配置写入路径的内存侧；P2 接配置管线持久化）。
    pub fn set_extension_preference(&self, point: &str, plugin_id: Option<String>) {
        self.resolver
            .lock()
            .expect("resolver lock poisoned")
            .set_preference(point, plugin_id);
    }

    fn views(&self) -> (ResolutionTable, FlatWorld) {
        let entries = self.entries.lock().expect("entries lock poisoned");
        let mut world = FlatWorld::default();
        for (id, e) in entries.iter() {
            world.insert(
                id,
                e.phase == Phase::Active,
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

    /// 管理台/`/api/plugins` 投影（P2 与静态注册表清单合并）。
    pub fn snapshot_json(&self) -> serde_json::Value {
        let entries = self.entries.lock().expect("entries lock poisoned");
        let plugins: Vec<serde_json::Value> = entries
            .iter()
            .map(|(id, e)| {
                serde_json::json!({
                    "id": id,
                    "version": e.manifest.version,
                    "api_version": e.manifest.api_version,
                    "title": e.manifest.title,
                    "description": e.manifest.description,
                    "official": e.manifest.official,
                    "implements": e.manifest.implements,
                    "permissions": e.manifest.permissions,
                    "phase": phase_json(&e.phase),
                    "registration": {
                        "menu": e.registration.menu,
                        "config_fields": e.registration.config_fields,
                        "actions": e.registration.actions,
                    },
                })
            })
            .collect();
        // 扩展点解析投影：对所有已声明的扩展点求值
        let mut points: Vec<String> = Vec::new();
        for e in entries.values() {
            for p in &e.manifest.implements {
                if !points.contains(p) {
                    points.push(p.clone());
                }
            }
        }
        drop(entries);
        let (table, world) = self.views();
        let mut ext_map = serde_json::Map::new();
        for p in &points {
            ext_map.insert(p.clone(), resolution_json(&table.resolve(p, &world)));
        }
        let extensions = serde_json::Value::Object(ext_map);
        serde_json::json!({ "plugins": plugins, "extensions": extensions })
    }

    /// kv 只读访问（调试/测试）
    pub fn kv(&self) -> &KvStore {
        &self.kv
    }

    /// 取出一批受限 LAN 命令，交给 native connector 执行；不会暴露队列内部锁。
    pub fn drain_lan_commands(&self, max: usize) -> Vec<LanSocketCommand> {
        self.lan_queue.drain(max)
    }

    /// 当前待执行 LAN 命令数，用于背压和观测。
    pub fn pending_lan_commands(&self) -> usize {
        self.lan_queue.len()
    }

    /// **探测**未知组件的 manifest id（管理台上传 id 缺省时用：文件名不可靠，id 在
    /// 组件里）。隔离实例化 + 调 manifest()，不落表、不采用；验证仍由 `validate` 全量做。
    pub fn probe_id(&self, wasm: &[u8]) -> Result<String> {
        let mut probe = LoadedPlugin::instantiate(
            &self.engine,
            wasm,
            "__probe__",
            self.limits,
            Arc::clone(&self.kv),
        )
        .context("组件实例化失败（不是合法的插件组件或激活期陷阱）")?;
        probe.manifest().map(|m| m.id)
    }

    /// 已装载插件目录 `(id, 标题)`（workflow designer 的观察者绑定校验/投影源；
    /// 未编入 feature 时 Engines 侧返回空表）。
    pub fn observers(&self) -> Vec<(String, String)> {
        self.entries
            .lock()
            .expect("entries lock poisoned")
            .values()
            .map(|e| (e.manifest.id.clone(), e.manifest.title.clone()))
            .collect()
    }

    /// 单步派发（**不执行**效果、不回灌）：调插件 → 权限门 → 返回待执行效果。
    /// 观察者挂钩路径用：会话线程拿效果后 [`Self::spawn_deferred`] 异步执行，
    /// wasm 调用本身的阻塞由调用方包 spawn_blocking。
    /// `Ok(None)` = 插件业务性拒绝（合法，不算错误）；`Err` = 陷阱（已置 Degraded）。
    pub fn try_dispatch(
        &self,
        plugin_id: &str,
        event: &str,
        payload: &str,
    ) -> Result<Option<Vec<Effect>>> {
        let mut entries = self.entries.lock().expect("entries lock poisoned");
        let called = {
            let entry = entries
                .get_mut(plugin_id)
                .ok_or_else(|| anyhow::anyhow!("插件 {plugin_id} 未装载"))?;
            let phase = entry.phase.as_str();
            let Some(instance) = entry.instance.as_mut() else {
                anyhow::bail!("插件 {plugin_id} 无运行实例（phase={phase}）");
            };
            match instance.handle_event(event, payload) {
                Ok(Ok(wit_effects)) => Ok((entry.permissions.clone(), wit_effects)),
                Ok(Err(plugin_msg)) => {
                    tracing::info!(plugin = plugin_id, event, "{plugin_msg}");
                    Ok((entry.permissions.clone(), Vec::new()))
                }
                Err(call_err) => Err(call_err),
            }
        };
        let (permissions, wit_effects) = match called {
            Ok(v) => v,
            Err(call_err) => {
                let kind = classify_trap(&call_err);
                return Err(self
                    .mark_degraded(&mut entries, plugin_id, kind, call_err.to_string())
                    .context("观察者派发失败"));
            }
        };
        let domain: Vec<Effect> = wit_effects.into_iter().map(Into::into).collect();
        match effects::enforce_permissions(&permissions, domain) {
            // 成功调用恢复 Active；空效果 → None（观察者无需动作）
            Ok(allowed) => {
                if let Some(e) = entries.get_mut(plugin_id) {
                    e.phase = Phase::Active;
                }
                Ok((!allowed.is_empty()).then_some(allowed))
            }
            Err(e) => Err(self
                .mark_degraded(&mut entries, plugin_id, TrapKind::Trap, e.to_string())
                .context("观察者派发权限拒绝")),
        }
    }

    /// 异步执行一批已放行的效果（fire-and-forget，tokio 池）。
    /// 观察者语义：效果即上报（log/kv/http webhook），**不回灌**——会话已继续，
    /// 没有"下一步"可言；http 响应体按设计丢弃（见设计文档 P2d 观察者边界）。
    pub fn spawn_deferred(&self, plugin_id: &str, effects: Vec<Effect>) {
        let kv = Arc::clone(&self.kv);
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

fn phase_json(p: &Phase) -> serde_json::Value {
    match p {
        Phase::Active => serde_json::json!({"state": "active"}),
        Phase::Failed(e) => serde_json::json!({"state": "failed", "error": e}),
        Phase::Degraded { kind, detail } => {
            serde_json::json!({"state": "degraded", "trap": format!("{kind:?}"), "detail": detail})
        }
    }
}

fn resolution_json(r: &Resolution) -> serde_json::Value {
    match r {
        Resolution::Resolved(id) => serde_json::json!({"state": "resolved", "implementation": id}),
        Resolution::Fallback {
            official,
            preferred,
            reason,
        } => serde_json::json!({
            "state": "fallback", "implementation": official, "preferred": preferred, "reason": reason,
        }),
        Resolution::Unresolved(reason) => {
            serde_json::json!({"state": "unresolved", "reason": reason})
        }
    }
}
