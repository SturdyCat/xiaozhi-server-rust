//! 定时器：插件可申请的 timeout / interval。
//!
//! 到期后宿主把 `host.timer.fired` 投递到事件总线（registry 负责查询订阅者）。
//! 插件停用时该插件的全部定时器被撤销（见 [`TimerWheel::clear_plugin`]）。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};

/// 到期回调：宿主把事件投给插件。
pub trait TimerSink: Send + Sync {
    fn fired(&self, plugin_id: &str, timer_id: u64, kind: &str);
}

/// 单次定时器上限（防插件刷爆宿主任务表）。
const MAX_TIMERS_PER_PLUGIN: usize = 64;
/// interval 最短周期（防 0 周期忙等）。
const MIN_INTERVAL_MS: u64 = 50;

struct Handle {
    plugin_id: String,
    abort: tokio::task::JoinHandle<()>,
}

/// 定时器轮。
pub struct TimerWheel {
    next_id: AtomicU64,
    handles: Mutex<BTreeMap<u64, Handle>>,
    runtime: Option<tokio::runtime::Handle>,
    sink: Arc<dyn TimerSink>,
}

impl TimerWheel {
    pub fn new(runtime: Option<tokio::runtime::Handle>, sink: Arc<dyn TimerSink>) -> Arc<Self> {
        Arc::new(Self {
            next_id: AtomicU64::new(1),
            handles: Mutex::new(BTreeMap::new()),
            runtime,
            sink,
        })
    }

    fn count(&self, plugin_id: &str) -> usize {
        self.handles
            .lock()
            .map(|h| h.values().filter(|x| x.plugin_id == plugin_id).count())
            .unwrap_or(0)
    }

    fn rt(&self) -> Result<&tokio::runtime::Handle> {
        self.runtime
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("宿主未绑定 tokio 运行时，定时器不可用"))
    }

    /// 单次延时。
    pub fn set_timeout(&self, plugin_id: &str, delay_ms: u64) -> Result<u64> {
        self.check_quota(plugin_id)?;
        let rt = self.rt()?.clone();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let sink = Arc::clone(&self.sink);
        let owner = plugin_id.to_string();
        let handle = rt.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            sink.fired(&owner, id, "timeout");
        });
        self.record(plugin_id, id, handle)?;
        Ok(id)
    }

    /// 周期。
    pub fn set_interval(&self, plugin_id: &str, interval_ms: u64) -> Result<u64> {
        if interval_ms < MIN_INTERVAL_MS {
            bail!("interval 周期过短：{interval_ms}ms（下限 {MIN_INTERVAL_MS}ms）");
        }
        self.check_quota(plugin_id)?;
        let rt = self.rt()?.clone();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let sink = Arc::clone(&self.sink);
        let owner = plugin_id.to_string();
        let handle = rt.spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_millis(interval_ms));
            // 首次 tick 立即返回：跳过，避免激活瞬间就触发
            ticker.tick().await;
            loop {
                ticker.tick().await;
                sink.fired(&owner, id, "interval");
            }
        });
        self.record(plugin_id, id, handle)?;
        Ok(id)
    }

    fn record(&self, plugin_id: &str, id: u64, abort: tokio::task::JoinHandle<()>) -> Result<()> {
        let mut handles = self
            .handles
            .lock()
            .map_err(|_| anyhow::anyhow!("定时器锁中毒"))?;
        handles.insert(
            id,
            Handle {
                plugin_id: plugin_id.to_string(),
                abort,
            },
        );
        Ok(())
    }

    fn check_quota(&self, plugin_id: &str) -> Result<()> {
        if self.count(plugin_id) >= MAX_TIMERS_PER_PLUGIN {
            bail!(
                "插件 {plugin_id} 定时器数量超上限（{MAX_TIMERS_PER_PLUGIN}）"
            );
        }
        Ok(())
    }

    /// 撤销单个定时器（只有 owner 能撤销自己的）。
    pub fn clear(&self, plugin_id: &str, timer_id: u64) -> Result<()> {
        let mut handles = self
            .handles
            .lock()
            .map_err(|_| anyhow::anyhow!("定时器锁中毒"))?;
        match handles.get(&timer_id) {
            Some(h) if h.plugin_id == plugin_id => {
                h.abort.abort();
                handles.remove(&timer_id);
                Ok(())
            }
            Some(_) => bail!("定时器 {timer_id} 不属于插件 {plugin_id}"),
            None => bail!("定时器 {timer_id} 不存在"),
        }
    }

    /// 撤销某插件的全部定时器（deactivate / remove）。
    pub fn clear_plugin(&self, plugin_id: &str) {
        if let Ok(mut handles) = self.handles.lock() {
            let ids: Vec<u64> = handles
                .iter()
                .filter(|(_, h)| h.plugin_id == plugin_id)
                .map(|(id, _)| *id)
                .collect();
            for id in ids {
                if let Some(h) = handles.remove(&id) {
                    h.abort.abort();
                }
            }
        }
    }

    pub fn active_count(&self) -> usize {
        self.handles.lock().map(|h| h.len()).unwrap_or(0)
    }
}
