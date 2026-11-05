//! Tuya 设备目录独立 SQLite 库（`<data>/tuya/devices.db`，spawn_blocking 调用）。
//!
//! ## 为什么独立于主配置库（`data.db`）
//!
//! ① **故障隔离**——local_key 属敏感凭据，独立文件可单独收紧权限（0600，见
//! [`TuyaStore::open`]）；表损坏/重置不影响主配置库。② **写入特征独立**——在线轮询
//! 每 60s 更新 last_seen/online，写频高于 config 保存态，不污染 config 库 WAL。
//!
//! ## 表结构（与架构文档 §2.1 一致）
//!
//! - `devices`：云同步与局域网发现共写的设备目录。`ip`/`protocol_version` 以
//!   **局域网实测优先**（发现覆盖云同步值——云里的 ip 可能是设备上次的公网地址）。
//! - `sync_history`：每次云同步一条记录（时间/结果/数量），UI 状态条展示最近一次。
//!
//! DP 实时状态**不落库**（易变数据，走 [`crate::device::tuya::TuyaEngine`] 内存缓存，
//! 重启后由轮询重建）。
//!
//! 连接模式：单连接 + `Mutex` + 懒开库（首次操作才建目录/建表），与
//! [`crate::config::store::sqlite`] 同款；全部方法是**阻塞**的——调用方（engine）
//! 一律包 `spawn_blocking`。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::Connection;

/// 设备目录一行（`devices` 表投影；local_key 属敏感字段，**禁止打日志/进 API**）。
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceRow {
    pub device_id: String,
    pub local_key: String,
    pub name: String,
    pub model: String,
    pub product_id: String,
    pub ip: String,
    pub protocol_version: String,
    pub online: bool,
    pub last_seen_ms: Option<i64>,
    /// 本地别名（P1-4）：UI 展示优先于 name，不影响云端名称。
    pub alias: String,
    pub updated_at: i64,
}

/// 一次云同步的记录（`sync_history` 一行）。
#[derive(Debug, Clone, PartialEq)]
pub struct SyncRecord {
    pub started_at: i64,
    pub finished_at: i64,
    /// ok / partial / cloud_auth_failed / network_error / skipped
    pub result: String,
    pub device_count: i64,
    pub message: String,
}

/// 独立 Tuya 库句柄（`Arc` 共享；`Mutex<Option<Connection>>` 懒开库）。
pub struct TuyaStore {
    path: PathBuf,
    conn: Mutex<Option<Connection>>,
}

impl TuyaStore {
    /// 构造句柄：**不做任何 IO**（dev 模式/测试装配零副作用），首次操作才开库。
    pub fn new(data_dir: &Path) -> Self {
        TuyaStore {
            path: data_dir.join("devices.db"),
            conn: Mutex::new(None),
        }
    }

    /// 数据库文件落点（`GET /api/tuya/status` 展示"设备目录存哪了"）。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 打开（或复用）连接并执行 `f`：建父目录 + 建表 + 收紧文件权限（0600，仅 Unix）。
    /// 借用边界收在闭包里——调用方拿不到 `Connection` 引用出函数。
    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let mut guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("创建 Tuya 数据目录 {} 失败", parent.display()))?;
            }
            let conn = Connection::open(&self.path)
                .with_context(|| format!("打开 Tuya 设备库 {} 失败", self.path.display()))?;
            conn.busy_timeout(std::time::Duration::from_secs(5))?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS devices (
                    device_id TEXT PRIMARY KEY,
                    local_key TEXT NOT NULL,
                    name TEXT NOT NULL DEFAULT '',
                    model TEXT NOT NULL DEFAULT '',
                    product_id TEXT NOT NULL DEFAULT '',
                    ip TEXT NOT NULL DEFAULT '',
                    protocol_version TEXT NOT NULL DEFAULT '',
                    online INTEGER NOT NULL DEFAULT 0,
                    last_seen_ms INTEGER,
                    alias TEXT NOT NULL DEFAULT '',
                    updated_at INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE IF NOT EXISTS sync_history (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    started_at INTEGER NOT NULL,
                    finished_at INTEGER NOT NULL,
                    result TEXT NOT NULL,
                    device_count INTEGER NOT NULL DEFAULT 0,
                    message TEXT NOT NULL DEFAULT ''
                );",
            )?;
            // local_key 明文落盘（架构 §6 的取舍：独立文件已收窄风险面），权限收紧到 0600。
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
            }
            *guard = Some(conn);
        }
        f(guard.as_ref().expect("懒开库后连接必然存在"))
    }

    /// 云同步 upsert（覆盖 local_key/name/model/product_id/online/ip）；
    /// local_key 变化时调用方负责重建对应局域网会话。
    pub fn upsert_device(&self, row: &DeviceRow) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO devices (device_id, local_key, name, model, product_id, ip,
                                      protocol_version, online, last_seen_ms, alias, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(device_id) DO UPDATE SET
                    local_key = excluded.local_key,
                    name = excluded.name,
                    model = excluded.model,
                    product_id = excluded.product_id,
                    ip = CASE WHEN excluded.ip != '' THEN excluded.ip ELSE devices.ip END,
                    protocol_version = CASE WHEN excluded.protocol_version != ''
                                            THEN excluded.protocol_version ELSE devices.protocol_version END,
                    online = excluded.online,
                    alias = CASE WHEN devices.alias != '' THEN devices.alias ELSE excluded.alias END,
                    updated_at = excluded.updated_at",
                rusqlite::params![
                    row.device_id,
                    row.local_key,
                    row.name,
                    row.model,
                    row.product_id,
                    row.ip,
                    row.protocol_version,
                    row.online as i64,
                    row.last_seen_ms,
                    row.alias,
                    row.updated_at,
                ],
            )?;
            Ok(())
        })
    }

    /// 全表设备（按 device_id 排序；UI 列表与轮询 worker 的数据源）。
    pub fn list_devices(&self) -> Result<Vec<DeviceRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT device_id, local_key, name, model, product_id, ip, protocol_version,
                        online, last_seen_ms, alias, updated_at
                 FROM devices ORDER BY device_id",
            )?;
            let rows = stmt
                .query_map([], row_from)
                .context("查询 Tuya 设备列表失败")?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// 单台设备（控制/详情端点先取行再走局域网会话池）。
    pub fn get_device(&self, device_id: &str) -> Result<Option<DeviceRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT device_id, local_key, name, model, product_id, ip, protocol_version,
                        online, last_seen_ms, alias, updated_at
                 FROM devices WHERE device_id = ?1",
            )?;
            let mut rows = stmt.query_map([device_id], row_from)?;
            match rows.next() {
                Some(row) => Ok(Some(row?)),
                None => Ok(None),
            }
        })
    }

    /// 在线状态与 last_seen（轮询 worker / 控制回读共用；ip 有实测值时一并修正）。
    pub fn set_online(&self, device_id: &str, online: bool, ip: Option<&str>) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE devices SET online = ?2,
                        last_seen_ms = CASE WHEN ?2 = 1 THEN ?3 ELSE last_seen_ms END,
                        ip = COALESCE(NULLIF(?4, ''), ip)
                 WHERE device_id = ?1",
                rusqlite::params![device_id, online as i64, now_ms(), ip.unwrap_or("")],
            )?;
            Ok(())
        })
    }

    /// 本地别名（P1-4）；空串 = 清除别名。
    pub fn set_alias(&self, device_id: &str, alias: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE devices SET alias = ?2, updated_at = ?3 WHERE device_id = ?1",
                rusqlite::params![device_id, alias, now_ms()],
            )?;
            Ok(())
        })
    }

    /// 局域网发现合并：**已登记**设备才更新 ip/协议版本（实测覆盖云值），返回是否已知；
    /// 未登记设备不自动纳管（目录以云同步为准，调用方记日志）。
    pub fn merge_discovery(&self, device_id: &str, ip: &str, version: &str) -> Result<bool> {
        self.with_conn(|conn| {
            let n = conn.execute(
                "UPDATE devices SET ip = ?2,
                        protocol_version = CASE WHEN ?3 != '' THEN ?3 ELSE protocol_version END,
                        updated_at = ?4
                 WHERE device_id = ?1",
                rusqlite::params![device_id, ip, version, now_ms()],
            )?;
            Ok(n > 0)
        })
    }

    /// 追加一条同步记录（每次同步一条；不清理——增长速率 = 手动+每日一次，可忽略）。
    pub fn record_sync(&self, record: &SyncRecord) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sync_history (started_at, finished_at, result, device_count, message)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    record.started_at,
                    record.finished_at,
                    record.result,
                    record.device_count,
                    record.message,
                ],
            )?;
            Ok(())
        })
    }

    /// 最近一次同步记录（UI 状态条）。
    pub fn last_sync(&self) -> Result<Option<SyncRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT started_at, finished_at, result, device_count, message
                 FROM sync_history ORDER BY id DESC LIMIT 1",
            )?;
            let mut rows = stmt.query_map([], |r| {
                Ok(SyncRecord {
                    started_at: r.get(0)?,
                    finished_at: r.get(1)?,
                    result: r.get(2)?,
                    device_count: r.get(3)?,
                    message: r.get(4)?,
                })
            })?;
            match rows.next() {
                Some(row) => Ok(Some(row?)),
                None => Ok(None),
            }
        })
    }
}

fn row_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceRow> {
    Ok(DeviceRow {
        device_id: r.get(0)?,
        local_key: r.get(1)?,
        name: r.get(2)?,
        model: r.get(3)?,
        product_id: r.get(4)?,
        ip: r.get(5)?,
        protocol_version: r.get(6)?,
        online: r.get::<_, i64>(7)? != 0,
        last_seen_ms: r.get(8)?,
        alias: r.get(9)?,
        updated_at: r.get(10)?,
    })
}

/// 毫秒时间戳（store 与 engine 共用的唯一时间出口，测试可断言单调）。
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
