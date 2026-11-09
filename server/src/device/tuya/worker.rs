//! Tuya 后台 worker：定时云同步 + 60s 错峰在线轮询 + 5min 局域网发现扫描。
//!
//! ## 热生效
//!
//! 常驻任务**每 tick 重读配置**（SQLite `cfg_documents`，`spawn_blocking`）：
//! `[tuya].enabled` 关了 → 本 tick 什么都不做；凭证指纹变化 → 丢弃 token 缓存
//! （[`TuyaEngine::check_credentials`]）；同步周期改了 → 下一 tick 按新值判定。
//! 全程不需要重启服务端。
//!
//! ## 错峰
//!
//! - 在线轮询：每台设备按 device_id 哈希散列到 60s 相位（[`TuyaEngine::poll_due`]），
//!   避免整点齐发几十台 TCP 握手；
//! - 发现扫描：启动先扫一轮，之后每 5 分钟一轮（[`SCAN_INTERVAL`]）。

use std::sync::Arc;
use std::time::Duration;

use crate::runtime::Engines;

use super::{TuyaEngine, SCAN_INTERVAL};

#[cfg(test)]
#[path = "worker_tests.rs"]
mod worker_tests;

/// worker 心跳周期：每 tick 重读配置并检查各定时器。
const TICK: Duration = Duration::from_secs(60);

/// 启动 Tuya worker（常驻 task；dev 模式由调用方决定是否 spawn）。
pub fn spawn(engines: Arc<Engines>) {
    tokio::spawn(async move {
        let engine: Arc<TuyaEngine> = engines.tuya.clone();
        let mut scan_due_at = tokio::time::Instant::now(); // 启动先扫一轮
        let mut last_scan_merged: Option<usize> = None; // 发现日志去重（台数没变不刷 INFO）
        loop {
            tokio::time::sleep(TICK).await;
            tick(&engines, &engine, &mut scan_due_at, &mut last_scan_merged).await;
        }
    });
}

/// 一个 tick：读配置 →（enabled 时）凭证热检 + 定时同步 + 在线轮询 + 发现扫描。
/// 任何一步失败只告警，不终止 worker（下一 tick 重来）。
async fn tick(
    engines: &Engines,
    engine: &TuyaEngine,
    scan_due_at: &mut tokio::time::Instant,
    last_scan_merged: &mut Option<usize>,
) {
    let cfg = match load_config(engines).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!("Tuya worker 读配置失败（本 tick 跳过）: {e}");
            return;
        }
    };
    let tuya = &cfg.tuya;
    if !tuya.enabled {
        return;
    }
    // 凭证热生效：指纹变化 → 丢弃 token 缓存
    engine.check_credentials(tuya);
    // 定时云同步（手动按钮共用同一闸门与 60s 间隔）
    if engine.auto_sync_due(tuya.sync_interval_secs) {
        let outcome = engine.sync_now(tuya).await;
        if outcome.result != "skipped" {
            tracing::info!("[tuya] 定时同步：{}（{}）", outcome.result, outcome.message);
        }
    }
    // 在线状态轮询（每设备错峰 60s；引擎内部判定到期设备）
    let online = engine.poll_due().await;
    if online > 0 {
        tracing::debug!("[tuya] 本轮在线探测：{online} 台在线");
    }
    // 局域网发现（启动 + 每 5 分钟）；合并台数与上轮相同只留 debug，
    // 稳态环境每 5 分钟一条 INFO 也是刷屏（状态有变化才升 INFO）。
    if tokio::time::Instant::now() >= *scan_due_at {
        *scan_due_at = tokio::time::Instant::now() + SCAN_INTERVAL;
        let merged = engine.scan_and_merge().await;
        if scan_report_worthy(*last_scan_merged, merged) {
            tracing::info!("[tuya] 局域网发现完成：合并 {merged} 台已登记设备");
        } else {
            tracing::debug!("[tuya] 局域网发现完成：合并 {merged} 台已登记设备（与上轮相同）");
        }
        *last_scan_merged = Some(merged);
    }
}

/// 发现结果是否值得打 INFO：合并台数与上轮不同（状态变化）才算新消息，
/// 否则降为 debug——`worker_tests` 钉住口径。
fn scan_report_worthy(last: Option<usize>, merged: usize) -> bool {
    last != Some(merged)
}

/// 读最新配置（热生效唯一来源：SQLite 配置库；spawn_blocking）。
async fn load_config(engines: &Engines) -> anyhow::Result<crate::config::Config> {
    let store = engines.config_store.clone();
    tokio::task::spawn_blocking(move || store.load()).await?
}
