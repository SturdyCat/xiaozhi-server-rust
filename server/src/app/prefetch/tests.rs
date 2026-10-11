//! [`crate::app::prefetch`] 的单元测试。
//!
//! 覆盖预取管线的**结构性收益**（这是长回复断音的根因修复，比计时数值更重要）：
//! ① 下一句的合成在上一句**播完之前**就已启动；② 合成仍严格串行；
//! ③ 打断即停止；④ 缓存命中零合成；⑤ 合成失败上报；⑥ 顺序不变；
//! ⑦ 下载缓冲（[`DownloadBudget`]）到预算即暂停下载、归还即恢复、任何退出路径都不泄漏额度。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::sync::mpsc;

use super::{
    DownloadBudget, PcmChunk, PreparedAudio, SentenceSource, chunk_ms, push_pcm,
    spawn_sentence_prefetch,
};
use crate::app::tts_cache::TtsCache;
use crate::plugins::tts::{TtsChunkCallback, TtsEngine};

/// 合成事件日志项：(事件名, 文本, 时刻)。事件名 `synth_start` / `synth_end`。
type Log = Arc<Mutex<Vec<(&'static str, String, Instant)>>>;

/// 可编程的假 TTS：记录事件时刻 + 统计调用次数，便于断言"谁先谁后"。
struct MockTts {
    log: Log,
    calls: AtomicUsize,
    /// 合成启动延迟（模拟在线合成短连接握手）
    startup: Duration,
    /// 分片之间的间隔
    chunk_gap: Duration,
    /// 分片数
    chunks: usize,
    /// 每片样本数（24kHz 下 2400 = 100ms 音频）
    samples_per_chunk: usize,
    /// 恒失败（校验 `done` 上报）
    fail: bool,
    /// 回调被取消（打断）的次数
    cancelled: AtomicUsize,
}

impl MockTts {
    fn new(log: Log) -> Self {
        Self {
            log,
            calls: AtomicUsize::new(0),
            startup: Duration::from_millis(20),
            chunk_gap: Duration::from_millis(30),
            chunks: 4,
            samples_per_chunk: 2400,
            fail: false,
            cancelled: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn at(&self, event: &str, text: &str) -> Option<Instant> {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(e, t, _)| *e == event && t == text)
            .map(|(_, _, at)| *at)
    }
}

impl TtsEngine for MockTts {
    fn synthesize_stream(
        &self,
        text: &str,
        _speed: f32,
        _speaker: i32,
        chunk_cb: TtsChunkCallback,
    ) -> Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(("synth_start", text.to_string(), Instant::now()));
        std::thread::sleep(self.startup);
        if self.fail {
            anyhow::bail!("mock 合成失败");
        }
        let mut cb = chunk_cb;
        let pcm = vec![0.01f32; self.samples_per_chunk];
        for _ in 0..self.chunks {
            if !cb(24_000, &pcm) {
                self.cancelled.fetch_add(1, Ordering::SeqCst);
                break;
            }
            std::thread::sleep(self.chunk_gap);
        }
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(("synth_end", text.to_string(), Instant::now()));
        Ok(())
    }

    fn output_sample_rate(&self) -> u32 {
        24_000
    }

    fn name(&self) -> &'static str {
        "mock"
    }
}

fn sentences(items: &[&str]) -> mpsc::UnboundedReceiver<String> {
    let (tx, rx) = mpsc::unbounded_channel();
    for s in items {
        tx.send((*s).to_string()).expect("投递句子");
    }
    drop(tx);
    rx
}

fn resolver(tts: Arc<MockTts>, cache_key: u64) -> impl FnMut(&str) -> SentenceSource + Send + 'static {
    move |_s: &str| SentenceSource { tts: tts.clone(), cache_key, speed: 1.0, speaker: 0 }
}

fn abort_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

/// 本轮下载缓冲（8s 预算）：单测只关心它不改变既有语义（顺序/串行/打断）。
fn budget() -> Arc<DownloadBudget> {
    DownloadBudget::new(8_000)
}

/// **核心回归**：下一句的合成必须在上一句**播完之前**启动。
///
/// 旧实现把合成串在播放之后（上一句最后一帧发完才启动下一句的合成），于是每到一个句边界
/// 都要重新付一次合成启动延迟（在线合成为一条短连接，数百 ms）——设备解码队列只有
/// `downlink_lead_ms` 的存货，覆盖不住 → 长回复逐句断音。预取把等待搬进播放时间窗内。
#[tokio::test]
async fn prefetch_starts_next_synthesis_before_current_finishes_playing() {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let tts = Arc::new(MockTts::new(log.clone()));
    let mut prepared =
        spawn_sentence_prefetch(
            abort_flag(),
            Arc::new(TtsCache::new(16)),
            budget(),
            resolver(tts.clone(), 1),
            sentences(&["A", "B"]),
        );

    // 第一句的成品立刻可取（预取 worker 不与消费者同步）
    let first = prepared.recv().await.expect("应取到第一句");
    assert_eq!(first.text, "A");
    // 模拟"按实时播放 A"：每片 100ms 音频 → 100ms 播放时间
    let PreparedAudio::Stream { mut chunks, .. } = first.audio else {
        panic!("首句应为流式合成路径");
    };
    let mut played = 0usize;
    while chunks.recv().await.is_some() {
        tokio::time::sleep(Duration::from_millis(100)).await;
        played += 1;
    }
    let downlink_end_a = Instant::now();
    assert_eq!(played, 4, "A 应播放 4 片（400ms）");
    // B 的成品也应已就绪（预取领先）
    let second = prepared.recv().await.expect("第二句应已预取就绪");
    assert_eq!(second.text, "B");

    let start_b = tts.at("synth_start", "B").expect("B 的合成应已启动");
    assert!(
        start_b < downlink_end_a,
        "B 的合成必须在 A 播完之前启动（旧串行实现做不到），实测 A 播完={downlink_end_a:?} B 起步={start_b:?}"
    );
    let lead = downlink_end_a.duration_since(start_b);
    assert!(
        lead >= Duration::from_millis(100),
        "预取应留出可观的时间窗（≥100ms），实测仅 {lead:?}"
    );
}

/// 合成必须**串行**：下一句的启动不得早于上一句的结束（本地 Kokoro 并发会过订阅 CPU）。
#[tokio::test]
async fn prefetch_serializes_synthesis() {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let tts = Arc::new(MockTts::new(log.clone()));
    let mut prepared = spawn_sentence_prefetch(
        abort_flag(),
        Arc::new(TtsCache::new(16)),
        budget(),
        resolver(tts.clone(), 1),
        sentences(&["A", "B", "C"]),
    );
    let mut texts = Vec::new();
    while let Some(item) = prepared.recv().await {
        texts.push(item.text.clone());
        if let PreparedAudio::Stream { chunks, .. } = item.audio {
            drop(chunks); // 丢弃分片接收端：合成随即因 send 失败/打断而收尾
        }
    }
    assert_eq!(texts, vec!["A", "B", "C"], "顺序必须与句子顺序一致");
    let end_a = tts.at("synth_end", "A").expect("A 结束");
    let start_b = tts.at("synth_start", "B").expect("B 开始");
    let end_b = tts.at("synth_end", "B").expect("B 结束");
    let start_c = tts.at("synth_start", "C").expect("C 开始");
    assert!(start_b >= end_a, "B 不得早于 A 结束（合成需串行）");
    assert!(start_c >= end_b, "C 不得早于 B 结束（合成需串行）");
}

/// 打断（barge-in）：置位后不再启动新的合成，且进行中的合成立即收尾、worker 退出。
///
/// 注意"取到 A"与"置位 abort"之间 worker 可能已经抢跑启动的第 2 句：这是允许的
/// （abort 是异步信号），断言因此不断言"恰好一句"，而是断言**不会无限往下跑**
/// （第 3 句起绝不启动）且 worker 最终退出。
#[tokio::test]
async fn prefetch_stops_on_abort() {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let mut tts = MockTts::new(log.clone());
    tts.chunks = 200; // 不打断的话要跑 ≈2s
    tts.chunk_gap = Duration::from_millis(10);
    let tts = Arc::new(tts);
    let abort = abort_flag();
    let mut prepared = spawn_sentence_prefetch(
        abort.clone(),
        Arc::new(TtsCache::new(16)),
        budget(),
        resolver(tts.clone(), 1),
        sentences(&["A", "B", "C"]),
    );
    // 取到 A 后立刻打断（A 还在合成中）
    let first = prepared.recv().await.expect("应取到 A");
    abort.store(true, Ordering::Relaxed);
    if let PreparedAudio::Stream { chunks, .. } = first.audio {
        drop(chunks);
    }
    // worker 应尽快退出（而不是把 B/C 全合成完）
    let mut texts = vec!["A".to_string()];
    loop {
        match tokio::time::timeout(Duration::from_millis(1500), prepared.recv()).await {
            Ok(Some(item)) => texts.push(item.text),
            Ok(None) => break,
            Err(e) => panic!("打断后 worker 未在 1.5s 内退出（已投递 {texts:?}）: {e:?}"),
        }
    }
    assert!(
        !texts.contains(&"C".to_string()),
        "打断后不得再往下合成新句子，实测投递 {texts:?}"
    );
    assert!(tts.calls() <= 2, "打断后最多只允许在飞的一句，实测 {} 次合成", tts.calls());
    assert!(
        tts.cancelled.load(Ordering::SeqCst) >= 1,
        "进行中的合成应被回调取消（回调返回 false → 引擎立即收尾）"
    );
}

/// 缓存命中：零合成、零编码，直接把已编码帧投递给下行。
#[tokio::test]
async fn prefetch_cache_hit_delivers_encoded_frames() {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let tts = Arc::new(MockTts::new(log));
    let cache = Arc::new(TtsCache::new(16));
    let frames = vec![vec![1u8, 2], vec![3u8], vec![4u8, 5, 6]];
    cache.insert(42, frames.clone());
    let mut prepared =
        spawn_sentence_prefetch(
            abort_flag(),
            cache,
            budget(),
            resolver(tts.clone(), 42),
            sentences(&["好的"]),
        );
    let item = prepared.recv().await.expect("应取到缓存命中项");
    assert_eq!(item.text, "好的");
    assert_eq!(item.cache_key, 42);
    match item.audio {
        PreparedAudio::Cached(f) => assert_eq!(*f, frames, "应原样投递缓存帧"),
        PreparedAudio::Stream { .. } => panic!("缓存命中不应再合成"),
    }
    assert_eq!(tts.calls(), 0, "缓存命中不得调用合成引擎");
}

/// 合成失败：`done` 必须报 false，调用方据此**不把残缺句写入缓存**。
#[tokio::test]
async fn prefetch_reports_failed_synthesis() {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let mut mock = MockTts::new(log);
    mock.fail = true;
    let tts = Arc::new(mock);
    let mut prepared = spawn_sentence_prefetch(
        abort_flag(),
        Arc::new(TtsCache::new(16)),
        budget(),
        resolver(tts.clone(), 1),
        sentences(&["会失败的句子"]),
    );
    let item = prepared.recv().await.expect("失败也要投递（调用方据此跳过缓存）");
    match item.audio {
        PreparedAudio::Stream { mut chunks, done, .. } => {
            assert!(chunks.recv().await.is_none(), "失败句不应有分片");
            assert!(!done.await.expect("done 应有值"), "合成失败必须上报 false");
        }
        PreparedAudio::Cached(_) => panic!("不应命中缓存"),
    }
}

/// 合成正常结束：`done` 报 true（调用方据此允许写缓存）。
#[tokio::test]
async fn prefetch_reports_successful_synthesis() {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let tts = Arc::new(MockTts::new(log));
    let mut prepared = spawn_sentence_prefetch(
        abort_flag(),
        Arc::new(TtsCache::new(16)),
        budget(),
        resolver(tts, 1),
        sentences(&["正常句子"]),
    );
    let item = prepared.recv().await.expect("应取到成品");
    match item.audio {
        PreparedAudio::Stream { mut chunks, done, .. } => {
            let mut n = 0;
            while chunks.recv().await.is_some() {
                n += 1;
            }
            assert_eq!(n, 4, "4 个分片应全部投递");
            assert!(done.await.expect("done 应有值"), "正常结束必须上报 true");
        }
        PreparedAudio::Cached(_) => panic!("不应命中缓存"),
    }
}

// ---- 下载缓冲（DownloadBudget）：按秒给"TTS 流下载"定预算 ----

/// 一片 PCM 的音频时长口径（生产端与消费端共用，必须一致）。
#[test]
fn chunk_ms_measures_audio_duration() {
    assert_eq!(chunk_ms(16_000, 16_000), 1_000, "16k 下 16000 样本 = 1s");
    assert_eq!(chunk_ms(2_400, 24_000), 100, "24k 下 2400 样本 = 100ms");
    assert_eq!(chunk_ms(0, 16_000), 1, "零样本按 1ms 计，不把水位算成 0");
    assert_eq!(chunk_ms(5, 0), 5_000, "sr=0 不得除零");
}

/// **核心语义**：积压到预算就必须**暂停下载**（阻塞在 reserve），消费归还后立刻恢复。
#[test]
fn download_budget_pauses_at_limit_and_resumes_on_release() {
    let b = DownloadBudget::new(300); // 300ms 预算
    assert!(b.reserve(200, &AtomicBool::new(false), &|| false));
    assert_eq!(b.queued_ms(), 200);

    // 再要 200ms 会超预算 → 必须在另一个线程里阻塞（= 下载被限速）
    let b2 = b.clone();
    let waiter = std::thread::spawn(move || b2.reserve(200, &AtomicBool::new(false), &|| false));
    std::thread::sleep(Duration::from_millis(60));
    assert!(!waiter.is_finished(), "积压到预算时必须暂停下载");

    b.release(200); // 消费一片 → 额度归还
    assert!(waiter.join().expect("线程正常结束"), "额度归还后下载必须立刻恢复");
    assert_eq!(b.queued_ms(), 200);
    // 限速等待在**等到之后**记账（未完成/被打断的等待不记账——那说明本轮已结束）
    let after = b.stats();
    assert!(after.throttle_ms >= 40, "限速等待必须被记账，实测 {}ms", after.throttle_ms);
    assert_eq!(after.throttle_events, 1, "一次等待只记一次事件");
    assert_eq!(after.peak_ms, 200, "峰值水位 = 实际积压");
}

/// 单片大于预算：按"至少放行一片"处理，否则整段一次性回传的上游会被卡死。
#[test]
fn download_budget_admits_oversized_single_chunk() {
    let b = DownloadBudget::new(100);
    assert!(b.reserve(5_000, &AtomicBool::new(false), &|| false), "单片超预算也必须放行");
    assert_eq!(b.queued_ms(), 5_000);
}

/// 额度归还靠 [`PcmChunk`] 的 Drop：**任何**退出路径都不泄漏。
///
/// 漏还的后果很隐蔽——同一轮后续句子的下载会被虚高水位**永久摁住**
///（表现为"第一句之后就没声了"），所以这里把三条路径都钉住。
#[test]
fn pcm_chunk_drop_returns_quota_on_every_path() {
    let b = DownloadBudget::new(300);
    let abort = AtomicBool::new(false);
    // ① 正常消费（取走即 Drop）
    assert!(b.reserve(100, &abort, &|| false));
    assert_eq!(b.queued_ms(), 100);
    drop(PcmChunk::budgeted(vec![0.0; 100], 100, b.clone()));
    assert_eq!(b.queued_ms(), 0, "取走分片必须归还额度");

    // ② 接收端被整个丢弃（打断 / 本轮结束）：通道里残留的分片照样归还
    let (tx, rx) = mpsc::channel::<PcmChunk>(4);
    assert!(b.reserve(50, &abort, &|| false));
    tx.try_send(PcmChunk::budgeted(vec![0.0; 10], 50, b.clone())).expect("入队");
    assert_eq!(b.queued_ms(), 50);
    drop(rx);
    assert_eq!(b.queued_ms(), 0, "接收端被丢弃也必须归还额度");

    // ③ 未记账分片（AIUI 一次性回传路径）：不影响水位
    drop(PcmChunk::unbudgeted(vec![0.0; 10]));
    assert_eq!(b.queued_ms(), 0);
}

/// 读端已放弃时 `push_pcm` 返回 false **且不泄漏额度**（合成据此立刻收尾）。
#[test]
fn push_pcm_releases_quota_when_consumer_gone() {
    let b = DownloadBudget::new(8_000);
    let (tx, rx) = mpsc::channel::<PcmChunk>(4);
    drop(rx);
    let abort = AtomicBool::new(false);
    assert!(
        !push_pcm(&b, &tx, vec![0.0; 1_600], 16_000, &abort),
        "读端已放弃 → 回调必须返回 false"
    );
    assert_eq!(b.queued_ms(), 0, "失败路径不得泄漏额度");
}

/// 打断置位时 `reserve` 立刻放弃（不把合成线程钉在限速等待里）。
#[test]
fn reserve_gives_up_on_abort() {
    let b = DownloadBudget::new(100);
    assert!(b.reserve(100, &AtomicBool::new(false), &|| false));
    let abort = AtomicBool::new(true);
    assert!(!b.reserve(100, &abort, &|| false), "已打断 → 不得再等额度");
    // 读端关闭同样立刻放弃（不必等 abort）
    let abort2 = AtomicBool::new(false);
    assert!(!b.reserve(100, &abort2, &|| true), "读端关闭 → 不得再等额度");
}

/// **端到端**：预算把"上游下载"摁住，但**不丢片、不乱序、内存有界**。
///
/// 这是"根据缓存需要控制下载流速"的验收：上游尽速回传（4s 音频、零间隔），消费端
/// 故意慢（10ms/片），预算只有 300ms → 必然触发限速；但流式语义不能因此退化。
#[tokio::test]
async fn download_budget_bounds_pipeline_without_losing_chunks() {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let mut mock = MockTts::new(log);
    mock.startup = Duration::from_millis(0);
    mock.chunk_gap = Duration::from_millis(0); // 上游尽速下载
    mock.chunks = 40;
    mock.samples_per_chunk = 2_400; // 24k → 100ms/片，共 4s 音频
    let tts = Arc::new(mock);
    let budget = DownloadBudget::new(300); // 远小于 4s → 必然限速
    let mut prepared = spawn_sentence_prefetch(
        abort_flag(),
        Arc::new(TtsCache::new(16)),
        budget.clone(),
        resolver(tts, 1),
        sentences(&["A"]),
    );
    let item = prepared.recv().await.expect("应取到成品");
    let PreparedAudio::Stream { mut chunks, .. } = item.audio else {
        panic!("首句应为流式合成路径");
    };
    let mut got = 0;
    // 消费端慢取（模拟"编码入水库"跟不上上游下载）
    while chunks.recv().await.is_some() {
        got += 1;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(got, 40, "限速不得丢片（流式语义不变）");
    let st = budget.stats();
    assert!(st.throttle_ms > 0, "上游尽速 + 消费慢 → 必须触发下载限速");
    assert!(st.throttle_events >= 1, "限速事件必须被记账");
    assert!(st.peak_ms <= 300, "积压不得超出预算，实测 {}ms", st.peak_ms);
    assert_eq!(budget.queued_ms(), 0, "本轮结束后额度必须归零");
}
