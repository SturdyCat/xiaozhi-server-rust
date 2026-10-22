//! `tts_test` 合成超时竞速（从 `bench.rs` 拆出：bench.rs 行数护栏 + 职责独立）。
//!
//! 预算只约束"合成出结果"，**不含**音频下发播放（排空按真实节拍走，长文本合法地
//! 超过 30s）；与 AIUI 的 ActiveDeadline 同一哲学：不惩罚正常耗时，只防"永挂"。
//! 超时即取消合成任务并向测试台回报结构化错误帧（不再让测试台 UI 无限等待）。

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// `tts_test` 合成链总预算：覆盖**引擎热切换 + 推理闸门排队 + 流式合成出结果**。
pub(super) const TTS_TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// 超时错误文案（测试台对话框直接可见；括号里带预算秒数便于与服务端日志对账）。
pub(super) fn timeout_error_text() -> String {
    format!(
        "合成超时({}s)：合成链路未在预算内完成，已取消本次合成",
        TTS_TEST_TIMEOUT.as_secs()
    )
}

/// 合成竞速结果（见 [`race_synth`]；独立 enum 便于外置测试直驱）。
#[derive(Debug)]
pub(super) enum SynthRace {
    /// 合成 producer 先完成（JoinError 已并入 Err，调用方按合成失败回报）。
    Producer(anyhow::Result<()>),
    /// 下发（drain）先完成——音频已按节播完，剩余收尾由调用方 join producer。
    Drained,
    /// 超时：`cancelled` 已置位（合成回调链收到 false → producer 闭包退出 →
    /// 推理闸门 permit 随 RAII 归还，不泄漏）。
    TimedOut,
}

/// 合成 producer / 音频下发 / 超时三者竞速。
///
/// 超时分支**先置位 `cancelled`** 再返回 [`SynthRace::TimedOut`]，保证 producer
/// 的下一个分片回调拿到 `false` 提前终止；producer 与 drain 必须同时被 poll
/// （下发负责消费水库，停 poll 会让下载缓冲顶死、合成假死）。
pub(super) async fn race_synth<D, F>(
    producer: &mut tokio::task::JoinHandle<anyhow::Result<()>>,
    mut drain: F,
    cancelled: &AtomicBool,
    deadline: tokio::time::Instant,
) -> (SynthRace, Option<anyhow::Result<D>>)
where
    F: Future<Output = anyhow::Result<D>> + Unpin,
{
    tokio::select! {
        r = producer => (
            SynthRace::Producer(r.unwrap_or_else(|e| Err(anyhow::anyhow!("合成任务异常: {e}")))),
            None,
        ),
        d = &mut drain => (SynthRace::Drained, Some(d)),
        _ = tokio::time::sleep_until(deadline) => {
            cancelled.store(true, Ordering::Relaxed);
            (SynthRace::TimedOut, None)
        }
    }
}
