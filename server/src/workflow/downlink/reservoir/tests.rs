//! `server/src/app/downlink/reservoir.rs`（下行音频水库）的单元测试。
//!
//! 未启用 `sherpa` 时 Opus 编码器是占位实现（返回空帧），因此断言只看**顺序、帧数、
//! 水位与背压**，不看负载字节。

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use super::*;
use crate::media::media::{AudioCodec, AudioFormat};
use crate::media::hot_audio::{HotAudioKey, MAX_ENTRY_BYTES};
use crate::workflow::pipeline::prefetch::{PcmChunk, PreparedAudio};
use crate::workflow::protocol::BinVersion;
use crate::workflow::session::SessionParams;
use crate::workflow::pipeline::tts_cache::TtsCache;

/// 测试用下行档位（与 `params()` 的 24k/60ms 一致；只影响内存键）。
fn fmt() -> AudioFormat {
    AudioFormat::new(AudioCodec::Opus, 24_000, 60)
}

fn mem(k: &HotAudioKey) -> u64 {
    k.mem_key(fmt())
}

fn params() -> SessionParams {
    SessionParams {
        output_codec: crate::media::media::AudioCodec::Opus,
        uplink_bin_ver: BinVersion::V1,
        downlink_bin_ver: BinVersion::V1,
        uplink_sr: 16000,
        downlink_sr: 24000,
        downlink_frame_ms: 60,
        downlink_lead_ms: 0,
        downlink_buffer_ms: 600,
        download_buffer_ms: 600,
        downlink_buffer_start_ms: 0,
        downlink_buffer_start_max_ms: 0,
    }
}

fn abort_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

/// 顺序契约：句首通知先于同句音频帧；水位只按**帧**计（句首通知不占水位）。
#[tokio::test]
async fn push_preserves_order_and_tracks_depth() {
    let (sink, mut source) = new_reservoir(600, 60); // 容量 10 帧
    assert!(
        sink.push(DownlinkItem::SentenceStart("你好".to_string()))
            .await
    );
    assert!(sink.push(DownlinkItem::Frame(vec![1])).await);
    assert!(sink.push(DownlinkItem::Frame(vec![2, 2])).await);
    assert_eq!(source.depth_frames(), 2, "句首通知不占帧水位");
    assert_eq!(source.depth_ms(), 120);

    assert!(matches!(source.recv().await, Some(DownlinkItem::SentenceStart(t)) if t == "你好"));
    assert!(matches!(source.recv().await, Some(DownlinkItem::Frame(f)) if f == vec![1]));
    assert_eq!(source.depth_frames(), 1);
    assert!(matches!(source.recv().await, Some(DownlinkItem::Frame(_))));
    assert_eq!(source.depth_frames(), 0, "抽空后水位归零");

    // 空且写端仍在 → 不该返回 None（发送端靠这个语义区分"暂时没数据"与"本轮结束"）
    assert!(
        tokio::time::timeout(Duration::from_millis(30), source.recv())
            .await
            .is_err(),
        "写端仍在时 recv 必须等待而不是返回 None"
    );
}

/// 收工语义：写端全部丢弃 → `producer_done()`；存货取完 → `recv()` 返回 None。
#[tokio::test]
async fn producer_done_closes_source_after_drain() {
    let (sink, mut source) = new_reservoir(600, 60);
    assert!(sink.push(DownlinkItem::Frame(vec![1])).await);
    assert!(!source.producer_done(), "写端仍在 = 生产端未收工");
    drop(sink);
    assert!(source.producer_done(), "写端全部丢弃 = 生产端收工");
    assert!(source.recv().await.is_some(), "已入队的存货仍要能取到");
    assert!(source.recv().await.is_none(), "抽空后 recv 返回 None");
}

/// 容量 = 帧数上限（另给每句一个句首通知槽位）：满则 `push` 等待（背压），
/// 读端取走一项后立刻放行；**被取消的 push 不得污染水位**。
#[tokio::test]
async fn full_reservoir_backpressures_producer() {
    let (sink, mut source) = new_reservoir(120, 60); // 容量 2 帧
    let slots = 2; // Extra channel slots are metadata, never extra audio duration.
    for i in 0..slots {
        assert!(
            sink.push(DownlinkItem::Frame(vec![i as u8])).await,
            "第 {i} 项应能入队（槽位 = 帧容量 + 流水线深度）"
        );
    }
    let depth_before = source.depth_frames();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(50),
            sink.push(DownlinkItem::Frame(vec![9]))
        )
        .await
        .is_err(),
        "水库满时 push 必须等待（这就是背压：不再无界合成/占内存）"
    );
    assert_eq!(
        source.depth_frames(),
        depth_before,
        "被取消的 push 必须回滚记账（否则水位虚高、起跑闸误判）"
    );
    // 读端取走一项 → 空间释放 → 入队成功
    assert!(source.recv().await.is_some());
    assert!(
        tokio::time::timeout(
            Duration::from_millis(200),
            sink.push(DownlinkItem::Frame(vec![9]))
        )
        .await
        .is_ok(),
        "腾出空间后 push 应立即成功"
    );
}

/// 填充任务：先推句首通知再推该句各帧；完整合成的句子写入结果缓存。
#[tokio::test]
async fn filler_pushes_sentence_start_first_and_caches_complete_sentence() {
    let cache = Arc::new(TtsCache::new(8));
    let (sink, mut source) = new_reservoir(600, 60);
    let (sentence_tx, sentence_rx) = mpsc::channel(1);
    let (chunk_tx, chunk_rx) = mpsc::channel(4);
    let (done_tx, done_rx) = oneshot::channel::<bool>();
    // 24k/60ms → 1440 样本/帧；2880 样本 = 2 整帧
    chunk_tx
        .send(PcmChunk::unbudgeted(vec![0.1f32; 2880]))
        .await
        .expect("灌入 PCM");
    drop(chunk_tx);
    done_tx.send(true).expect("标记合成正常结束");
    sentence_tx
        .send(PreparedSentence {
            text: "你好".to_string(),
            audio: PreparedAudio::Stream {
                sr: 24000,
                chunks: chunk_rx,
                done: done_rx,
            },
            cache_key: 7,
        })
        .await
        .expect("投递成品");
    drop(sentence_tx);

    let filler = spawn_reservoir_filler(abort_flag(), cache.clone(), params(), sentence_rx, sink);
    assert!(matches!(source.recv().await, Some(DownlinkItem::SentenceStart(t)) if t == "你好"));
    assert!(matches!(source.recv().await, Some(DownlinkItem::Frame(_))));
    assert!(matches!(source.recv().await, Some(DownlinkItem::Frame(_))));
    assert!(
        source.recv().await.is_none(),
        "句子流结束 → 抽空后 recv 返回 None"
    );
    filler.await.expect("填充任务正常收尾");

    let hit = cache.get(7).expect("完整句应写入结果缓存");
    assert_eq!(hit.opus_frames.len(), 2, "缓存应含整句 2 帧");
}

/// 合成未正常结束（`done=false`）的句子**不得**入缓存（残缺句会被反复复用）。
#[tokio::test]
async fn filler_skips_cache_for_incomplete_sentence() {
    let cache = Arc::new(TtsCache::new(8));
    let (sink, mut source) = new_reservoir(600, 60);
    let (sentence_tx, sentence_rx) = mpsc::channel(1);
    let (chunk_tx, chunk_rx) = mpsc::channel(4);
    let (done_tx, done_rx) = oneshot::channel::<bool>();
    chunk_tx
        .send(PcmChunk::unbudgeted(vec![0.1f32; 2880]))
        .await
        .expect("灌入 PCM");
    drop(chunk_tx);
    done_tx.send(false).expect("标记合成失败");
    sentence_tx
        .send(PreparedSentence {
            text: "你好".to_string(),
            audio: PreparedAudio::Stream {
                sr: 24000,
                chunks: chunk_rx,
                done: done_rx,
            },
            cache_key: 9,
        })
        .await
        .expect("投递成品");
    drop(sentence_tx);

    let filler = spawn_reservoir_filler(abort_flag(), cache.clone(), params(), sentence_rx, sink);
    while source.recv().await.is_some() {}
    filler.await.expect("填充任务正常收尾");
    assert!(cache.get(9).is_none(), "残缺句不得入缓存");
}

/// 缓存命中路径（已编码帧）：零合成直接入水库，顺序与流式路径一致。
#[tokio::test]
async fn filler_pushes_cached_frames_in_order() {
    let cache = Arc::new(TtsCache::new(8));
    let (sink, mut source) = new_reservoir(600, 60);
    let (sentence_tx, sentence_rx) = mpsc::channel(1);
    sentence_tx
        .send(PreparedSentence {
            text: "好的".to_string(),
            audio: PreparedAudio::Cached(Arc::new(vec![vec![1], vec![2], vec![3]])),
            cache_key: 0,
        })
        .await
        .expect("投递成品");
    drop(sentence_tx);

    let filler = spawn_reservoir_filler(abort_flag(), cache, params(), sentence_rx, sink);
    assert!(matches!(source.recv().await, Some(DownlinkItem::SentenceStart(t)) if t == "好的"));
    let mut frames = 0;
    while let Some(item) = source.recv().await {
        if let DownlinkItem::Frame(f) = item {
            frames += 1;
            assert_eq!(f, vec![frames as u8], "缓存帧顺序必须保持");
        }
    }
    assert_eq!(frames, 3);
    filler.await.expect("填充任务正常收尾");
}

/// 读端被丢弃（打断/本轮结束）→ 填充任务立刻收尾，不再白合成/白编码。
#[tokio::test]
async fn filler_exits_when_reader_dropped() {
    let cache = Arc::new(TtsCache::new(8));
    let (sink, source) = new_reservoir(600, 60);
    drop(source); // 发送端放弃
    let (chunk_tx, chunk_rx) = mpsc::channel(1);
    let (done_tx, done_rx) = oneshot::channel::<bool>();
    chunk_tx
        .try_send(PcmChunk::unbudgeted(vec![0.1f32; 2880]))
        .expect("预置分片");
    let _ = done_tx.send(true);
    let filler = spawn_single_sentence_filler(
        abort_flag(),
        cache,
        params(),
        sink,
        "你好".to_string(),
        PreparedAudio::Stream {
            sr: 24000,
            chunks: chunk_rx,
            done: done_rx,
        },
        0,
    );
    tokio::time::timeout(Duration::from_secs(1), filler)
        .await
        .expect("读端已丢弃，填充任务应立即收尾")
        .expect("填充任务正常收尾");
}

/// 发送端等空（水库见底）要计入"上游断供"观测：这是"该调大水库还是该治上游"的依据。
#[tokio::test]
async fn recv_records_starvation_when_empty() {
    let (sink, mut source) = new_reservoir(600, 60);
    let producer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(120)).await;
        sink.push(DownlinkItem::Frame(vec![1])).await;
        drop(sink);
    });
    assert!(source.recv().await.is_some());
    producer.await.expect("生产者正常结束");
    let stats = source.stats();
    assert!(stats.starve_ms >= 60, "等空应计入断供时长: {stats:?}");
    assert_eq!(stats.starve_events, 1, "等空超过一帧应计一次: {stats:?}");
    assert_eq!(stats.cap_ms, 600, "容量口径按帧长换算: {stats:?}");
}

/// 下载缓冲：水库填充任务每编码完一片就**归还额度**（上游据此恢复下载）。
///
/// 这条是"两段缓冲各按自己的口径吃满"的关键连接点：若填充任务不归还额度，
/// 上游下载会被虚高水位永久摁住——表现是"第一句之后就没声了"。
#[tokio::test]
async fn filler_returns_download_quota_per_chunk() {
    let budget = crate::workflow::pipeline::prefetch::DownloadBudget::new(8_000);
    let abort = abort_flag();
    let cache = Arc::new(TtsCache::new(8));
    let (sink, mut source) = new_reservoir(600, 60);
    let (sentence_tx, sentence_rx) = mpsc::channel(1);
    let (chunk_tx, chunk_rx) = mpsc::channel(4);
    let (done_tx, done_rx) = oneshot::channel::<bool>();
    // 两片各 100ms（24k 下 2400 样本）：先预留额度，再以"带额度分片"入队
    for _ in 0..2 {
        assert!(budget.reserve(100, &abort, &|| false));
        chunk_tx
            .send(PcmChunk::budgeted(vec![0.1f32; 2400], 100, budget.clone()))
            .await
            .expect("灌入 PCM");
    }
    assert_eq!(budget.queued_ms(), 200, "两片在缓冲里 → 水位 200ms");
    drop(chunk_tx);
    done_tx.send(true).expect("标记合成正常结束");
    sentence_tx
        .send(PreparedSentence {
            text: "你好".to_string(),
            audio: PreparedAudio::Stream {
                sr: 24000,
                chunks: chunk_rx,
                done: done_rx,
            },
            cache_key: 0,
        })
        .await
        .expect("投递成品");
    drop(sentence_tx);

    let filler = spawn_reservoir_filler(abort_flag(), cache.clone(), params(), sentence_rx, sink);
    while source.recv().await.is_some() {}
    filler.await.expect("填充任务正常收尾");
    assert_eq!(budget.queued_ms(), 0, "消费完分片后额度必须归零");
    assert!(
        cache.get(0).is_none(),
        "cache_key == 0 的流式句子不得入缓存"
    );
}

#[test]
fn cache_collection_is_bounded_without_stopping_streaming() {
    // 超限：任一上限（PCM 字节）触顶 → 整句放弃收集（帧照发，缓存不收）
    let mut c = SentenceCollector::new();
    c.on_frame(&[1]);
    c.on_chunk(&vec![0f32; MAX_ENTRY_BYTES / 2 + 1]);
    c.on_frame(&[2]);
    assert!(c.take().is_none(), "超限后整句不入缓存");

    // 正常路径：PCM 原样落盘（不重采样），帧与 PCM 都收集
    let mut c = SentenceCollector::new();
    c.on_chunk(&[0.1f32; 2400]);
    c.on_frame(&[1, 2]);
    let (frames, stored) = c.take().expect("正常句应收集");
    assert_eq!(frames.len(), 1);
    assert_eq!(
        stored.len(),
        2400 * 2,
        "原样落盘 → 样本数不变（S16LE 双字节）"
    );
}

#[tokio::test]
async fn abort_while_waiting_for_synthesis_result_never_caches() {
    use crate::media::hot_audio::{HotAudioKey, HotAudioStore};
    let dir = std::env::temp_dir().join(format!("xz-reservoir-{}", uuid::Uuid::new_v4()));
    let store = Arc::new(HotAudioStore::new(Some(dir.clone())));
    let cache = Arc::new(TtsCache::with_store(8, store.clone()));
    let key = HotAudioKey::from_session(&crate::config::Config::default(), "晚到打断", 24_000);
    cache.register_identity(key.clone(), mem(&key));
    let abort = abort_flag();
    let (sink, mut source) = new_reservoir(600, 60);
    let (chunk_tx, chunk_rx) = mpsc::channel(1);
    let (done_tx, done_rx) = oneshot::channel();
    chunk_tx
        .send(PcmChunk::unbudgeted(vec![0.1; 2880]))
        .await
        .unwrap();
    drop(chunk_tx);
    let filler = spawn_single_sentence_filler(
        abort.clone(),
        cache.clone(),
        params(),
        sink,
        "晚到打断".into(),
        PreparedAudio::Stream {
            sr: 24000,
            chunks: chunk_rx,
            done: done_rx,
        },
        mem(&key),
    );
    assert!(matches!(
        source.recv().await,
        Some(DownlinkItem::SentenceStart(_))
    ));
    for _ in 0..2 {
        assert!(matches!(source.recv().await, Some(DownlinkItem::Frame(_))));
    }
    abort.store(true, Ordering::Relaxed);
    done_tx.send(true).unwrap();
    filler.await.unwrap();
    assert!(cache.get(mem(&key)).is_none());
    assert!(store.get(&key).is_none());
    assert!(!dir.exists(), "晚到打断不应创建磁盘缓存");
}

/// 默认 feature 编码器返回空占位帧；完整持久化链路需要真实 Opus 编码。
#[cfg(feature = "sherpa")]
#[tokio::test]
async fn complete_production_sentence_persists_and_restarts() {
    use crate::media::hot_audio::{HotAudioKey, HotAudioStore};
    let dir = std::env::temp_dir().join(format!("xz-reservoir-{}", uuid::Uuid::new_v4()));
    let store = Arc::new(HotAudioStore::new(Some(dir.clone())));
    let cache = Arc::new(TtsCache::with_store(8, store.clone()));
    let key = HotAudioKey::from_session(&crate::config::Config::default(), "完整普通回答", 24_000);
    cache.register_identity(key.clone(), mem(&key));
    let (sink, mut source) = new_reservoir(600, 60);
    let (chunk_tx, chunk_rx) = mpsc::channel(1);
    let (done_tx, done_rx) = oneshot::channel();
    chunk_tx
        .send(PcmChunk::unbudgeted(vec![0.1; 2880]))
        .await
        .unwrap();
    drop(chunk_tx);
    done_tx.send(true).unwrap();
    let filler = spawn_single_sentence_filler(
        abort_flag(),
        cache.clone(),
        params(),
        sink,
        key.text.clone(),
        PreparedAudio::Stream {
            sr: 24000,
            chunks: chunk_rx,
            done: done_rx,
        },
        mem(&key),
    );
    while source.recv().await.is_some() {}
    filler.await.unwrap();
    drop(cache);
    let restarted = TtsCache::with_store(8, store);
    restarted.register_identity(key.clone(), mem(&key));
    let hit = restarted
        .get_async(mem(&key))
        .await
        .expect("完整普通句重启后命中");
    assert_eq!(hit.opus_frames.len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn failed_synthesis_never_creates_persistent_entry() {
    use crate::media::hot_audio::{HotAudioKey, HotAudioStore};
    let dir = std::env::temp_dir().join(format!("xz-reservoir-{}", uuid::Uuid::new_v4()));
    let store = Arc::new(HotAudioStore::new(Some(dir.clone())));
    let cache = Arc::new(TtsCache::with_store(8, store.clone()));
    let key = HotAudioKey::from_session(&crate::config::Config::default(), "失败句", 24_000);
    cache.register_identity(key.clone(), mem(&key));
    let (sink, mut source) = new_reservoir(600, 60);
    let (chunk_tx, chunk_rx) = mpsc::channel(1);
    let (done_tx, done_rx) = oneshot::channel();
    chunk_tx
        .send(PcmChunk::unbudgeted(vec![0.1; 2880]))
        .await
        .unwrap();
    drop(chunk_tx);
    done_tx.send(false).unwrap();
    let filler = spawn_single_sentence_filler(
        abort_flag(),
        cache.clone(),
        params(),
        sink,
        "失败句".into(),
        PreparedAudio::Stream {
            sr: 24000,
            chunks: chunk_rx,
            done: done_rx,
        },
        mem(&key),
    );
    while source.recv().await.is_some() {}
    filler.await.unwrap();
    assert!(cache.get(mem(&key)).is_none());
    assert!(store.get(&key).is_none());
    assert!(!dir.exists());
}

#[tokio::test]
async fn byte_budget_backpressures_independently_and_reader_drop_unblocks_waiters() {
    let (sink, mut source) = new_reservoir(6000, 60);
    assert!(
        sink.push(DownlinkItem::Frame(vec![0; MAX_RESERVOIR_BYTES]))
            .await
    );
    assert!(tokio::time::timeout(
        Duration::from_millis(5),
        sink.push(DownlinkItem::Frame(vec![1]))
    )
    .await
    .is_err());
    assert_eq!(source.depth_frames(), 1);
    drop(source.recv().await);
    assert!(sink.push(DownlinkItem::Frame(vec![1])).await);
    assert!(
        !sink
            .push(DownlinkItem::Frame(vec![0; MAX_RESERVOIR_BYTES + 1]))
            .await
    );
    drop(source);
    assert!(!sink.push(DownlinkItem::Frame(vec![2])).await);
}
