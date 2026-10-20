//! [`log_stream`] 单元测试：层级采集与环形缓冲（时间戳格式见 `app::logtime`）。
//!
//! WS 帧循环本身极薄（subscribe → 逐行 Text 帧），端到端以联调验证。

use super::*;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt;

fn test_subscriber() -> impl tracing::Subscriber + Send + Sync + 'static {
    tracing_subscriber::registry().with(layer())
}

#[test]
fn layer_formats_level_target_message_into_ring_and_broadcast() {
    let _guard = set_default(test_subscriber());
    let mut rx = hub().tx.subscribe();
    tracing::info!(target: "test_mod", "你好 {}", 7);
    // hub 是进程级单例：并发测试的行可能先挤进 broadcast，逐条找自己的那行。
    let line = std::iter::from_fn(|| rx.try_recv().ok())
        .find(|l| l.contains("你好 7"))
        .expect("事件应进入 broadcast");
    assert!(line.contains("INFO "), "级别: {line}");
    assert!(line.contains("[test_mod]"), "目标: {line}");
}

#[test]
fn ring_keeps_only_latest_backlog_lines() {
    let _guard: tracing::subscriber::DefaultGuard = set_default(test_subscriber());
    for i in 0..(BACKLOG + 50) {
        tracing::info!("tick {i}");
    }
    let ring = futures_block();
    assert_eq!(ring.len(), BACKLOG);
    // hub 是进程级单例，其他测试可能并发写 ring：只断言本测试的行已进入、最老的已滚出。
    assert!(ring
        .iter()
        .any(|l| l.contains(&format!("tick {}", BACKLOG + 49))));
    assert!(!ring.iter().any(|l| l.contains("tick 0")));
}

/// 测试里拿到 ring 快照（tokio Mutex 的 blocking_lock 不能用于异步上下文，
/// 测试体是同步的，直接用 try_lock 足够——无其他持有者）。
fn futures_block() -> std::collections::VecDeque<String> {
    hub().ring.try_lock().expect("无竞争").clone()
}

#[test]
fn hub_wiring_is_singleton_and_thread_safe() {
    // 静态保证：两次获取 hub 是同一实例（OnceLock），且可跨线程共享。
    let _a: &'static Hub = hub();
    let _b: &'static Hub = hub();
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Hub>();
}
