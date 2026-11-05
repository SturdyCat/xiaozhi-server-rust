//! 60s 同步闸门单测：`sync_now` / `auto_sync_due` / `MIN_SYNC_INTERVAL`。
//!
//! 时钟注入方式：不 sleep，直接改写 `sync_state.last_start`（闸门状态对同模块
//! 测试可见）来模拟"距上次同步 X 秒"。execute_sync 走未配置凭证的快速失败路径
//! （`cloud_auth_failed`），零网络依赖，只落临时目录 SQLite。
//!
//! 归属缺陷（QA 回归）：60s 闸门此前零测试覆盖——防连点 + 涂鸦云免费额度保护的
//! 唯一防线不能只靠人肉回归。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::sync::MIN_SYNC_INTERVAL;
use super::{TuyaConfig, TuyaEngine};

/// 临时目录引擎（对齐 store/tests.rs 的 temp 模式；返回 dir 便于清理）。
fn temp_engine(tag: &str) -> (PathBuf, TuyaEngine) {
    let dir = std::env::temp_dir().join(format!(
        "xiaozhi-tuya-sync-gate-test-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    (dir.clone(), TuyaEngine::new(&dir))
}

/// 未配置凭证的配置：闸门放行后 execute_sync 走 `cloud_auth_failed` 快速路径。
fn unconfigured_cfg() -> TuyaConfig {
    TuyaConfig::default()
}

/// 直改闸门状态：模拟"距上次同步开始已过 `ago` 秒"。
fn set_last_start(engine: &TuyaEngine, ago: Option<Duration>) {
    let mut state = engine.sync_state.lock().unwrap_or_else(|e| e.into_inner());
    state.last_start = ago.map(|d| Instant::now() - d);
    state.syncing = false;
}

// ---- sync_now 闸门 ----

/// 首次调用（无历史 last_start）→ 闸门放行（结果不是 skipped）。
#[tokio::test]
async fn sync_now_first_call_passes_gate() {
    let (_dir, engine) = temp_engine("first");
    let outcome = engine.sync_now(&unconfigured_cfg()).await;
    assert_ne!(
        outcome.result, "skipped",
        "首次同步不应被 60s 闸门拦截: {}",
        outcome.message
    );
    assert_eq!(outcome.result, "cloud_auth_failed");
    // 放行的同时必须记下本次时刻（否则闸门对后续调用失效）
    let state = engine.sync_state.lock().unwrap_or_else(|e| e.into_inner());
    assert!(state.last_start.is_some(), "放行后必须记录 last_start");
}

/// 间隔内（< 60s）→ 拒绝（skipped），且 syncing 不被置位。
#[tokio::test]
async fn sync_now_rejected_within_interval() {
    let (_dir, engine) = temp_engine("within");
    set_last_start(
        &engine,
        Some(Duration::from_secs(MIN_SYNC_INTERVAL.as_secs() - 1)),
    );
    let outcome = engine.sync_now(&unconfigured_cfg()).await;
    assert_eq!(outcome.result, "skipped", "间隔内必须被拦截");
    assert!(
        outcome.message.contains("60"),
        "拒绝文案应提示最小间隔: {}",
        outcome.message
    );
    let state = engine.sync_state.lock().unwrap_or_else(|e| e.into_inner());
    assert!(!state.syncing, "被拦截的调用不得占用 syncing 标记");
}

/// 间隔外（≥ 60s）→ 重新放行。
#[tokio::test]
async fn sync_now_passes_after_interval() {
    let (_dir, engine) = temp_engine("after");
    set_last_start(&engine, Some(MIN_SYNC_INTERVAL + Duration::from_secs(1)));
    let outcome = engine.sync_now(&unconfigured_cfg()).await;
    assert_ne!(
        outcome.result, "skipped",
        "超过 60s 后应放行: {}",
        outcome.message
    );
}

// ---- auto_sync_due（worker 定时判定）----

/// 无历史（进程刚启动）→ 首个 tick 即到期（启动目录刷新）。
#[test]
fn auto_sync_due_no_history_returns_true() {
    let (_dir, engine) = temp_engine("no-history");
    assert!(
        engine.auto_sync_due(3600),
        "无 last_start 时首个 tick 应判定到期"
    );
}

/// interval = 0（仅手动）→ 永不触发，即使无历史。
#[test]
fn auto_sync_due_disabled_when_interval_zero() {
    let (_dir, engine) = temp_engine("interval-zero");
    assert!(!engine.auto_sync_due(0), "interval=0 必须关闭定时同步");
}

/// 间隔内 → false；间隔外 → true。
#[test]
fn auto_sync_due_respects_interval() {
    let (_dir, engine) = temp_engine("interval");
    set_last_start(&engine, Some(Duration::from_secs(10)));
    assert!(!engine.auto_sync_due(60), "10s 前同步过，60s 周期不应到期");
    set_last_start(&engine, Some(Duration::from_secs(61)));
    assert!(engine.auto_sync_due(60), "61s 前同步过，60s 周期应到期");
}

/// 同步进行中 → false（防并发重入）。
#[test]
fn auto_sync_due_false_while_syncing() {
    let (_dir, engine) = temp_engine("syncing");
    {
        let mut state = engine.sync_state.lock().unwrap_or_else(|e| e.into_inner());
        state.syncing = true;
        state.last_start = None;
    }
    assert!(!engine.auto_sync_due(3600), "syncing 期间不应判定到期");
}
