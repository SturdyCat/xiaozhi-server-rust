//! `server/src/app/session/bench.rs` 的单元测试。
//!
//! 只测 `tts_test` **超时特性**的可独立驱动部分（`race_synth` 竞速 + 错误帧形状）：
//! `handle_tts_test` 依赖真实 TTS 引擎与协商会话（`Session` 需真实模型构造，
//! 同 `mcp/tests.rs` 的口径），端到端超时行为由联调覆盖。测试用极小 deadline 驱动，
//! 不依赖墙钟时长。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::*;

/// 小超时预算：测试用（毫秒级），不拖慢全量测试。
const TEST_DEADLINE_MS: u64 = 50;

fn deadline_after(ms: u64) -> tokio::time::Instant {
    tokio::time::Instant::now() + Duration::from_millis(ms)
}

#[tokio::test]
async fn 合成挂死时竞速返回超时并置位取消标志() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancelled_c = cancelled.clone();
    // 模拟网络挂死：只有 cancelled 置位才退出（与真实 push_pcm 取消路径同构）
    let mut producer = tokio::task::spawn(async move {
        while !cancelled_c.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        Ok::<(), anyhow::Error>(())
    });
    let drain = futures_util::future::pending::<anyhow::Result<()>>();
    let (race, drained) =
        race_synth(&mut producer, drain, &cancelled, deadline_after(TEST_DEADLINE_MS)).await;
    assert!(matches!(race, SynthRace::TimedOut), "应判超时，实际 {race:?}");
    assert!(drained.is_none());
    assert!(cancelled.load(Ordering::Relaxed), "超时必须置位取消标志");
    // 取消标志置位后 producer 应能自行退出（permit 随闭包归还，不泄漏）
    producer.await.unwrap().unwrap();
}

#[tokio::test]
async fn 合成先完成时竞速返回producer结果() {
    let cancelled = AtomicBool::new(false);
    let mut producer = tokio::task::spawn(async { Ok::<(), anyhow::Error>(()) });
    let mut drain = Box::pin(async {
        tokio::time::sleep(Duration::from_millis(TEST_DEADLINE_MS)).await;
        Ok(())
    });
    let (race, drained) =
        race_synth(&mut producer, &mut drain, &cancelled, deadline_after(10_000)).await;
    match race {
        SynthRace::Producer(Ok(())) => {}
        other => panic!("应返回 Producer(Ok)，实际 {other:?}"),
    }
    assert!(drained.is_none());
    assert!(!cancelled.load(Ordering::Relaxed));
}

#[tokio::test]
async fn 下发先完成时竞速返回drained并携带结果() {
    let cancelled = AtomicBool::new(false);
    // producer 慢于 drain：drain 完成即返回，调用方稍后自行 join producer
    let mut producer = tokio::task::spawn(async {
        tokio::time::sleep(Duration::from_millis(TEST_DEADLINE_MS)).await;
        Ok::<(), anyhow::Error>(())
    });
    let mut drain = Box::pin(async { Ok(7usize) });
    let (race, drained) =
        race_synth(&mut producer, &mut drain, &cancelled, deadline_after(10_000)).await;
    assert!(matches!(race, SynthRace::Drained), "应判 Drained，实际 {race:?}");
    let value = drained.expect("Drained 必带结果").expect("下发未报错");
    assert_eq!(value, 7);
    assert!(!cancelled.load(Ordering::Relaxed));
    let _ = producer.await.unwrap();
}

#[tokio::test]
async fn 超时错误文案带预算秒数() {
    let text = timeout_error_text();
    assert!(text.contains("合成超时"), "文案应含「合成超时」：{text}");
    assert!(text.contains("30s"), "文案应带预算秒数：{text}");
}

#[tokio::test]
async fn 超时错误帧形状符合tts_test协议() {
    // 超时回报用与引擎失败同一条结构化通道：tts_test + state=error + 可读 text
    let msg = ServerMessage::TtsTestResult {
        session_id: "s1".to_string(),
        state: "error".to_string(),
        engine: Some("xfyun".to_string()),
        text: Some(timeout_error_text()),
    };
    let json: serde_json::Value =
        serde_json::from_str(&msg.to_json()).expect("tts_test 帧是合法 JSON");
    assert_eq!(json["type"], "tts_test");
    assert_eq!(json["state"], "error");
    assert_eq!(json["engine"], "xfyun");
    assert!(
        json["text"].as_str().unwrap().contains("合成超时"),
        "text 应可读地写明超时: {}",
        json["text"]
    );
}
