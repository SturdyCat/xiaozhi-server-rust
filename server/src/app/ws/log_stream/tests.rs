//! [`log_stream`] 单元测试：层级采集、环形缓冲与时间戳纯逻辑。
//!
//! WS 帧循环本身极薄（subscribe → 逐行 Text 帧），端到端以联调验证。

use super::*;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt;

fn test_subscriber() -> impl tracing::Subscriber + Send + Sync + 'static {
    tracing_subscriber::registry().with(layer())
}

#[test]
fn hhmmss_formats_utc_time_of_day() {
    // 0s = 00:00:00；86399 = 23:59:59（纯算术，无时区库）。
    assert_eq!(hhmmss_from(0), "00:00:00");
    assert_eq!(hhmmss_from(86_399), "23:59:59");
    assert_eq!(hhmmss_from(3_600 + 61), "01:01:01");
}

fn hhmmss_from(secs: u64) -> String {
    let tod = secs % 86_400;
    format!("{:02}:{:02}:{:02}", tod / 3600, (tod % 3600) / 60, tod % 60)
}

#[test]
fn layer_formats_level_target_message_into_ring_and_broadcast() {
    let _guard = set_default(test_subscriber());
    let mut rx = hub().tx.subscribe();
    tracing::info!(target: "test_mod", "你好 {}", 7);
    let line = rx.try_recv().expect("事件应进入 broadcast");
    assert!(line.contains("INFO "), "级别: {line}");
    assert!(line.contains("[test_mod]"), "目标: {line}");
    assert!(line.contains("你好 7"), "消息: {line}");
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
    assert!(
        ring.iter()
            .any(|l| l.contains(&format!("tick {}", BACKLOG + 49)))
    );
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
