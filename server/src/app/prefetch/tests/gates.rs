//! 推理闸门（缺口①）与 first_pcm 埋点（缺口②）的预取层测试。
//!
//! 从父模块 `tests.rs` 拆出（AGENTS.md §5.10 文件规模约定：行为不变的搬移），
//! 复用父模块的 MockTts / Log / fixture（`use super::*`）。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::*;
use crate::config::Config;

/// 测试用 TTS 闸门（默认与生产一致的 1 路许可；串行断言用例传 1）。
fn gate(permits: u32) -> crate::app::inference::GateHandle {
    let mut cfg = Config::default();
    cfg.tts.max_concurrent = permits;
    crate::app::inference::InferenceGates::new(&cfg).tts()
}

/// 跟踪并发合成数的假 TTS：进入 `synthesize_stream` 计数 +1、退出 −1，
/// 同时记录观测到的**最大并发**——闸门生效则恒为 1。
struct ConcurrencyTrackingTts {
    live: Arc<AtomicUsize>,
    max_live: Arc<AtomicUsize>,
}

impl TtsEngine for ConcurrencyTrackingTts {
    fn synthesize_stream(
        &self,
        _text: &str,
        _speed: f32,
        _speaker: i32,
        chunk_cb: TtsChunkCallback,
    ) -> Result<()> {
        let now = self.live.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_live.fetch_max(now, Ordering::SeqCst);
        // 合成耗时长一点，让两路有机会重叠（闸门若失效，这里会观察到 max=2）
        std::thread::sleep(Duration::from_millis(120));
        let mut cb = chunk_cb;
        let _ = cb(24_000, &vec![0.01f32; 2400]);
        self.live.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }
    fn output_sample_rate(&self) -> u32 {
        24_000
    }
    fn name(&self) -> &'static str {
        "concurrency-mock"
    }
}

/// **闸门串行**：单许可闸门下，两个会话（两个独立预取 worker）同时合成，
/// 任一时刻在飞合成路数 ≤ 1（`max_live == 1`）。这是缺口①的核心验收——
/// 生产语义"多设备同时说话时在闸门排队而不是压垮 CPU"。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inference_gate_serializes_concurrent_synthesis() {
    let live = Arc::new(AtomicUsize::new(0));
    let max_live = Arc::new(AtomicUsize::new(0));
    let tts = Arc::new(ConcurrencyTrackingTts { live: live.clone(), max_live: max_live.clone() });
    // 两个会话共享**同一把** 1 许可闸门（进程级语义：GateHandle clone 即共享）
    let shared_gate = gate(1);

    let mk = |g: crate::app::inference::GateHandle| {
        let tts = tts.clone();
        spawn_sentence_prefetch(
            abort_flag(),
            Arc::new(TtsCache::new(16)),
            budget(),
            g,
            None,
            move |_s: &str| SentenceSource { tts: tts.clone(), cache_key: 0, speed: 1.0, speaker: 0 },
            sentences(&["句一"]),
        )
    };
    let (mut a, mut b) = (mk(shared_gate.clone()), mk(shared_gate));
    // 两路都取到成品（各自一句合成完成）
    let (item_a, item_b) = tokio::join!(a.recv(), b.recv());
    for item in [item_a.expect("A 应有成品"), item_b.expect("B 应有成品")] {
        if let PreparedAudio::Stream { chunks, .. } = item.audio {
            drop(chunks);
        }
    }
    // 等两个 worker 完全收尾（合成退出计数回零）
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        max_live.load(Ordering::SeqCst),
        1,
        "单许可闸门下并发合成必须串行（实测最大并发 {}）",
        max_live.load(Ordering::SeqCst)
    );
    assert_eq!(live.load(Ordering::SeqCst), 0, "收尾后无在飞合成");
}

/// 多许可闸门（permits=2）：两路可并行（闸门是"上限"而不是"串行器"）。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inference_gate_allows_parallel_when_permits_allow() {
    let live = Arc::new(AtomicUsize::new(0));
    let max_live = Arc::new(AtomicUsize::new(0));
    let tts = Arc::new(ConcurrencyTrackingTts { live: live.clone(), max_live: max_live.clone() });
    let shared_gate = gate(2);
    let mk = |g: crate::app::inference::GateHandle| {
        let tts = tts.clone();
        spawn_sentence_prefetch(
            abort_flag(),
            Arc::new(TtsCache::new(16)),
            budget(),
            g,
            None,
            move |_s: &str| SentenceSource { tts: tts.clone(), cache_key: 0, speed: 1.0, speaker: 0 },
            sentences(&["句一"]),
        )
    };
    let (mut a, mut b) = (mk(shared_gate.clone()), mk(shared_gate));
    let (item_a, item_b) = tokio::join!(a.recv(), b.recv());
    for item in [item_a.expect("A 应有成品"), item_b.expect("B 应有成品")] {
        if let PreparedAudio::Stream { chunks, .. } = item.audio {
            drop(chunks);
        }
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        max_live.load(Ordering::SeqCst),
        2,
        "2 许可下两路合成应并行（放宽闸门 = 提高并发上限）"
    );
}

/// first_pcm 埋点：只在**非缓存**句触发一次。first-only 语义由调用方闭包用
/// `AtomicBool::swap` 实现（与生产 `reply.rs` 同款写法）；本测试验证
/// prefetch 把回调接进了合成路径（每个分片都会调它）。
#[tokio::test]
async fn first_pcm_hook_fires_once_for_uncached_sentence() {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let tts = Arc::new(MockTts::new(log));
    let fired = Arc::new(AtomicUsize::new(0));
    let once = Arc::new(AtomicBool::new(false));
    let fired_cb = fired.clone();
    let hook: Arc<dyn Fn() + Send + Sync> = {
        let once = once.clone();
        Arc::new(move || {
            // first-only：与生产 reply.rs 同款（swap 返回旧值 false 时才计数）
            if !once.swap(true, Ordering::Relaxed) {
                fired_cb.fetch_add(1, Ordering::SeqCst);
            }
        })
    };
    let mut prepared = spawn_sentence_prefetch(
        abort_flag(),
        Arc::new(TtsCache::new(16)),
        budget(),
        gate(1),
        Some(hook),
        resolver(tts, 1),
        sentences(&["多分片的句子"]),
    );
    let item = prepared.recv().await.expect("应取到成品");
    if let PreparedAudio::Stream { mut chunks, .. } = item.audio {
        while chunks.recv().await.is_some() {} // 消费全部分片（4 片）
    }
    assert_eq!(
        fired.load(Ordering::SeqCst),
        1,
        "4 个分片只应计一次 first_pcm（first-only 由闭包保证）"
    );
}
