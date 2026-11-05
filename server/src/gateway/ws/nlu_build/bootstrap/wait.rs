//! 等**自己这一版**构建结束。
//!
//! 单独成文件是因为这里全是**竞态**：构建只有一个槽位，手动构建随时可能抢槽。
//! 判定写错不会有编译错误，只会把归因指错方向（把"结果不可观察"说成"训练超时"），
//! 而那种错要等 3700s 之后才暴露。

use std::time::Duration;

use super::super::{job, BuildJob};
use super::POLL_MS;

pub(super) struct BuildSnapshot {
    pub(super) state: &'static str,
    pub(super) error: String,
    pub(super) log_tail: String,
}

/// 等**自己这一版**构建结束。
///
/// 必须校验 `build_id`：轮询间隔 250ms，自举构建 A 结束到下一轮采样之间用户可能点了手动
/// 构建 B。只看 `!running` 会等到 B 结束，然后拿 **B 的日志**去解析冒烟用例、配 **A 的
/// artifact** 跑门禁——用例与模型错配，误通过/误失败都可能发生（QA 复核发现的真实竞态）。
pub(super) enum WaitOutcome {
    Done(BuildSnapshot),
    /// 槽位被别人接管：我们自己那一次的结果**已经无法观察**。
    ///
    /// 这种情况绝不能当成超时。A 训练结束（`running=false`）到下一次 250ms 采样之间，
    /// 用户点的手动构建 B 会覆盖 `build_id`；此后 `is_finished("A")` 永远不满足，
    /// 只能等满 `TRAIN_WAIT_SECS=3700s`，然后把"槽位被抢"误报成"训练超时"——
    /// 运维会去查一个早已结束的构建。也不要去读 A 的产物：A 成功与否未知，
    /// 半成品只会给出更难懂的错。
    TakenOver {
        by: String,
    },
}

/// 槽位是否已被**别人**接管（我们那一次的结果已不可观察）。
pub(super) fn taken_over(job: &BuildJob, build_id: &str) -> Option<String> {
    if is_finished(job, build_id) {
        return None;
    }
    match job.build_id.as_deref() {
        Some(other) if other != build_id => Some(other.to_string()),
        // 槽位被清空（进程重启/被重置）：同样拿不到自己那次的结果。
        None if !job.running => Some("（槽位已重置）".to_string()),
        _ => None,
    }
}

pub(super) async fn wait_build_done(build_id: &str) -> WaitOutcome {
    loop {
        {
            let j = job().lock().await;
            if is_finished(&j, build_id) {
                return WaitOutcome::Done(BuildSnapshot {
                    state: j.state,
                    error: j.error.clone(),
                    log_tail: j.log.clone(),
                });
            }
            if let Some(by) = taken_over(&j, build_id) {
                return WaitOutcome::TakenOver { by };
            }
        }
        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
    }
}

/// 这一版构建是否已经结束，且**确实是我们这一版**。
///
/// 单独抽成纯函数是为了能被单测钉住：只看 `!running` 的实现会在"自举 A 结束、用户同时
/// 点了手动构建 B"的窗口里把 B 的结果当成 A 的。
pub(super) fn is_finished(job: &BuildJob, build_id: &str) -> bool {
    !job.running && job.build_id.as_deref() == Some(build_id)
}
