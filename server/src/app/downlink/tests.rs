//! `server/src/app/downlink.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::pad_tail;
use super::poll_abort;
use super::DownlinkPacer;
use super::PacerStats;
use crate::app::transport::{IncomingFrame, Transport};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 纯模拟下发（无 sleep）：`arrivals[i]` = 第 i 帧数据可就绪的时刻（毫秒，相对基准）。
/// 严格对齐 `pace` 语义：`now = max(到达, 上一帧**实际下发**时刻)`，等到目标时刻才下发
/// （已过则立即），**下发之后**才 `mark_sent`。返回 (每帧等待毫秒, 本轮观测统计)。
fn simulate_pacing(lead_ms: u64, frame_ms: u64, arrivals: &[u64]) -> (Vec<u64>, PacerStats) {
    let base = Instant::now();
    let mut p = DownlinkPacer::new(lead_ms);
    let mut send_at = 0u64;
    let waits = arrivals
        .iter()
        .map(|&a| {
            let now = a.max(send_at);
            let wait = p
                .plan(base + Duration::from_millis(now), frame_ms)
                .as_millis() as u64;
            send_at = now + wait;
            p.mark_sent(base + Duration::from_millis(send_at));
            wait
        })
        .collect();
    (waits, p.stats())
}

/// 节拍器核心语义（零提前量＝贴实时）：首帧零等待 → 每帧等一个帧长（**不重锚定、不突发**）。
/// ⚠️ 必须按 `pace` 语义模拟（下发**之后**才 `mark_sent`）：若在下发之前记录，贴实时时
/// 每帧都会把睡眠时长算成真空从而反复重锚定 → 节流失效。
#[test]
fn pacer_keeps_real_time_cadence() {
    let (waits, stats) = simulate_pacing(0, 60, &[0; 8]);
    assert_eq!(waits[0], 0, "首帧零等待：{waits:?}");
    assert!(
        waits[1..].iter().all(|&w| w == 60),
        "贴实时应严格每 60ms 一帧（重锚定/突发都会打乱节奏）：{waits:?}"
    );
    assert_eq!(stats.vacuum_events, 0, "匀速下发不得被误判为真空");
}

/// 真实计时（async 走 `pace` 的 sleep 路径）：帧按帧长铺开而非瞬时灌入。
/// 覆盖贴实时（lead=0）——这是"阈值 + 记录时机"两处改动的回归防线。
#[tokio::test]
async fn pacer_actually_sleeps_real_time() {
    let mut p = DownlinkPacer::new(0);
    let t0 = Instant::now();
    for _ in 0..6 {
        p.pace(40).await;
    }
    let elapsed = t0.elapsed();
    assert!(
        elapsed >= Duration::from_millis(190),
        "6 帧应按 40ms 节奏铺开（实测 {elapsed:?}）；瞬时灌入会撑爆 ESP 20 包解码队列"
    );
    assert_eq!(p.stats().vacuum_events, 0, "匀速下发不得被误判为真空");
}

/// 真实路径的预充语义：开局突发预充帧（≈lead/帧长 + 1），随后回到帧长匀速。
#[tokio::test]
async fn pacer_real_path_pre_fills_then_throttles() {
    let mut p = DownlinkPacer::new(240); // 预充 4~5 帧
    let t0 = Instant::now();
    for _ in 0..13 {
        p.pace(60).await;
    }
    let elapsed = t0.elapsed();
    // 13 帧 ≈ 预充 5 帧 + 8 帧 × 60ms ≈ 480ms；节流失效会瞬间跑完
    assert!(
        elapsed >= Duration::from_millis(400),
        "预充后必须回到实时节奏（13 帧实测 {elapsed:?}）"
    );
    assert_eq!(p.stats().vacuum_events, 0, "匀速下发不得被误判为真空");
}

/// 长真空（句间 LLM 停顿 / 下一句合成没跟上）**不得**把整段积压一次性突发回补：
/// 真空 ≥ max(lead, 帧长) 即重锚定，一次性补发量被限制在"提前量 + 2 帧"以内。
/// 这是长回复断音/断断续续的直接防线——旧实现只判定"同句内 recv 等待"，句间真空漏判，
/// 5 秒停顿会以 ~83 帧（≈5s 音频）的突发补发，以在线合成 ~7 倍速灌入必然撑爆
/// ESP 固件 20 包（≈1.2s）解码队列 → 队满静默丢包 → 听感"断断续续"。
#[test]
fn pacer_bounds_catch_up_burst_after_long_vacuum() {
    let (lead, frame) = (240u64, 60u64);
    let mut arrivals = vec![0u64; 5]; // 开局数据就绪：预充
    arrivals.extend(std::iter::repeat_n(5_000u64, 60)); // 5 秒真空后数据就绪
    let (waits, stats) = simulate_pacing(lead, frame, &arrivals);
    let burst = waits[5..].iter().take_while(|&&w| w == 0).count() as u64;
    assert!(
        burst <= lead / frame + 2,
        "真空后补发帧数应受提前量约束（≤{} 帧），实测 {burst} 帧",
        lead / frame + 2
    );
    assert_eq!(
        waits[(5 + burst) as usize],
        frame,
        "补发结束后必须立刻回到实时节奏，实测 {waits:?}"
    );
    // 5 秒断供必然打穿设备缓冲：真空 + 队列余量两项都要有记录
    assert_eq!(stats.vacuum_events, 1, "{stats:?}");
    assert!(stats.max_vacuum_ms >= 5_000, "{stats:?}");
    assert!(
        stats.queue_min_ms.unwrap_or(0) < 0,
        "应观测到队列见底：{stats:?}"
    );
}

/// 提前量（抖动缓冲）语义：开局连续零等待预充约 lead/帧长 + 1 帧，随后回到匀速但
/// 时间表整体比实时早 lead_ms；短真空（< lead）不重锚定（提前量就是为吸收它而设）。
#[test]
fn pacer_lead_pre_fills_and_absorbs_jitter() {
    // 数据全程就绪：前 5 帧零等待（预充 240ms + 首帧），随后 60ms 匀速
    let (waits, stats) = simulate_pacing(240, 60, &[0; 16]);
    assert!(
        waits[..5].iter().all(|&w| w == 0),
        "开局应突发预充 4~5 帧，实测 {waits:?}"
    );
    assert!(
        waits[5..].iter().all(|&w| w == 60),
        "预充完成后应恢复 60ms 匀速，实测 {waits:?}"
    );
    assert_eq!(
        stats.queue_min_ms.unwrap_or(0),
        300,
        "匀速段余量应稳定在 lead+帧长：{stats:?}"
    );
    // 100ms 短真空（< lead 240）：不重锚定，时间表不动 → 只按差额立即补 1~2 帧，
    // 随后回到 60ms 匀速（提前量吸收掉了这次抖动）
    let mut arrivals = vec![0u64; 5];
    arrivals.extend(std::iter::repeat_n(100u64, 15));
    let (waits, stats) = simulate_pacing(240, 60, &arrivals);
    let caught_up = waits[5..].iter().take_while(|&&w| w < 60).count();
    assert!(
        caught_up <= 2,
        "短真空只应补回差额（1~2 帧），实测补 {caught_up} 帧：{waits:?}"
    );
    assert_eq!(
        waits[5 + caught_up],
        60,
        "短真空后回到 60ms 匀速：{waits:?}"
    );
    assert_eq!(
        stats.vacuum_events, 0,
        "100ms < 提前量：不构成真空：{stats:?}"
    );
}

/// 观测统计（每轮日志的现场证据）：
/// ① 匀速段不得计入真空/不得报"队列见底"（否则天天误报）；
/// ② 长真空既报真空次数又报队列见底；
/// ③ **持续抽干**（上游只是略慢于实时、单次间隔够不上阈值）必须被"队列余量"捕捉
///    ——这正是"真空次数 0 但长回复一直断续"的形态。
#[test]
fn pacer_reports_vacuum_and_queue_stats() {
    // ① 匀速：零真空、余量稳定
    let (_, stats) = simulate_pacing(240, 60, &[0; 20]);
    assert_eq!(stats.vacuum_events, 0, "{stats:?}");
    assert_eq!(stats.max_vacuum_ms, 0, "{stats:?}");
    assert!(
        stats.queue_min_ms.unwrap_or(0) >= 240,
        "匀速段不得报队列见底：{stats:?}"
    );

    // ③ 生产者每帧晚 120ms 才给数据（单次间隔 < 提前量 240 → 不重锚定、不报真空），
    //    但音频只够实时的一半 → 队列被持续抽干（净亏 60ms/帧）
    let mut arrivals: Vec<u64> = vec![0; 5];
    arrivals.extend((1..=20u64).map(|k| 120 * k));
    let (_, stats) = simulate_pacing(240, 60, &arrivals);
    assert_eq!(stats.vacuum_events, 0, "单次间隔够不上阈值：{stats:?}");
    assert!(
        stats.queue_min_ms.unwrap_or(0) < 0,
        "持续慢供数必须被队列余量捕捉：{stats:?}"
    );

    // 统计在新一轮回复（`reset`）时清空
    let mut fresh = DownlinkPacer::new(240);
    let base = Instant::now();
    let _ = fresh.plan(base, 60);
    fresh.mark_sent(base);
    assert!(fresh.stats().queue_min_ms.is_none(), "首帧尚未测量");
    fresh.reset();
    let s = fresh.stats();
    assert_eq!(
        (s.vacuum_events, s.max_vacuum_ms, s.queue_min_ms),
        (0, 0, None)
    );
}

/// 开局预充的**包数**必须低于设备队列容量——队列是 **N 包**（容量 = N × 帧长），
/// 不是固定毫秒。这是 `AudioConfig::max_lead_ms` 的护栏口径：按固定 1200ms 算，
/// 40ms 帧下预充 31 包 > 20 包 → 固件静默丢弃 → "回复开头少一截"。
#[test]
fn prefill_packet_budget_stays_within_firmware_queue() {
    let queue = crate::config::FIRMWARE_DECODE_QUEUE_PACKETS as u64;
    for frame in [20u64, 40, 60] {
        let cap = (queue - 2) * frame; // 安全上限 = (队列 − 2) 包
        let prefill = cap.div_ceil(frame) + 1; // ceil(lead/帧长) + 1
        assert!(
            prefill < queue,
            "帧长 {frame}ms：预充 {prefill} 包 ≥ 队列 {queue} 包（会静默丢包）"
        );
    }
    // 默认提前量（生产帧长 60ms）也必须安全
    let lead = super::DEFAULT_LEAD_MS;
    assert!(
        lead.div_ceil(60) + 1 < queue,
        "默认 lead={lead}ms 的预充超队"
    );
    // 反例（旧口径）：固定按 1200ms 当容量，40ms 帧下必然溢出
    assert!(
        1200u64.div_ceil(40) + 1 > queue,
        "旧口径确实超队列，说明按包数算才正确"
    );
}

/// 默认构造 = 内置默认提前量（1000ms），与 `[audio].downlink_lead_ms` 默认一致。
#[test]
fn pacer_default_uses_default_lead() {
    let p = DownlinkPacer::default();
    assert_eq!(p.lead_ms, super::DEFAULT_LEAD_MS);
    // 默认提前量必须给固件的 20 包解码队列留足余量：开局预充 ceil(lead/帧长)+1 包，
    // 安全上限 = (20 − 2) × 60ms = 1080ms（队列是 **包**，不是固定毫秒）。
    let lead = super::DEFAULT_LEAD_MS;
    let cap = (crate::config::FIRMWARE_DECODE_QUEUE_PACKETS as u64 - 2) * 60;
    assert!(lead <= cap, "默认提前量 {lead}ms 应低于安全上限 {cap}ms");
}

/// 内存传输：预置上行帧序列，供 poll_abort 无网络单测。
struct SeqTransport {
    frames: Vec<IncomingFrame>,
}

impl Transport for SeqTransport {
    async fn send_text(&mut self, _json: String) -> anyhow::Result<()> {
        Ok(())
    }
    async fn send_binary(&mut self, _data: Vec<u8>) -> anyhow::Result<()> {
        Ok(())
    }
    async fn recv(&mut self) -> IncomingFrame {
        if self.frames.is_empty() {
            IncomingFrame::Closed
        } else {
            self.frames.remove(0)
        }
    }
    fn try_recv(&mut self) -> Option<IncomingFrame> {
        (!self.frames.is_empty()).then(|| self.frames.remove(0))
    }
    async fn keepalive(&mut self) {}
}

fn transport_with(frames: Vec<IncomingFrame>) -> SeqTransport {
    SeqTransport { frames }
}

fn abort_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[test]
fn poll_abort_triggers_on_abort_text() {
    let mut t = transport_with(vec![IncomingFrame::Text(r#"{"type":"abort"}"#.to_string())]);
    let flag = abort_flag();
    assert!(poll_abort(&mut t, "s1", &flag));
}

#[test]
fn poll_abort_ignores_non_abort_text_and_binary() {
    let mut t = transport_with(vec![
        IncomingFrame::Text(r#"{"type":"listen","state":"start"}"#.to_string()),
        IncomingFrame::Binary(vec![0u8; 8]),
    ]);
    let flag = abort_flag();
    assert!(!poll_abort(&mut t, "s1", &flag));
}

#[test]
fn poll_abort_treats_closed_as_interrupt() {
    let mut t = transport_with(vec![IncomingFrame::Closed]);
    let flag = abort_flag();
    assert!(poll_abort(&mut t, "s1", &flag));
}

#[test]
fn poll_abort_empty_is_not_interrupt() {
    let mut t = transport_with(vec![]);
    let flag = abort_flag();
    assert!(!poll_abort(&mut t, "s1", &flag));
}

#[test]
fn pad_tail_pads_partial_frame() {
    // 24k/60ms = 1440 样本/帧；60 残帧必须补零到整帧，否则 Opus 编码 BadArgument
    let pcm = vec![0.5f32; 60];
    let tail = pad_tail(&pcm, 1440);
    assert_eq!(tail.len(), 1440, "残帧必须补零到整帧");
    assert_eq!(tail[0], 0.5);
    assert_eq!(tail[59], 0.5);
    assert_eq!(tail[60], 0.0);
    assert_eq!(tail[1439], 0.0);
}

#[test]
fn pad_tail_returns_empty_when_full() {
    // 整帧或更长：无需补零（下行热路径直接 chunks 遍历，不分配）
    let pcm = vec![1.0f32; 2880];
    assert!(pad_tail(&pcm, 1440).is_empty());
    let pcm2 = vec![1.0f32; 1441];
    assert!(pad_tail(&pcm2, 1440).is_empty());
}

#[test]
fn pad_tail_short_input() {
    // 短于一帧：补成一帧
    let pcm = vec![0.25f32; 100];
    let tail = pad_tail(&pcm, 1440);
    assert_eq!(tail.len(), 1440);
    assert_eq!(tail[99], 0.25);
    assert_eq!(tail[100], 0.0);
}

#[test]
fn pad_tail_empty() {
    assert!(pad_tail(&[], 1440).is_empty());
}
