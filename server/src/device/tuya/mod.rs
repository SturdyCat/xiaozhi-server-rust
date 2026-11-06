//! Tuya IoT 设备集成（`[tuya]`）：云目录同步 + 纯局域网发现/控制 + 管理面 `/api/tuya/*`。
//!
//! ## 定位（与 echomuse 的同与不同）
//!
//! tuya 与 echomuse 同为"受管设备"，但**不接入语音会话的 Transport 翻译层**：
//! 涂鸦云只用于凭证与周期/手动同步设备目录（local_key、名称、模型、在线状态），
//! 日常的发现（UDP 6666/6667/7000）、状态读取与控制（TCP 6668）全部纯局域网——
//! 零云依赖、低延迟、断云可用。语音控制（NLU iot 域技能）经会话 plan 的
//! `tuya.control` 能力进入 [`voice`]（名称匹配 + 开关 DP 下发，infallible 话术）。
//!
//! ## 模块结构
//!
//! - [`config`]：`[tuya]` 配置段（全部热生效）；
//! - [`store`]：独立 SQLite `<data>/tuya/devices.db`（devices + sync_history，0600）；
//! - [`cloud`]：涂鸦云 OpenAPI 客户端（官方 v1.0 签名 + token 缓存 + 设备清单翻页
//!   + 调试代理 `proxy_request`；2026-11-03 真凭证联调全链路验证）；
//! - [`cloud_transport`]：云 HTTP 传输层（请求/响应形状 + reqwest 发送抽象）；
//! - [`sync`]：同步编排（拉清单 → upsert → sync_history；60s 最小间隔全局闸门）；
//! - [`lan`]：rustuya 会话池（懒式建连）+ 失败三归类 + DP 载荷解析；
//! - [`voice`]：NLU iot 域语音控制（名称匹配 / 开关 DP 识别 / 话术组装）；
//! - [`worker`]：后台任务（定时同步 + 60s 错峰在线轮询 + 5min 发现扫描）；
//! - [`api`]：axum 管理面（经管理鉴权，secret 永不回显）。
//!
//! ## 安全铁律
//!
//! - `access_secret` **只写不读**：任何 API 响应/日志都不含原值；
//! - `local_key` 只落独立库（0600），**禁止出现在日志里**；
//! - lan/cloud 日志只记命令类型、数量与错误文本，禁止输出原始加密报文与 sign 值。

pub mod api;
pub mod cloud;
pub mod cloud_transport;
pub mod config;
pub mod lan;
pub mod store;
pub mod sync;
pub mod voice;
#[cfg(feature = "wasm-plugins")]
mod wasm;
pub mod worker;

#[cfg(test)]
mod api_tests;
#[cfg(test)]
mod cloud_tests;
#[cfg(test)]
mod lan_tests;
#[cfg(test)]
mod sync_tests;
#[cfg(test)]
mod voice_tests;
#[cfg(all(test, feature = "wasm-plugins"))]
mod wasm_tests;

pub use config::TuyaConfig;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lan::{ControlFailure, LanPool, LanSink};
use store::{now_ms, DeviceRow, TuyaStore};

/// DP 实时状态内存缓存（per-device `dp_id → value`）。易变数据不落库，
/// 重启后由轮询/控制回读重建（架构 §2.1）。
pub type DpCache = Mutex<HashMap<String, HashMap<String, serde_json::Value>>>;

/// LAN 扫描发现的**未登记**设备（内存快照，仅状态条展示；不纳管、不落库）。
///
/// 目录真源是云同步：未登记设备等云目录把它带回来（云同步会写入 local_key），
/// 这里只把"局域网里有这么一台设备"透出到 `GET /api/tuya/status` 的
/// `lan_unregistered`，让管理台不再只有日志可见。
#[derive(Debug, Clone, PartialEq)]
pub struct UnregisteredDevice {
    pub device_id: String,
    pub ip: String,
    pub discovered_at_ms: i64,
}

/// 快照容量上限：家庭规模足够，防异常网络下无限增长。
const UNREGISTERED_MAX: usize = 32;

/// 在线轮询的每设备记账（错峰相位 + 连续失败计数）。
struct PollState {
    /// 下次应轮询的时刻（首见设备按 device_id 哈希散列到 60s 内，避免整点齐发）。
    next_due: Instant,
    fails: u32,
}

/// 同步闸门状态：进行中标记 + 最近一次尝试时刻（60s 最小间隔，手动/定时共用）。
struct SyncState {
    syncing: bool,
    last_start: Option<Instant>,
}

/// TuyaEngine：store/cloud/lan 状态共享 + DP 内存缓存 + 各类闸门。
///
/// 装配进 [`crate::runtime::Engines`]（`engines.tuya`），管理面 handler 与 worker 共用。
/// 构造**零 IO**（store 懒开库、cloud 懒建连、lan 懒建会话）——dev 模式装配同样安全。
pub struct TuyaEngine {
    pub(crate) store: Arc<TuyaStore>,
    /// 云客户端（token 缓存在内；跨 await 持锁 → tokio Mutex）。
    pub(crate) cloud: tokio::sync::Mutex<cloud::CloudClient>,
    pub(crate) lan: LanPool,
    pub(crate) dp_cache: Arc<DpCache>,
    /// "控制成功过"的 dp 集合（writable 判定：控制成功即 true，不做能力探测）。
    pub(crate) writable: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    /// 单设备在途控制互斥（同设备同时只允许 1 个控制请求，架构 §4）。
    inflight: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    poll: Mutex<HashMap<String, PollState>>,
    sync_state: Mutex<SyncState>,
    /// LAN 发现的未登记设备快照（状态条展示用；容量见 [`UNREGISTERED_MAX`]）。
    unregistered: Mutex<Vec<UnregisteredDevice>>,
    /// 已生效的凭证指纹（变化 → 丢弃 token 缓存；勿打日志，含 secret）。
    cred_fp: Mutex<String>,
}

/// 在线轮询周期（worker 每 tick 检查 next_due）。
const POLL_INTERVAL: Duration = Duration::from_secs(60);
/// 发现扫描周期（启动 + 每 5 分钟，架构 §4）。
pub(crate) const SCAN_INTERVAL: Duration = Duration::from_secs(300);

/// 真正的装配函数：dp_cache 在 store/sink/lan/engine 之间共享，只能在一处组装。
fn assemble(data_dir: &Path) -> TuyaEngine {
    let store = Arc::new(TuyaStore::new(data_dir));
    let dp_cache: Arc<DpCache> = Arc::new(DpCache::new(HashMap::new()));
    let sink = LanSink {
        store: store.clone(),
        dp_cache: dp_cache.clone(),
    };
    TuyaEngine {
        store,
        cloud: tokio::sync::Mutex::new(cloud::CloudClient::new()),
        lan: LanPool::new(sink),
        dp_cache,
        writable: Arc::new(Mutex::new(HashMap::new())),
        inflight: Mutex::new(HashMap::new()),
        poll: Mutex::new(HashMap::new()),
        sync_state: Mutex::new(SyncState {
            syncing: false,
            last_start: None,
        }),
        unregistered: Mutex::new(Vec::new()),
        cred_fp: Mutex::new(String::new()),
    }
}

impl TuyaEngine {
    /// **唯一**的构造入口（见 [`assemble`]）：零 IO，`Engines::assemble` 与测试共用。
    pub fn new(data_dir: &Path) -> Self {
        assemble(data_dir)
    }

    // ============================================================
    // 目录读取（阻塞 SQLite → spawn_blocking）
    // ============================================================

    /// 单台设备（控制/详情端点先取行再走会话池）。
    pub async fn get_device(&self, device_id: &str) -> Result<Option<DeviceRow>, String> {
        let store = self.store.clone();
        let id = device_id.to_string();
        tokio::task::spawn_blocking(move || store.get_device(&id).map_err(|e| e.to_string()))
            .await
            .map_err(|e| format!("读库任务失败: {e}"))?
    }

    /// 全量设备目录（按 device_id 排序）。
    pub async fn list_devices(&self) -> Result<Vec<DeviceRow>, String> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.list_devices().map_err(|e| e.to_string()))
            .await
            .map_err(|e| format!("读库任务失败: {e}"))?
    }

    /// 本地别名（P1-4）。
    pub async fn set_alias(&self, device_id: &str, alias: &str) -> Result<(), String> {
        let store = self.store.clone();
        let id = device_id.to_string();
        let alias = alias.trim().to_string();
        tokio::task::spawn_blocking(move || store.set_alias(&id, &alias).map_err(|e| e.to_string()))
            .await
            .map_err(|e| format!("写库任务失败: {e}"))?
    }

    /// 最近一次同步记录（UI 状态条）。
    pub async fn last_sync(&self) -> Result<Option<store::SyncRecord>, String> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.last_sync().map_err(|e| e.to_string()))
            .await
            .map_err(|e| format!("读库任务失败: {e}"))?
    }

    // ============================================================
    // 控制（per-device 互斥 → 会话池 → set_value → 回读 → 缓存）
    // ============================================================

    fn inflight_gate(&self, device_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.inflight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(device_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// 下发一个 DP 值并回读状态。成功返回全量 DP 快照（写缓存）；
    /// 失败返回三归类（offline/timeout/unsupported）+ 建议动作。
    pub async fn set_dp(
        &self,
        device_id: &str,
        dp: &str,
        value: serde_json::Value,
    ) -> Result<HashMap<String, serde_json::Value>, ControlFailure> {
        let row = self
            .get_device(device_id)
            .await
            .map_err(|e| ControlFailure::offline(format!("读取设备目录失败: {e}")))?
            .ok_or_else(|| ControlFailure::offline("设备不在目录中（请先「立即同步」）".into()))?;
        // online 预检（架构 §4）：离线设备直接归类，不发网络请求
        if !row.online {
            return Err(ControlFailure::offline(
                "设备离线（等待轮询确认上线）".into(),
            ));
        }
        // 单设备互斥：同设备同时只允许 1 个在途控制
        let gate = self.inflight_gate(device_id);
        let _guard = gate.lock().await;
        let device = self.lan.get_or_build(&row).await?;
        self.lan.ensure_listener(device_id);
        let dp_id = dp.to_string();
        lan::with_call_timeout(device.set_value(dp_id.clone(), value.clone())).await?;
        // 控制成功 → 回读全量状态更新缓存（回读失败不影响控制成功的结论）
        let mut dps = match lan::with_call_timeout(device.status()).await {
            Ok(Some(text)) => lan::parse_dps(&text),
            _ => HashMap::from([(dp_id.clone(), value.clone())]),
        };
        if dps.is_empty() {
            dps.insert(dp_id.clone(), value);
        }
        lan::mark_online(&self.store, device_id, true, None);
        self.dp_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(device_id.to_string(), dps.clone());
        self.writable
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(device_id.to_string())
            .or_default()
            .insert(dp_id);
        Ok(dps)
    }

    /// 设备 DP 快照（详情端点）：缓存里没有 → None（等轮询/控制回读填充）。
    pub fn cached_dps(&self, device_id: &str) -> Option<HashMap<String, serde_json::Value>> {
        self.dp_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(device_id)
            .cloned()
    }

    /// DP 是否控制成功过（writable 判定）。
    pub fn dp_writable(&self, device_id: &str, dp: &str) -> bool {
        self.writable
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(device_id)
            .is_some_and(|set| set.contains(dp))
    }

    // ============================================================
    // 在线轮询与发现（worker 调用；api 不触碰）
    // ============================================================

    /// 轮询单台：status() 成功 → online=1 + DP 缓存；连续 2 次失败 → online=0。
    /// 返回本轮是否成功。
    pub(crate) async fn poll_device(&self, row: &DeviceRow) -> bool {
        let device = match self.lan.get_or_build(row).await {
            Ok(d) => d,
            Err(failure) => {
                tracing::debug!(
                    "Tuya 设备 {} 轮询建连失败（{}）",
                    row.device_id,
                    failure.kind
                );
                self.note_poll_failure(row);
                return false;
            }
        };
        self.lan.ensure_listener(&row.device_id);
        let ok = match lan::with_call_timeout(device.status()).await {
            Ok(Some(text)) => {
                let dps = lan::parse_dps(&text);
                if !dps.is_empty() {
                    self.dp_cache
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(row.device_id.clone(), dps);
                }
                true
            }
            Ok(None) => true,
            Err(_) => false,
        };
        if ok {
            lan::mark_online(&self.store, &row.device_id, true, Some(&row.ip));
            self.reset_poll(&row.device_id);
        } else {
            self.note_poll_failure(row);
        }
        ok
    }

    /// 到期设备错峰轮询一轮，返回在线台数。
    pub(crate) async fn poll_due(&self) -> usize {
        let devices = match self.list_devices().await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("Tuya 在线轮询读目录失败: {e}");
                return 0;
            }
        };
        let now = Instant::now();
        let due: Vec<DeviceRow> = {
            let mut book = self.poll.lock().unwrap_or_else(|e| e.into_inner());
            devices
                .into_iter()
                .filter(|row| {
                    let entry = book
                        .entry(row.device_id.clone())
                        .or_insert_with(|| PollState {
                            // 错峰相位：device_id 哈希散列到 0..60s，避免整点齐发
                            next_due: now + Duration::from_secs(hash_offset(&row.device_id)),
                            fails: 0,
                        });
                    entry.next_due <= now
                })
                .collect()
        };
        let results =
            futures_util::future::join_all(due.iter().map(|row| self.poll_device(row))).await;
        results.iter().filter(|ok| **ok).count()
    }

    fn reset_poll(&self, device_id: &str) {
        if let Some(entry) = self
            .poll
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(device_id)
        {
            entry.fails = 0;
            entry.next_due = Instant::now() + POLL_INTERVAL;
        }
    }

    /// 轮询失败：连续 2 次才落 online=0（单次网络抖动不翻转状态）。
    fn note_poll_failure(&self, row: &DeviceRow) {
        let mut book = self.poll.lock().unwrap_or_else(|e| e.into_inner());
        let entry = book
            .entry(row.device_id.clone())
            .or_insert_with(|| PollState {
                next_due: Instant::now() + POLL_INTERVAL,
                fails: 0,
            });
        entry.fails += 1;
        entry.next_due = Instant::now() + POLL_INTERVAL;
        if entry.fails >= 2 && row.online {
            drop(book);
            tracing::info!("Tuya 设备 {} 连续 2 次探测失败，标记离线", row.device_id);
            lan::mark_online(&self.store, &row.device_id, false, None);
        }
    }

    /// 局域网发现一轮（Scanner UDP 6666/6667/7000）：已登记设备合并 ip/协议版本
    /// （实测覆盖云值），未登记设备只记日志不纳管。返回合并台数。
    pub(crate) async fn scan_and_merge(&self) -> usize {
        let scan = tokio::time::timeout(Duration::from_secs(20), rustuya::Scanner::scan()).await;
        let found = match scan {
            Ok(Ok(found)) => found,
            Ok(Err(e)) => {
                tracing::warn!("Tuya 局域网发现失败: {e}");
                return 0;
            }
            Err(_) => {
                tracing::warn!("Tuya 局域网发现超时（20s）");
                return 0;
            }
        };
        let mut merged = 0;
        for d in found {
            let version = d.version.map(|v| v.to_string()).unwrap_or_default();
            let store = self.store.clone();
            let id = d.id.clone();
            let ip = d.ip.clone();
            let merged_ok = tokio::task::spawn_blocking(move || {
                store
                    .merge_discovery(&id, &ip, &version)
                    .map_err(|e| e.to_string())
            })
            .await;
            match merged_ok {
                Ok(Ok(true)) => {
                    merged += 1;
                    self.lan.drop_if_ip_differs(&d.id, &d.ip);
                }
                Ok(Ok(false)) => {
                    tracing::info!(
                        "Tuya 发现未登记设备 {}（不自动纳管，目录以云同步为准）",
                        d.id
                    );
                    self.note_unregistered(&d.id, &d.ip);
                }
                Ok(Err(e)) => tracing::warn!("Tuya 发现结果合并失败（{}）: {e}", d.id),
                Err(e) => tracing::warn!("Tuya 发现合并任务失败: {e}"),
            }
        }
        merged
    }

    /// 记录一台 LAN 发现的未登记设备（upsert by device_id；容量到顶丢最旧）。
    fn note_unregistered(&self, device_id: &str, ip: &str) {
        let now = now_ms();
        let mut list = self.unregistered.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = list.iter_mut().find(|u| u.device_id == device_id) {
            entry.ip = ip.to_string();
            entry.discovered_at_ms = now;
            return;
        }
        list.push(UnregisteredDevice {
            device_id: device_id.to_string(),
            ip: ip.to_string(),
            discovered_at_ms: now,
        });
        if list.len() > UNREGISTERED_MAX {
            // 丢最旧（发现时间最小）
            list.sort_by(|a, b| b.discovered_at_ms.cmp(&a.discovered_at_ms));
            list.truncate(UNREGISTERED_MAX);
        }
    }

    /// 未登记设备快照（status 状态条）：按发现时间倒序，上限 [`UNREGISTERED_MAX`]。
    pub(crate) fn unregistered_snapshot(&self) -> Vec<UnregisteredDevice> {
        let mut list = self
            .unregistered
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        list.sort_by(|a, b| b.discovered_at_ms.cmp(&a.discovered_at_ms));
        list.truncate(UNREGISTERED_MAX);
        list
    }

    // ============================================================
    // 凭证热生效
    // ============================================================
    /// 凭证指纹比对（worker 每 tick / 凭证保存后调用）：变化 → 丢弃 token 缓存。
    ///
    /// 竞态修复：指纹提交放在 `try_lock` **成功之后**。若先提交指纹再拿锁失败，
    /// 本次 token 缓存不会失效，且后续 tick 因 guard 已等于新指纹永远不再触发
    /// （旧 token 会一直复用到过期）。拿不到锁就不动指纹，下个 tick 自然重试。
    pub fn check_credentials(&self, cfg: &TuyaConfig) {
        let fp = cfg.credentials_fingerprint();
        let mut guard = self.cred_fp.lock().unwrap_or_else(|e| e.into_inner());
        if *guard != fp {
            if let Ok(mut client) = self.cloud.try_lock() {
                if !guard.is_empty() {
                    tracing::info!("[tuya] 凭证已变更，丢弃云 token 缓存");
                }
                client.invalidate();
                *guard = fp;
            }
            // try_lock 失败：指纹保持旧值，下个 tick 重试（token 缓存不受影响）
        }
    }

    /// 凭证被管理面显式改写后立即丢弃 token 缓存。
    pub fn invalidate_credentials(&self) {
        *self.cred_fp.lock().unwrap_or_else(|e| e.into_inner()) = String::new();
        if let Ok(mut client) = self.cloud.try_lock() {
            client.invalidate();
        }
    }
    /// 是否有同步正在进行（API 状态条）。
    pub fn syncing(&self) -> bool {
        self.sync_state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .syncing
    }

    /// 设备目录库落点（状态条展示）。
    pub fn store_path(&self) -> &Path {
        self.store.path()
    }
}

/// device_id → 0..60 的错峰相位（FNV-1a 取模；纯哈希，非安全用途）。
fn hash_offset(device_id: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in device_id.bytes() {
        h ^= byte as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h % 60
}

/// 同步记录快照（`GET /api/tuya/status` 的 last_sync 投影；api_tests 直驱）。
#[derive(Debug, Clone, PartialEq)]
pub struct SyncRecordView {
    pub finished_at: i64,
    /// 距完成的毫秒数（服务端算好给 UI，客户端不依赖本机时钟）。
    pub age_ms: i64,
    pub result: String,
    pub device_count: i64,
    pub message: String,
}

impl SyncRecordView {
    pub(crate) fn from_record(record: &store::SyncRecord) -> Self {
        SyncRecordView {
            finished_at: record.finished_at,
            age_ms: (now_ms() - record.finished_at).max(0),
            result: record.result.clone(),
            device_count: record.device_count,
            message: record.message.clone(),
        }
    }
}
