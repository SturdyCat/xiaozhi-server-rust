//! `server/src/app/inference.rs` 的单元测试。
//!
//! 覆盖闸门的结构性语义（不依赖真实引擎、不依赖时间精度）：
//! 许可排队与释放、abort 打断（TTS 侧）、reconfigure 热对账、stats_json 形状、clamp。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::InferenceGates;
use crate::config::Config;

fn gates(permits: u32) -> InferenceGates {
    let mut cfg = Config::default();
    cfg.asr.max_concurrent = permits;
    cfg.tts.max_concurrent = permits;
    InferenceGates::new(&cfg)
}

/// 默认配置 = 每能力 1 路许可（缺口①的拍板决策：段预算口径）。
#[test]
fn default_permits_are_one_per_capability() {
    let g = InferenceGates::new(&Config::default());
    let j = g.stats_json();
    assert_eq!(j["inference_gates"]["asr"]["permits"], 1);
    assert_eq!(j["inference_gates"]["tts"]["permits"], 1);
}

/// clamp：配置写 0 也不得把推理永久堵死（≥1）。
#[test]
fn permits_clamped_to_at_least_one() {
    let mut cfg = Config::default();
    cfg.asr.max_concurrent = 0;
    cfg.tts.max_concurrent = 0;
    let g = InferenceGates::new(&cfg);
    let j = g.stats_json();
    assert_eq!(j["inference_gates"]["asr"]["permits"], 1, "asr clamp ≥1");
    assert_eq!(j["inference_gates"]["tts"]["permits"], 1, "tts clamp ≥1");
}

/// 排队串行：第二路必须等第一路释放 permit 后才能进（RAII Drop 归还）。
#[tokio::test]
async fn second_acquire_waits_until_first_released() {
    let g = gates(1);
    let gate = g.asr();
    let p1 = gate.acquire().await.expect("空闲时应立即获得许可");
    // 单许可被占 → 第二次获取在 200ms 内不应完成
    let second = gate.acquire();
    match tokio::time::timeout(Duration::from_millis(200), second).await {
        Ok(_) => panic!("单许可被占时第二次获取必须排队"),
        Err(_) => {}
    }
    drop(p1); // RAII 归还
    let p2 = tokio::time::timeout(Duration::from_millis(500), gate.acquire())
        .await
        .expect("释放后应立即获得许可")
        .expect("信号量不应被关闭");
    drop(p2);
}

/// TTS 侧：排队期间 abort 置位 → 放弃等待返回 `None`（不白等）。
#[tokio::test]
async fn abortable_acquire_gives_up_on_abort() {
    let g = gates(1);
    let gate = g.tts();
    let _held = gate.acquire().await.expect("先占住唯一许可");
    let abort = std::sync::Arc::new(AtomicBool::new(false));
    let setter = {
        let abort = abort.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            abort.store(true, Ordering::Relaxed);
        }
    };
    let started = std::time::Instant::now();
    let r = tokio::join!(gate.acquire_with_abort(&abort), setter).0;
    assert!(r.is_none(), "abort 置位后应放弃排队");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "放弃必须及时（200ms 轮询粒度）"
    );
}

/// abort 未置位时 abortable 获取照常成功（与非 abort 路径同语义）。
#[tokio::test]
async fn abortable_acquire_succeeds_when_idle() {
    let g = gates(2);
    let gate = g.tts();
    let abort = AtomicBool::new(false);
    let p = gate
        .acquire_with_abort(&abort)
        .await
        .expect("空闲时无论 abort 与否都应立即获得");
    drop(p);
}

/// 热切换对账：许可数变化 → 换新闸门（新获取按新上限走）；许可数不变 → 不动。
#[tokio::test]
async fn reconfigure_tts_swaps_semaphore_when_permits_change() {
    let g = gates(1);
    // 先占住旧闸门的唯一许可
    let _old_permit = g.tts().acquire().await.expect("旧闸门 1 许可可用");

    g.reconfigure_tts(2);
    let j = g.stats_json();
    assert_eq!(j["inference_gates"]["tts"]["permits"], 2, "许可数应已调整");

    // 新闸门 2 许可：旧 permit 仍被持有（瞬时重叠是文档化的可接受行为），
    // 新获取应能立刻再拿 2 个（第 3 个才会排队）。
    let a = g.tts().acquire().await.expect("新闸门许可 1");
    let b = tokio::time::timeout(Duration::from_millis(200), g.tts().acquire())
        .await
        .expect("新闸门许可 2 应立即可得")
        .expect("信号量不应被关闭");
    drop((a, b, _old_permit));

    // 不变的许可数 → 无操作（幂等，无日志噪音）
    g.reconfigure_tts(2);
}

/// stats_json 形状：inference_gates 节包含 asr/tts 两组计数（additive 投影契约）。
#[test]
fn stats_json_shape() {
    let j = gates(3).stats_json();
    let root = j
        .get("inference_gates")
        .expect("应有 inference_gates 节");
    for cap in ["asr", "tts"] {
        let n = &root[cap];
        assert!(n.get("permits").is_some(), "{cap} 应有 permits");
        assert!(n.get("wait_events").is_some(), "{cap} 应有 wait_events");
        assert!(n.get("wait_ms_total").is_some(), "{cap} 应有 wait_ms_total");
        assert!(n.get("wait_ms_max").is_some(), "{cap} 应有 wait_ms_max");
    }
}

/// 排队等待被记账：制造一次真实等待后 wait_events ≥ 1。
#[tokio::test]
async fn waiting_is_counted() {
    let g = gates(1);
    let gate = g.asr();
    let p1 = gate.acquire().await.expect("先占住许可");
    // 让第二路真实排队一小段时间再释放
    let gate2 = gate.clone();
    let waiter = tokio::spawn(async move {
        let _p = gate2.acquire().await.expect("释放后应获得");
    });
    tokio::time::sleep(Duration::from_millis(60)).await;
    drop(p1);
    waiter.await.unwrap();

    let j = g.stats_json();
    let asr = &j["inference_gates"]["asr"];
    let events = asr["wait_events"].as_u64().expect("应为整数");
    assert!(events >= 1, "排队事件必须被记账，实测 {events}");
}
