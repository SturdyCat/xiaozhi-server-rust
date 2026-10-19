//! EchoMuse 设备登记持久化：register 时 upsert 到
//! `<memory.db_path 父目录>/echomuse_devices.json`（与配置库同卷，默认 /data）。
//!
//! 落盘的是**静态元数据**（label/approved/版本/板型/首末见时间）；`online` 不落盘，
//! 由 HTTP 层实时与 hub 合并（hub key = 处于会话中的在线设备）。文件小（设备数个位数），
//! 每次变更整体重写；读写都在互斥锁内，register/approve 频率极低，不构成热点。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use super::DeviceRegister;

/// 登记文件名（位于数据目录，见 [`default_path`]）。
pub(crate) const FILE_NAME: &str = "echomuse_devices.json";

/// 一台设备的静态登记记录（落盘格式；`online`/`connected_secs` 由 hub 实时合并不落盘）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub device_id: String,
    /// 展示名；默认等于 device_id，approve 时可改。
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub approved: bool,
    /// register 上报的固件版本（每次接入刷新）。
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub board: Option<String>,
    #[serde(default)]
    pub base_os: Option<String>,
    /// register 上报的能力表（最新一次为准）。蓝牙代理管理面以此判定设备是否
    /// 声明了 `ble_connect`（协议要求双向声明才启用）。
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// 首次见到（unix 秒）。
    pub first_seen: u64,
    /// 最近一次接入（unix 秒，register 时刷新）。
    pub last_seen: u64,
    /// 每设备配置（config push 的部分更新，键为协议 camelCase 字段白名单子集；
    /// 设备接入时随 `config` 推送，管理面改设置时落盘）。
    #[serde(default)]
    pub settings: HashMap<String, serde_json::Value>,
}

impl DeviceRecord {
    /// 合并一次 register 上报：刷新版本/板型/能力/last_seen，保留 label/approved/first_seen。
    fn absorb(&mut self, reg: &DeviceRegister, now: u64) {
        self.version = reg.version.clone();
        self.board = reg.board.clone();
        self.base_os = reg.base_os.clone();
        self.capabilities = reg.capabilities.clone();
        self.last_seen = now;
    }

    /// HTTP 展示体（`online`/`connected_secs` 由调用方按 hub 实况填）。
    pub(crate) fn payload(&self, connected_secs: Option<u64>) -> serde_json::Value {
        serde_json::json!({
            "device_id": self.device_id,
            "label": self.label,
            "approved": self.approved,
            "version": self.version,
            "board": self.board,
            "base_os": self.base_os,
            "capabilities": self.capabilities,
            "first_seen": self.first_seen,
            "last_seen": self.last_seen,
            "online": connected_secs.is_some(),
            "connected_secs": connected_secs,
            "settings": self.settings,
        })
    }
}

/// 设备登记表：JSON 文件 + 内存镜像。
pub struct Registry {
    path: PathBuf,
    devices: Mutex<HashMap<String, DeviceRecord>>,
}

impl Registry {
    /// 从 `path` 装载；文件缺失或损坏按空表起步（损坏只告警，不阻塞服务启动）。
    pub fn load(path: PathBuf) -> Self {
        let mut devices = HashMap::new();
        if let Ok(text) = std::fs::read_to_string(&path) {
            match serde_json::from_str::<HashMap<String, DeviceRecord>>(&text) {
                Ok(map) => devices = map,
                Err(e) => {
                    tracing::warn!("[echomuse] 登记文件解析失败，按空表起步 {path:?}: {e}");
                }
            }
        }
        Registry {
            path,
            devices: Mutex::new(devices),
        }
    }

    /// register 时 upsert：新设备建记录（label 默认 device_id），老设备刷新元数据。
    pub fn upsert(&self, reg: &DeviceRegister, now: u64) {
        let mut guard = self.devices.lock().expect("echomuse registry 锁中毒");
        let record = guard
            .entry(reg.device_id.clone())
            .or_insert_with(|| DeviceRecord {
                device_id: reg.device_id.clone(),
                label: reg.device_id.clone(),
                approved: false,
                version: String::new(),
                board: None,
                base_os: None,
                capabilities: Vec::new(),
                first_seen: now,
                last_seen: now,
                settings: HashMap::new(),
            });
        record.absorb(reg, now);
        drop(guard);
        self.save();
    }

    /// 保存每设备配置（整体替换 body 白名单过滤后的键集；返回更新后记录，None=不存在）。
    pub fn set_settings(
        &self,
        device_id: &str,
        settings: HashMap<String, serde_json::Value>,
    ) -> Option<DeviceRecord> {
        let mut guard = self.devices.lock().expect("echomuse registry 锁中毒");
        let record = guard.get_mut(device_id)?;
        record.settings = settings;
        let snapshot = record.clone();
        drop(guard);
        self.save();
        Some(snapshot)
    }

    /// 认领设备：置 `approved=true`，`label` 非空则改名；返回更新后记录（None=不存在）。
    pub fn approve(&self, device_id: &str, label: Option<&str>) -> Option<DeviceRecord> {
        let mut guard = self.devices.lock().expect("echomuse registry 锁中毒");
        let record = guard.get_mut(device_id)?;
        record.approved = true;
        if let Some(l) = label {
            let l = l.trim();
            if !l.is_empty() {
                record.label = l.to_string();
            }
        }
        let snapshot = record.clone();
        drop(guard);
        self.save();
        Some(snapshot)
    }

    /// 删除一台设备的登记记录（管理面「删除记录」动作，清理 mock/联调设备等）。
    /// 只删静态登记：在线设备的会话不受影响，设备再次 register 会重新登记。
    /// 返回删除前是否存在（HTTP 层据此回 200/404）。
    pub fn remove(&self, device_id: &str) -> bool {
        let mut guard = self.devices.lock().expect("echomuse registry 锁中毒");
        let removed = guard.remove(device_id).is_some();
        drop(guard);
        if removed {
            self.save();
        }
        removed
    }

    /// 单台记录。
    pub fn get(&self, device_id: &str) -> Option<DeviceRecord> {
        self.devices
            .lock()
            .expect("echomuse registry 锁中毒")
            .get(device_id)
            .cloned()
    }

    /// 全表快照（按 device_id 排序，展示稳定）。
    pub fn list(&self) -> Vec<DeviceRecord> {
        let mut all: Vec<DeviceRecord> = self
            .devices
            .lock()
            .expect("echomuse registry 锁中毒")
            .values()
            .cloned()
            .collect();
        all.sort_by(|a, b| a.device_id.cmp(&b.device_id));
        all
    }

    /// 整体重写落盘（失败只告警：内存态仍然可用，下次变更再试）。
    fn save(&self) {
        let guard = self.devices.lock().expect("echomuse registry 锁中毒");
        let text = match serde_json::to_string_pretty(&*guard) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("[echomuse] 登记文件序列化失败: {e}");
                return;
            }
        };
        if let Err(e) = std::fs::write(&self.path, text) {
            tracing::warn!("[echomuse] 登记文件写入失败 {:?}: {e}", self.path);
        }
    }
}

/// 进程级单例（路径取自首次访问时的配置；`[echomuse]` RestartOnly，路径运行期不变）。
static REGISTRY: OnceLock<Registry> = OnceLock::new();

/// 全局登记表（HTTP 层入口；测试用 [`Registry::load`] 自建实例隔离）。
pub(crate) fn global(cfg: &crate::config::Config) -> &'static Registry {
    REGISTRY.get_or_init(|| Registry::load(default_path(cfg)))
}

/// 登记文件落点：`[memory].db_path` 的父目录（与配置库同卷），默认 `/data`。
pub(crate) fn default_path(cfg: &crate::config::Config) -> PathBuf {
    data_dir(cfg).join(FILE_NAME)
}

fn data_dir(cfg: &crate::config::Config) -> PathBuf {
    Path::new(&cfg.memory.db_path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("/data"))
}

/// 当前 unix 秒（first_seen/last_seen 用）。
pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
