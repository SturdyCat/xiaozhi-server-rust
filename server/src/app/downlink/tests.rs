//! `server/src/app/downlink.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

use super::pad_tail;
use super::poll_abort;
use super::send_cached_sentence_audio;
use super::DownlinkPacer;
use crate::app::transport::{IncomingFrame, Transport};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// 节拍器核心语义（纯 plan 计算，无 sleep，零提前量）：
/// 首帧零等待 → 按绝对时间表匀速（不累积漂移）→ 迟到零等待且时间表不倒退。
#[test]
fn pacer_keeps_real_time_cadence() {
    let mut p = DownlinkPacer::new(0);
    let base = Instant::now();
    // 首帧：锚点=now，目标=now → 零等待
    assert_eq!(p.plan(base, 60), Duration::ZERO);
    // 第二帧在目标前 50ms：等到 base+60
    assert_eq!(p.plan(base + Duration::from_millis(10), 60), Duration::from_millis(50));
    // 第三帧恰在目标时刻：零等待
    assert_eq!(p.plan(base + Duration::from_millis(120), 60), Duration::ZERO);
    // 迟到（如生产者慢）：零等待，且后续目标仍锚定原时间表（不倒退、不突发回补）
    assert_eq!(p.plan(base + Duration::from_millis(500), 60), Duration::ZERO);
    // 第四帧目标仍是 base+240（绝对时间表），base+500 已过 → 继续零等待
    assert_eq!(p.plan(base + Duration::from_millis(501), 60), Duration::ZERO);
}

/// 真实计时（async 走 sleep 路径）：N 帧按帧长铺开而非瞬时灌入。
/// 3 帧 × 40ms：首帧零等待，总耗时 ≥ 2×40ms（守护 ESP 解码队列的核心行为）。
#[tokio::test]
async fn pacer_actually_sleeps_real_time() {
    let mut p = DownlinkPacer::new(0);
    let t0 = Instant::now();
    for _ in 0..3 {
        p.pace(40).await;
    }
    let elapsed = t0.elapsed();
    assert!(
        elapsed >= Duration::from_millis(75),
        "3 帧应按 40ms 节奏铺开（实测 {elapsed:?}）；瞬时灌入会撑爆 ESP 20 包解码队列"
    );
}

/// 数据源真空 ≥ max(lead, 帧长) 后重锚定：新帧立即下发（播放已追平），后续恢复 60ms 节奏。
/// 小于阈值的短真空不得重锚定（预充的抖动缓冲就是为吸收它而设）。
#[test]
fn pacer_reanchors_after_producer_gap() {
    let mut p = DownlinkPacer::new(0);
    let base = Instant::now();
    assert_eq!(p.plan(base, 60), Duration::ZERO); // 第 1 帧
    // 真空 5 秒（recv 等待）→ 重锚定
    p.note_producer_gap(Duration::from_millis(5000), 60);
    // 重锚定后：目标 = now - pos + pos = now → 零等待（不把积压帧突发回补）
    assert_eq!(p.plan(Instant::now(), 60), Duration::ZERO);
    // 下一帧恢复匀速（目标 ≈ 60ms 后）
    let wait = p.plan(Instant::now(), 60);
    assert!(wait >= Duration::from_millis(58) && wait <= Duration::from_millis(60), "恢复节奏: {wait:?}");
    // 小于阈值的短真空：不重锚定（时间表保持原样——第 3 帧目标仍是 base+120）
    let mut q = DownlinkPacer::new(0);
    assert_eq!(q.plan(base, 60), Duration::ZERO); // 第 1 帧（目标 base+0）
    assert_eq!(q.plan(base + Duration::from_millis(60), 60), Duration::ZERO); // 第 2 帧（目标 base+60）
    q.note_producer_gap(Duration::from_millis(10), 60); // 10ms < 60ms：不锚定
    assert_eq!(
        q.plan(base + Duration::from_millis(60), 60),
        Duration::from_millis(60),
        "短真空不得改变时间表（第 3 帧目标仍为 base+120）"
    );
    // reset 后回到首帧语义
    q.reset();
    assert_eq!(q.plan(base, 60), Duration::ZERO);
}

/// 提前量（抖动缓冲）语义：开局连续零等待预充 lead/帧长 帧，随后回到匀速但
/// 时间表整体比实时早 lead_ms；短真空（< lead）不重锚定。
#[test]
fn pacer_lead_pre_fills_and_absorbs_jitter() {
    let mut p = DownlinkPacer::new(240); // 4 帧 @60ms
    let base = Instant::now();
    // 开局预充：前 4 帧目标 ≤ base+0 → 全部零等待（突发下发，填设备解码队列）
    for _ in 0..4 {
        assert_eq!(p.plan(base, 60), Duration::ZERO, "预充帧应零等待");
    }
    // 第 5 帧目标 = base + 4*60 - 240 = base → 仍零等待；第 6 帧起恢复 60ms 匀速
    assert_eq!(p.plan(base + Duration::from_millis(5), 60), Duration::ZERO);
    let wait = p.plan(base + Duration::from_millis(10), 60);
    assert_eq!(wait, Duration::from_millis(50), "预充完成后恢复匀速（目标 base+300-240=base+60）");
    // 100ms 短真空（< lead 240）：设备仍有存货 → 不重锚定，时间表不动
    // （预充使时间表整体领先 wall clock，第 6 帧目标 = base+360-240 = base+120）
    p.note_producer_gap(Duration::from_millis(100), 60);
    let wait = p.plan(base + Duration::from_millis(110), 60);
    assert_eq!(wait, Duration::from_millis(10), "短真空不重锚定：目标仍 base+360-240=base+120");
    // ≥ lead 的真空：确已打穿 → 重锚定，下一帧立即下发
    p.note_producer_gap(Duration::from_millis(500), 60);
    assert_eq!(p.plan(Instant::now(), 60), Duration::ZERO);
}

/// 默认构造 = 内置默认提前量（240ms），与 `[audio].downlink_lead_ms` 默认一致。
#[test]
fn pacer_default_uses_default_lead() {
    let p = DownlinkPacer::default();
    assert_eq!(p.lead_ms, super::DEFAULT_LEAD_MS);
}

/// 缓存命中路径：逐帧加协议头按节拍下发，返回帧数；打断（承载关闭）提前返回。
#[tokio::test]
async fn cached_send_sends_all_frames_then_aborts_on_closed() {
    use crate::app::protocol::BinVersion;
    use crate::app::session::SessionParams;
    let params = SessionParams {
        uplink_bin_ver: BinVersion::V1,
        downlink_bin_ver: BinVersion::V1,
        uplink_sr: 16000,
        downlink_sr: 24000,
        downlink_frame_ms: 60,
        downlink_lead_ms: 0,
    };
    let frames = Arc::new(vec![vec![1u8, 2], vec![3], vec![4, 5, 6]]);
    let mut t = transport_with(vec![]);
    let mut ts = 100u32;
    let mut pacer = DownlinkPacer::new(0);
    let mut first = None;
    let n = send_cached_sentence_audio(
        &mut t,
        &params,
        &mut ts,
        &abort_flag(),
        "s1",
        "你好",
        &frames,
        &mut pacer,
        &mut first,
    )
    .await
    .expect("缓存下发应成功");
    assert_eq!(n, 3, "应下发全部缓存帧");
    assert_eq!(ts, 100 + 3 * 60, "时间戳按帧长推进");
    assert!(first.is_some(), "首帧时刻应记录");
    // 承载已关闭 → 打断：一帧都不发（sentence_start 仍先发）
    let mut t2 = transport_with(vec![IncomingFrame::Closed]);
    let mut ts2 = 0u32;
    let mut first2 = None;
    let n2 = send_cached_sentence_audio(
        &mut t2,
        &params,
        &mut ts2,
        &abort_flag(),
        "s1",
        "你好",
        &frames,
        &mut pacer,
        &mut first2,
    )
    .await
    .expect("缓存下发应成功");
    assert_eq!(n2, 1, "首帧发出后即检测到关闭并中止");
    assert_eq!(ts2, 60);
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
    let mut t = transport_with(vec![IncomingFrame::Text(
        r#"{"type":"abort"}"#.to_string(),
    )]);
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
