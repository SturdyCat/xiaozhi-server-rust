//! 云同步编排：拉清单 → diff upsert → sync_history，加 60s 最小间隔全局闸门。
//!
//! 结果四归类（写 `sync_history.result`，UI 状态条展示最近一次）：
//! - `ok`：全量拉取并落库成功；
//! - `partial`：部分页失败（已取到的设备照常落库——云客户端中途失败不丢数据）；
//! - `cloud_auth_failed`：凭证/鉴权问题（提示检查凭证）；
//! - `network_error`：网络/超时/云端业务错误。
//!
//! **闸门**：手动（`POST /api/tuya/sync`）与定时（worker）共用同一个
//! [`MIN_SYNC_INTERVAL`]——防连点 + 涂鸦云免费额度保护（架构 §3）。

use std::time::{Duration, Instant};

use super::cloud::{CloudDevice, CloudError};
use super::store::{now_ms, DeviceRow, SyncRecord};
use super::{TuyaConfig, TuyaEngine};

/// 全局最小同步间隔（秒）：手动与定时共用。
pub(crate) const MIN_SYNC_INTERVAL: Duration = Duration::from_secs(60);

/// 一次同步的最终投影（结果字符串 + 人话消息；API/worker 共用）。
#[derive(Debug, Clone, PartialEq)]
pub struct SyncOutcome {
    pub result: String,
    pub message: String,
}

impl SyncOutcome {
    fn new(result: &str, message: impl Into<String>) -> Self {
        SyncOutcome {
            result: result.to_string(),
            message: message.into(),
        }
    }
}

impl TuyaEngine {
    /// 手动/定时共用的同步入口：闸门（busy + 60s 间隔）通过才真正执行。
    ///
    /// 被闸门拦下时返回 `result = "skipped"`（不算失败，UI 提示稍后再试）。
    pub async fn sync_now(&self, cfg: &TuyaConfig) -> SyncOutcome {
        {
            let mut state = self.sync_state.lock().unwrap_or_else(|e| e.into_inner());
            if state.syncing {
                return SyncOutcome::new("skipped", "已有一次同步在进行中");
            }
            if let Some(last) = state.last_start {
                if last.elapsed() < MIN_SYNC_INTERVAL {
                    return SyncOutcome::new(
                        "skipped",
                        format!(
                            "距上次同步不足 {}s，请稍后再试",
                            MIN_SYNC_INTERVAL.as_secs()
                        ),
                    );
                }
            }
            state.syncing = true;
            state.last_start = Some(Instant::now());
        }
        let outcome = self.execute_sync(cfg).await;
        self.sync_state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .syncing = false;
        outcome
    }

    /// 定时同步判定（worker 每 tick 调用）：interval=0 关闭；到期才触发。
    /// `last_start` 未设置（进程刚启动）→ 首个 tick 即同步一次（启动目录刷新）。
    pub fn auto_sync_due(&self, interval_secs: u64) -> bool {
        if interval_secs == 0 {
            return false;
        }
        let state = self.sync_state.lock().unwrap_or_else(|e| e.into_inner());
        if state.syncing {
            return false;
        }
        match state.last_start {
            None => true,
            Some(last) => last.elapsed() >= Duration::from_secs(interval_secs),
        }
    }

    /// 真正的同步执行（闸门已过）：拉清单 → 落库 → 写 sync_history。
    pub(crate) async fn execute_sync(&self, cfg: &TuyaConfig) -> SyncOutcome {
        let started_at = now_ms();
        if !cfg.credentials_ready() {
            return self
                .finish_sync(
                    started_at,
                    "cloud_auth_failed",
                    0,
                    "涂鸦云凭证未配置（请先在管理台录入）",
                )
                .await;
        }
        self.check_credentials(cfg);
        // 持云客户端锁执行整个拉取（token 缓存复用 + 翻页串行）
        let (devices, error) = {
            let mut client = self.cloud.lock().await;
            client
                .authorized_devices_all(&cfg.region, &cfg.access_id, &cfg.access_secret)
                .await
        };
        if let Some(err) = error {
            let (result, message) = classify_cloud_error(&err, devices.len());
            if !devices.is_empty() {
                // partial：已取到的部分先落库，再报错
                let (upserted, failed) = self.upsert_all(&devices, &cfg.access_id).await;
                let count = upserted;
                return self
                    .finish_sync(
                        started_at,
                        result,
                        count as i64,
                        format!("{message}；已落库 {upserted} 台（{failed} 台写库失败）"),
                    )
                    .await;
            }
            return self.finish_sync(started_at, result, 0, message).await;
        }
        let (upserted, failed) = self.upsert_all(&devices, &cfg.access_id).await;
        if failed > 0 && upserted == 0 {
            return self
                .finish_sync(
                    started_at,
                    "network_error",
                    0,
                    format!("设备清单落库全部失败（{failed} 台）"),
                )
                .await;
        }
        let mut message = format!("同步 {upserted} 台设备");
        if failed > 0 {
            message.push_str(&format!("（{failed} 台写库失败）"));
        }
        self.finish_sync(started_at, "ok", upserted as i64, message)
            .await
    }

    /// 设备清单落库（spawn_blocking）+ local_key 变化设备的会话重建标记。
    /// 返回 (成功台数, 失败台数)。
    async fn upsert_all(&self, devices: &[CloudDevice], _access_id: &str) -> (usize, usize) {
        // 先取旧表（判断 local_key 变化 → 重建对应会话）
        let old_keys: std::collections::HashMap<String, String> = match self.list_devices().await {
            Ok(rows) => rows
                .into_iter()
                .map(|r| (r.device_id, r.local_key))
                .collect(),
            Err(e) => {
                tracing::warn!("Tuya 同步前读旧目录失败（按全新处理）: {e}");
                std::collections::HashMap::new()
            }
        };
        let now = now_ms();
        let rows: Vec<DeviceRow> = devices
            .iter()
            .map(|d| DeviceRow {
                device_id: d.device_id.clone(),
                local_key: d.local_key.clone(),
                name: d.name.clone(),
                model: d.model.clone(),
                product_id: d.product_id.clone(),
                ip: d.ip.clone(),
                protocol_version: String::new(), // 协议版本以局域网实测为准（发现覆盖）
                online: d.online,
                last_seen_ms: None,
                alias: d.name.clone(), // 首次同步用云名称垫底；本地 alias 优先（store upsert 保护）
                updated_at: now,
            })
            .collect();
        for row in &rows {
            if let Some(old_key) = old_keys.get(&row.device_id) {
                if old_key != &row.local_key {
                    tracing::info!(
                        "Tuya 设备 {} local_key 已变更，丢弃旧局域网会话",
                        row.device_id
                    );
                    self.lan.drop_session(&row.device_id);
                }
            }
        }
        let store = self.store.clone();
        let task_rows = rows.clone();
        match tokio::task::spawn_blocking(move || {
            let mut ok = 0;
            let mut failed = 0;
            for row in &task_rows {
                match store.upsert_device(row) {
                    Ok(()) => ok += 1,
                    Err(e) => {
                        tracing::warn!("Tuya 设备 {} 落库失败: {e}", row.device_id);
                        failed += 1;
                    }
                }
            }
            (ok, failed)
        })
        .await
        {
            Ok((ok, failed)) => (ok, failed),
            Err(e) => {
                tracing::warn!("Tuya 设备清单落库任务失败: {e}");
                (0, rows.len())
            }
        }
    }

    /// 收尾：写 sync_history（失败只告警——同步本身已尽力，历史缺失不改变结果）。
    async fn finish_sync(
        &self,
        started_at: i64,
        result: &str,
        count: i64,
        message: impl Into<String>,
    ) -> SyncOutcome {
        let message = message.into();
        tracing::info!("[tuya] 云同步完成：result={result}, count={count}, message={message}");
        let record = SyncRecord {
            started_at,
            finished_at: now_ms(),
            result: result.to_string(),
            device_count: count,
            message: message.clone(),
        };
        let store = self.store.clone();
        let task_record = record.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || {
            store.record_sync(&task_record).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| format!("写库任务失败: {e}"))
        .and_then(|r| r)
        {
            tracing::warn!("Tuya sync_history 写库失败: {e}");
        }
        SyncOutcome::new(result, message)
    }
}

/// 云错误 → (sync_history 结果, 人话消息)。
fn classify_cloud_error(err: &CloudError, fetched: usize) -> (&'static str, String) {
    match err {
        CloudError::Auth(m) => (
            "cloud_auth_failed",
            format!("涂鸦云鉴权失败：{m}（请检查凭证与区域）"),
        ),
        CloudError::Network(m) => {
            if fetched > 0 {
                ("partial", format!("部分页拉取失败：{m}"))
            } else {
                ("network_error", format!("云请求失败：{m}"))
            }
        }
        CloudError::Api(m) => {
            if fetched > 0 {
                ("partial", format!("部分页云接口错误：{m}"))
            } else {
                ("network_error", format!("云接口错误：{m}"))
            }
        }
    }
}
