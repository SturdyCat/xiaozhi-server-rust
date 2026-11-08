//! 事件总线：topic 路由 + 订阅匹配 + 每插件有界队列。
//!
//! 宿主**不解释任何 topic 语义**，只做三件事：匹配订阅、按顶层键值过滤、投递。
//! 固定 topic（`host.*` / `sys.*` / `io.*`）由宿主产生；插件只能发布
//! `<plugin-id>.*`（在 [`super::registry`] 的 `publish` 守卫里校验）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use tokio::sync::mpsc;

/// 一次事件投递（宿主内部形态；与 WIT `event` 一一对应）。
#[derive(Debug, Clone)]
pub struct Envelope {
    pub topic: String,
    pub correlation_id: Option<String>,
    pub payload: Vec<u8>,
}

impl Envelope {
    pub fn json(topic: impl Into<String>, value: &serde_json::Value) -> Self {
        Self {
            topic: topic.into(),
            correlation_id: value
                .get("id")
                .or_else(|| value.get("request_id"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            payload: value.to_string().into_bytes(),
        }
    }

    pub fn bytes(topic: impl Into<String>, payload: Vec<u8>) -> Self {
        Self {
            topic: topic.into(),
            correlation_id: None,
            payload,
        }
    }
}

/// 订阅者：一个插件对某 topic（或 `prefix.*`）的投递通道。
#[derive(Clone)]
struct Subscriber {
    plugin_id: String,
    filter: Option<serde_json::Value>,
    tx: mpsc::Sender<Envelope>,
}

impl Subscriber {
    /// 顶层键值精确匹配；未声明过滤 → 全收。
    fn accepts(&self, env: &Envelope) -> bool {
        let Some(filter) = &self.filter else {
            return true;
        };
        let Some(obj) = filter.as_object() else {
            return true;
        };
        let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&env.payload) else {
            // 二进制载荷无法做 JSON 过滤：只有空过滤才放行，这里已排除
            return false;
        };
        let Some(pay_obj) = payload.as_object() else {
            return false;
        };
        obj.iter()
            .all(|(k, v)| pay_obj.get(k).map(|pv| pv == v).unwrap_or(false))
    }
}

/// 总线统计（`/api/plugins` 可投影）。
#[derive(Debug, Default)]
pub struct BusStats {
    pub published: AtomicU64,
    pub delivered: AtomicU64,
    pub dropped: AtomicU64,
    pub unmatched: AtomicU64,
}

#[derive(Default)]
struct Inner {
    /// 精确 topic → 订阅者
    exact: HashMap<String, Vec<Subscriber>>,
    /// 尾部通配：前缀（不含 `.*`）→ 订阅者
    wildcard: HashMap<String, Vec<Subscriber>>,
}

/// 事件总线。
pub struct EventBus {
    inner: RwLock<Inner>,
    stats: BusStats,
}

impl Default for EventBus {
    fn default() -> Self {
        Self {
            inner: RwLock::new(Inner::default()),
            stats: BusStats::default(),
        }
    }
}

impl EventBus {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 注册订阅。`tx` 由 registry 为每个插件创建（有界队列）。
    pub fn subscribe(
        &self,
        plugin_id: &str,
        topic: &str,
        filter: Option<serde_json::Value>,
        tx: mpsc::Sender<Envelope>,
    ) {
        let sub = Subscriber {
            plugin_id: plugin_id.to_string(),
            filter,
            tx,
        };
        let Ok(mut inner) = self.inner.write() else {
            return;
        };
        if let Some(prefix) = topic.strip_suffix(".*") {
            inner
                .wildcard
                .entry(prefix.to_string())
                .or_default()
                .push(sub);
        } else {
            inner
                .exact
                .entry(topic.to_string())
                .or_default()
                .push(sub);
        }
    }

    /// 移除某插件的全部订阅（deactivate / remove）。
    pub fn unsubscribe_all(&self, plugin_id: &str) {
        let Ok(mut inner) = self.inner.write() else {
            return;
        };
        for list in inner.exact.values_mut() {
            list.retain(|s| s.plugin_id != plugin_id);
        }
        for list in inner.wildcard.values_mut() {
            list.retain(|s| s.plugin_id != plugin_id);
        }
        inner.exact.retain(|_, v| !v.is_empty());
        inner.wildcard.retain(|_, v| !v.is_empty());
    }

    /// 发布。返回 `true` = 全部入队成功；`false` = 至少一个订阅者队列满（背压）。
    ///
    /// 同一插件同时命中精确与通配订阅时只投一次（精确优先）。
    pub fn publish(&self, env: &Envelope) -> bool {
        self.stats.published.fetch_add(1, Ordering::Relaxed);
        let targets: Vec<Subscriber> = {
            let Ok(inner) = self.inner.read() else {
                return false;
            };
            let mut targets = Vec::new();
            let mut seen = std::collections::HashSet::new();
            if let Some(list) = inner.exact.get(&env.topic) {
                for s in list {
                    if seen.insert(s.plugin_id.clone()) {
                        targets.push(s.clone());
                    }
                }
            }
            // 尾部通配：逐级前缀匹配（`a.b.c` 命中 `a.*` 与 `a.b.*`）
            let mut cursor = env.topic.as_str();
            while let Some(pos) = cursor.rfind('.') {
                cursor = &cursor[..pos];
                if let Some(list) = inner.wildcard.get(cursor) {
                    for s in list {
                        if seen.insert(s.plugin_id.clone()) {
                            targets.push(s.clone());
                        }
                    }
                }
            }
            targets
        };

        if targets.is_empty() {
            self.stats.unmatched.fetch_add(1, Ordering::Relaxed);
            return true;
        }

        let mut all_ok = true;
        for sub in targets {
            if !sub.accepts(env) {
                continue;
            }
            match sub.tx.try_send(env.clone()) {
                Ok(()) => {
                    self.stats.delivered.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    // 队列满或已关闭：丢最旧由 worker 侧保证，这里只计数 + 背压
                    self.stats.dropped.fetch_add(1, Ordering::Relaxed);
                    all_ok = false;
                }
            }
        }
        all_ok
    }

    /// 当前订阅者数量（诊断）。
    pub fn subscriber_count(&self) -> usize {
        self.inner
            .read()
            .map(|i| {
                i.exact.values().map(Vec::len).sum::<usize>()
                    + i.wildcard.values().map(Vec::len).sum::<usize>()
            })
            .unwrap_or(0)
    }

    pub fn stats_json(&self) -> serde_json::Value {
        serde_json::json!({
            "published": self.stats.published.load(Ordering::Relaxed),
            "delivered": self.stats.delivered.load(Ordering::Relaxed),
            "dropped": self.stats.dropped.load(Ordering::Relaxed),
            "unmatched": self.stats.unmatched.load(Ordering::Relaxed),
            "subscribers": self.subscriber_count(),
        })
    }
}

/// 校验插件只能发布自己命名空间的 topic。
pub fn publish_allowed(plugin_id: &str, topic: &str) -> bool {
    if topic.starts_with("host.") || topic.starts_with("sys.") || topic.starts_with("io.") {
        return false;
    }
    topic
        .strip_prefix(plugin_id)
        .map(|rest| rest.starts_with('.'))
        .unwrap_or(false)
}
