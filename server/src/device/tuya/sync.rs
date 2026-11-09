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
    ///
    /// 引擎分派（S3e）：`[tuya].engine = "wasm-policy"` 且宿主已注入 → 目录拉取经
    /// `connector.tuya` 插件云链路（协议语义在插件、续页编排与落库留 native）；
    /// 缺宿主/缺 feature 显式失败，不静默回退 native。其余情况走 native CloudClient。
    pub(crate) async fn execute_sync(&self, cfg: &TuyaConfig) -> SyncOutcome {
        let started_at = now_ms();
        #[cfg(feature = "wasm-plugins")]
        if cfg.engine_id() == "wasm-policy" {
            let host = self.wasm_host.get();
            if host.is_none() {
                return self
                    .finish_sync(
                        started_at,
                        "network_error",
                        0,
                        "[tuya].engine = \"wasm-policy\" 但 WASM 宿主未注入（宿主初始化失败或未编入）",
                    )
                    .await;
            }
            return self.plugin_execute_sync(started_at, host.unwrap()).await;
        }
        #[cfg(not(feature = "wasm-plugins"))]
        if cfg.engine_id() == "wasm-policy" {
            return self
                .finish_sync(
                    started_at,
                    "network_error",
                    0,
                    "[tuya].engine = \"wasm-policy\" 需要以 wasm-plugins feature 构建的服务端",
                )
                .await;
        }
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

// ============================================================
// S3e：插件云链路目录拉取（`[tuya].engine = "wasm-policy"`）
// ============================================================
//
// 协议语义（签名/解析）在 tuya.official（与宿主共用单源协议，见
// server/src/device/tuya/protocol.rs 与 plugins/wasm/tests/tuya_cloud.rs）；
// **编排与落库留 native**：token 一次 + 翻页 ≤ MAX_PAGES + upsert 全在宿主。
// 凭证来自插件配置命名空间（管理台插件配置），不读 [tuya] 段——两套凭证
// 互不混淆。取数步含阻塞 guest + 效果 HTTP，走 spawn_blocking（与
// plugins/weather/dispatch.rs 同纪律）。插件路径错误统一按 network_error
// 归类（Auth 分类是 native 客户端的业务语义，不跨插件边界）。

/// 承接云链路的官方插件 id（与 plugins-src/tuya、manifest.toml 登记项一致）。
#[cfg(feature = "wasm-plugins")]
const PLUGIN_CLOUD_ID: &str = "tuya.official";

/// 派发一个云链路事件并读回 kv 结果通道（阻塞步 → spawn_blocking）。
#[cfg(feature = "wasm-plugins")]
async fn plugin_dispatch_result(
    host: &std::sync::Arc<crate::plugins::wasm::WasmHost>,
    topic: &str,
    payload: serde_json::Value,
    result_key: &str,
) -> Result<serde_json::Value, String> {
    use serde_json::Value;

    let host = std::sync::Arc::clone(host);
    let topic = topic.to_string();
    let payload = payload.to_string();
    let result_key = result_key.to_string();
    let bytes = tokio::task::spawn_blocking(move || {
        let errors = host
            .handle_event(PLUGIN_CLOUD_ID, &topic, &payload)
            .map_err(|e| format!("插件派发失败：{e:#}"))?;
        if !errors.is_empty() {
            return Err(format!("插件效果执行失败：{errors:?}"));
        }
        host.kv()
            .get(PLUGIN_CLOUD_ID, &result_key)
            .ok_or_else(|| format!("插件未返回结果（需要 kv 键 {result_key}）"))
    })
    .await
    .map_err(|e| format!("插件任务失败：{e}"))??;
    serde_json::from_slice::<Value>(&bytes).map_err(|e| format!("插件结果损坏：{e}"))
}

/// 原始设备行 → [`CloudDevice`] 投影（业务语义，不进单源协议）。
#[cfg(feature = "wasm-plugins")]
fn project_device(d: &serde_json::Value) -> Option<CloudDevice> {
    use serde_json::Value;

    Some(CloudDevice {
        device_id: d.get("id").and_then(Value::as_str)?.to_string(),
        name: d
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        model: d
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        local_key: d
            .get("local_key")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        online: d.get("online").and_then(Value::as_bool).unwrap_or(false),
        ip: d
            .get("ip")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        product_id: d
            .get("product_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

#[cfg(feature = "wasm-plugins")]
impl TuyaEngine {
    /// 经插件云链路拉全量目录：token 一次 + 翻页 ≤ [`super::cloud::MAX_PAGES`]，
    /// 逐页落库复用 [`Self::upsert_all`]，四归类与 native 路径同口径
    /// （partial = 已取到的设备照常落库）。
    async fn plugin_execute_sync(
        &self,
        started_at: i64,
        host: &std::sync::Arc<crate::plugins::wasm::WasmHost>,
    ) -> SyncOutcome {
        use serde_json::json;

        // ① token（一次；后续页复用显式 token）
        let req = uuid::Uuid::new_v4().to_string();
        let token = match plugin_dispatch_result(
            host,
            "connector.tuya.cloud-token",
            json!({"req_id": req}),
            &format!("tuya-cloud-token:{req}"),
        )
        .await
        {
            Ok(v) if v["ok"] == json!(true) => {
                v["access_token"].as_str().unwrap_or_default().to_string()
            }
            Ok(v) => {
                let error = v["error"].as_str().unwrap_or("未知").to_string();
                return self
                    .finish_sync(
                        started_at,
                        "network_error",
                        0,
                        format!("插件云链路 token 失败：{error}"),
                    )
                    .await;
            }
            Err(e) => {
                return self.finish_sync(started_at, "network_error", 0, e).await;
            }
        };
        if token.is_empty() {
            return self
                .finish_sync(started_at, "network_error", 0, "插件云链路返回空 token")
                .await;
        }

        // ② 翻页（游标来自上一页；MAX_PAGES 护住死循环）
        let mut devices: Vec<CloudDevice> = Vec::new();
        let mut cursor = String::new();
        let mut last_error: Option<String> = None;
        for _ in 0..super::cloud::MAX_PAGES {
            let req = uuid::Uuid::new_v4().to_string();
            match plugin_dispatch_result(
                host,
                "connector.tuya.cloud-devices",
                json!({"req_id": req, "token": token, "last_row_key": cursor}),
                &format!("tuya-devices:{req}"),
            )
            .await
            {
                Ok(v) if v["ok"] == json!(true) => {
                    if let Some(arr) = v["devices"].as_array() {
                        devices.extend(arr.iter().filter_map(project_device));
                    }
                    if v["has_more"] != json!(true) {
                        break;
                    }
                    cursor = v["last_row_key"].as_str().unwrap_or_default().to_string();
                    if cursor.is_empty() {
                        break;
                    }
                }
                Ok(v) => {
                    last_error = Some(format!(
                        "插件云链路设备页失败：{}",
                        v["error"].as_str().unwrap_or("未知")
                    ));
                    break;
                }
                Err(e) => {
                    last_error = Some(e);
                    break;
                }
            }
        }

        // ③ 落库 + 四归类（partial：已取到的部分先落库，再报错——同 native 口径）
        if devices.is_empty() {
            let message = last_error.unwrap_or_else(|| "插件云链路未取到设备".to_string());
            return self
                .finish_sync(started_at, "network_error", 0, message)
                .await;
        }
        let (upserted, failed) = self.upsert_all(&devices, "").await;
        if let Some(e) = last_error {
            return self
                .finish_sync(
                    started_at,
                    "network_error",
                    upserted as i64,
                    format!("{e}；已落库 {upserted} 台（{failed} 台写库失败）"),
                )
                .await;
        }
        let mut message = format!("同步 {upserted} 台设备（经插件云链路）");
        if failed > 0 {
            message.push_str(&format!("（{failed} 台写库失败）"));
        }
        self.finish_sync(started_at, "ok", upserted as i64, message)
            .await
    }
}
