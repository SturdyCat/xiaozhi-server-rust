//! `server/src/app/downlink/sender.rs`（下行发送端：水库 → 实时节拍）的单元测试。

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use crate::workflow::downlink::reservoir::DownlinkItem;
use crate::workflow::downlink::{new_reservoir, DownlinkPacer};
use crate::workflow::protocol::BinVersion;
use crate::workflow::protocol::{IncomingFrame, Transport};
use crate::workflow::session::SessionParams;

fn params(buffer_ms: u32, start_ms: u32, start_max_ms: u32) -> SessionParams {
    SessionParams {
        output_codec: crate::media::media::AudioCodec::Opus,
        uplink_bin_ver: BinVersion::V1,
        downlink_bin_ver: BinVersion::V1,
        uplink_sr: 16000,
        downlink_sr: 24000,
        downlink_frame_ms: 60,
        downlink_lead_ms: 0,
        downlink_buffer_ms: buffer_ms,
        download_buffer_ms: buffer_ms,
        downlink_buffer_start_ms: start_ms,
        downlink_buffer_start_max_ms: start_max_ms,
    }
}

fn abort_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

/// 记录下发序列的传输（协议顺序 / 帧数 / 打断断言用）。
#[derive(Default)]
struct RecordingTransport {
    sent: Vec<String>,
    frames: Vec<IncomingFrame>,
}

impl Transport for RecordingTransport {
    async fn send_text(&mut self, json: String) -> anyhow::Result<()> {
        self.sent.push(json);
        Ok(())
    }
    async fn send_binary(&mut self, data: Vec<u8>) -> anyhow::Result<()> {
        self.sent.push(format!("bin({})", data.len()));
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

/// 协议顺序契约：`tts start` → `sentence_start(text)`（**恰好一次**）→ 音频帧。
#[tokio::test]
async fn sentence_start_exactly_once_and_before_frames() {
    let (sink, source) = new_reservoir(600, 60);
    assert!(
        sink.push(DownlinkItem::SentenceStart("你好".to_string()))
            .await
    );
    assert!(sink.push(DownlinkItem::Frame(vec![1, 2])).await);
    assert!(sink.push(DownlinkItem::Frame(vec![3])).await);
    drop(sink);

    let mut t = RecordingTransport::default();
    let mut ts = 0u32;
    let mut pacer = DownlinkPacer::new(0);
    let mut first = None;
    let out = drain_reservoir(
        &mut t,
        &params(600, 0, 0),
        "s1",
        &abort_flag(),
        &mut ts,
        &mut pacer,
        source,
        Instant::now(),
        &mut first,
    )
    .await
    .expect("下发应成功");

    assert_eq!(out.sentences, 1, "一句");
    assert_eq!(out.frames, 2, "两帧");
    assert!(out.tts_started, "已下发 tts start");
    assert!(!out.interrupted);
    assert_eq!(ts, 120, "时间戳按帧长推进");
    assert!(first.is_some(), "首帧时刻应记录");
    assert!(
        t.sent[0].contains("\"start\""),
        "第一条必须是 tts start：{:?}",
        t.sent
    );
    assert!(
        t.sent[1].contains("sentence_start"),
        "sentence_start 必须先于音频帧：{:?}",
        t.sent
    );
    assert_eq!(
        t.sent
            .iter()
            .filter(|m| m.contains("sentence_start"))
            .count(),
        1,
        "sentence_start 恰好一次：{:?}",
        t.sent
    );
    assert_eq!(
        t.sent.iter().filter(|m| m.starts_with("bin(")).count(),
        2,
        "应下发 2 个二进制帧：{:?}",
        t.sent
    );
}

/// 节拍器仍然生效：贴实时（lead=0）时 N 帧耗时 ≈ (N−1)×帧长（**不得**退化成尽速灌入）。
#[tokio::test]
async fn drain_keeps_real_time_cadence() {
    let (sink, source) = new_reservoir(600, 60);
    for _ in 0..4 {
        assert!(sink.push(DownlinkItem::Frame(vec![1])).await);
    }
    drop(sink);

    let mut t = RecordingTransport::default();
    let mut ts = 0u32;
    let mut pacer = DownlinkPacer::new(0);
    let mut first = None;
    let t0 = Instant::now();
    let out = drain_reservoir(
        &mut t,
        &params(600, 0, 0),
        "s1",
        &abort_flag(),
        &mut ts,
        &mut pacer,
        source,
        t0,
        &mut first,
    )
    .await
    .expect("下发应成功");
    assert_eq!(out.frames, 4);
    assert!(
        t0.elapsed() >= Duration::from_millis(160),
        "4 帧应至少耗时 3×60ms（节流失效会快得多）：{:?}",
        t0.elapsed()
    );
}

/// 起跑闸：首帧之前先攒够 `downlink_buffer_start_ms`（这里是生产端慢速投喂，攒到才放行）。
#[tokio::test]
async fn start_gate_waits_for_buffer_before_first_frame() {
    let (sink, source) = new_reservoir(1000, 60); // 容量 1000ms
    tokio::spawn(async move {
        for i in 0..8u8 {
            if !sink.push(DownlinkItem::Frame(vec![i])).await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        drop(sink);
    });

    let mut t = RecordingTransport::default();
    let mut ts = 0u32;
    let mut pacer = DownlinkPacer::new(0);
    let mut first = None;
    let t0 = Instant::now();
    let out = drain_reservoir(
        &mut t,
        &params(1000, 300, 2000),
        "s1",
        &abort_flag(),
        &mut ts,
        &mut pacer,
        source,
        t0,
        &mut first,
    )
    .await
    .expect("下发应成功");

    let ttfa = first.expect("应下发首帧").duration_since(t0);
    assert!(
        ttfa >= Duration::from_millis(250),
        "起跑闸应攒够 ~300ms 才发首帧：TTFA={ttfa:?}"
    );
    assert!(
        out.reservoir.gate_ms >= 250,
        "起跑闸等待时长应被记录：{:?}",
        out.reservoir
    );
    assert!(
        out.reservoir.gate_depth_ms >= 300,
        "闸门结束时的水位应 ≥ 起跑闸：{:?}",
        out.reservoir
    );
}

/// 短回复（生产端已收工）不白等：闸门立刻放行，不把 TTFA 拖到起跑闸时长。
#[tokio::test]
async fn start_gate_skips_when_producer_already_done() {
    let (sink, source) = new_reservoir(1000, 60);
    assert!(sink.push(DownlinkItem::Frame(vec![1])).await);
    drop(sink);

    let mut t = RecordingTransport::default();
    let mut ts = 0u32;
    let mut pacer = DownlinkPacer::new(0);
    let mut first = None;
    let t0 = Instant::now();
    let out = drain_reservoir(
        &mut t,
        &params(1000, 2000, 4000),
        "s1",
        &abort_flag(),
        &mut ts,
        &mut pacer,
        source,
        t0,
        &mut first,
    )
    .await
    .expect("下发应成功");
    assert_eq!(out.frames, 1);
    assert!(
        t0.elapsed() < Duration::from_millis(500),
        "生产端已收工 → 起跑闸不应白等 2s：{:?}",
        t0.elapsed()
    );
    assert!(out.reservoir.gate_ms < 500, "{:?}", out.reservoir);
}

/// 打断（barge-in）：首帧发出后收到 abort → 立刻停发并标记 interrupted。
#[tokio::test]
async fn abort_during_drain_stops_sending() {
    let (sink, source) = new_reservoir(1000, 60);
    for _ in 0..5 {
        assert!(sink.push(DownlinkItem::Frame(vec![1])).await);
    }
    drop(sink);

    let mut t = RecordingTransport {
        sent: Vec::new(),
        frames: vec![IncomingFrame::Text(r#"{"type":"abort"}"#.to_string())],
    };
    let mut ts = 0u32;
    let mut pacer = DownlinkPacer::new(0);
    let mut first = None;
    let out = drain_reservoir(
        &mut t,
        &params(1000, 0, 0),
        "s1",
        &abort_flag(),
        &mut ts,
        &mut pacer,
        source,
        Instant::now(),
        &mut first,
    )
    .await
    .expect("下发应成功");
    assert!(out.interrupted, "应标记被打断");
    assert_eq!(out.frames, 1, "首帧发出后即检测到 abort，不再继续");
    assert_eq!(
        t.sent.iter().filter(|m| m.starts_with("bin(")).count(),
        1,
        "{:?}",
        t.sent
    );
}

/// **水库的核心承诺**：上游停顿（生产端一段时间不出数据）时，只要水库里有存货，
/// 发送端就继续按实时节奏供数——同一停顿在"无水库（逐帧直通）"下必然断供。
#[tokio::test]
async fn reservoir_absorbs_upstream_stall_that_direct_mode_cannot() {
    async fn run(buffer_ms: u32) -> ReservoirDrain {
        let (sink, source) = new_reservoir(buffer_ms, 60);
        tokio::spawn(async move {
            for _ in 0..20 {
                if !sink.push(DownlinkItem::Frame(vec![1])).await {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(700)).await; // 上游停顿
            for _ in 0..4 {
                if !sink.push(DownlinkItem::Frame(vec![1])).await {
                    return;
                }
            }
            drop(sink);
        });
        let mut t = RecordingTransport::default();
        let mut ts = 0u32;
        let mut pacer = DownlinkPacer::new(0);
        let mut first = None;
        drain_reservoir(
            &mut t,
            &params(buffer_ms, 0, 0),
            "s1",
            &abort_flag(),
            &mut ts,
            &mut pacer,
            source,
            Instant::now(),
            &mut first,
        )
        .await
        .expect("下发应成功")
    }

    let buffered = run(4000).await;
    assert_eq!(buffered.frames, 24, "全部帧都要下发");
    assert_eq!(
        buffered.reservoir.starve_events, 0,
        "水库里先备了 20 帧（1.2s），700ms 停顿应被完全吸收：{:?}",
        buffered.reservoir
    );

    let direct = run(0).await;
    assert_eq!(direct.frames, 24);
    assert!(
        direct.reservoir.starve_events >= 1,
        "无水库（逐帧直通）时同样的停顿必然断供：{:?}",
        direct.reservoir
    );
}
