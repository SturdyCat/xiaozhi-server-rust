//! 下行音频**水库**：用服务器内存换下行连续性的那一层缓冲。
//!
//! ## 为什么需要（2026-10 真机反馈：长回复「从中间某句起断断续续」）
//!
//! 设备解码队列是**硬上限**（`[audio].downlink_queue_packets` 包 × 帧长，官方 20×60ms
//! ≈1.2s，见 [`crate::workflow::downlink::DownlinkPacer`]），服务端再怎么提前也只有 ~1s 存货。
//! 而**上游供数常常慢于实时**：LLM 逐句吐字（真机实测 14 字用 5.7s、音频 3.7s →
//! 约 0.64 倍速）、TTS 在线合成偶发握手/首片延迟、句与句之间的模型停顿……任何一种
//! 只要让"音频到达速率 < 播放消耗速率"持续超过 ~1s，设备队列就被抽干 → 断断续续。
//!
//! 能让**服务端**继续按实时节奏供数的唯一办法：把"已经合成好的音频"多备几秒放在
//! **服务器内存**里（水库），发送端从水库取，而不是"现合成、现发送"。于是：
//!
//! - 上游停顿 ≤ 水库水位 + 设备队列（≈ `downlink_buffer_ms` + `lead_ms`）时听感**全程连续**；
//! - 代价是**内存**（Opus 帧约 2KB/s，30s ≈ 60KB/会话，几乎可忽略）与**起跑延迟**
//!   （`[audio].downlink_buffer_start_ms`）：先攒后播，把"散在全句的断续"
//!   换成"一次起跑等待"。
//!
//! ⚠️ **水库救不了"上游持续慢"**：它只能备住**已经合成出来**的音频。上游**平均**慢于实时
//! （上面那个 0.64×）时缺口是**累积**的，要整轮不断音得先把累计缺口攒出来
//! `所需缓冲 ≈ (1/生产速率 − 1) × 音频总长`（0.64× 生产 + 80s 音频 → 约 45s，等于
//! 开口前先静默 45s）。那种情形只有**提速上游**（提高 `[tts].speed` 让音频变短 / 换更快的
//! LLM/TTS）才是根治——判据是每轮日志的 `本轮吞吐：音频 Xms / 播放窗口 Yms ≈ N× 实时`。
//!
//! ## 结构（生产者 / 消费者解耦）
//!
//! ```text
//! LLM 分句 → prefetch（逐句串行合成） → 填充任务（重采样 + Opus 分帧） → 水库 → 发送端（实时节拍）
//!                                  [`spawn_reservoir_filler`]        `ReservoirSink`  `ReservoirSource`
//! ```
//!
//! 水库上游还有一层**下载缓冲**（`workflow::pipeline::prefetch::DownloadBudget`，按秒给 TTS 流下载定预算）；
//! 本模块**不感知**它——额度随分片 `PcmChunk` 走，编码完即 Drop 归还。
//!
//! - **容量** = `[audio].downlink_buffer_ms`（帧数上限）：满则填充任务 `push` 等待（**背压**），
//!   不会无界合成、无界占内存；
//! - **顺序**：`SentenceStart(text)` 与同句各帧同序入队，发送端按序取出，
//!   天然保证「句首通知先于该句音频」；
//! - **收工**：填充任务丢弃写端 → 读端抽空后 `recv()` 返回 `None`；
//! - **打断**：读端被丢弃（或 `abort` 置位）→ `push` 返回 `false` → 填充任务立刻收尾，
//!   不再白合成/白编码本轮剩余音频。

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, Notify};

use crate::workflow::pipeline::prefetch::{PreparedAudio, PreparedSentence, PREFETCH_SENTENCES};
use crate::workflow::session::SessionParams;
use crate::workflow::pipeline::tts_cache::TtsCache;

use self::collect::SentenceCollector;
use super::framer::StreamingFramer;

/// 水库里的一项（入队顺序 = 播出顺序）。
#[derive(Debug, Clone)]
pub(crate) enum DownlinkItem {
    /// 句首通知：发送端据此下发 `sentence_start(text)`，**必须先于**同句音频帧。
    SentenceStart(String),
    /// 一个按会话实际格式编码的下行帧负载（Opus 或 PCM S16LE）（**不含**二进制协议头：协议头由发送端按当前
    /// 下行版本 + 时间戳现包，水库因此与协议版本解耦）。
    Frame(Vec<u8>),
}

/// 一轮下行的水库观测（每轮回复结束汇总打印，长回复断音现场证据）。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ReservoirStats {
    /// 水库容量（毫秒）
    pub cap_ms: u64,
    /// 水位峰值（毫秒）：**实际用上了多少缓冲**
    pub peak_ms: u64,
    /// 水位峰值对应的内存（字节，已编码帧）
    pub peak_bytes: usize,
    /// 填充任务因水库满而等待的累计时长（毫秒，= 上游被背压的时间）
    pub backpressure_ms: u64,
    /// 发送端等不到帧（水库见底）的次数与累计时长：> 0 说明上游供数确实慢于播放
    pub starve_events: u64,
    pub starve_ms: u64,
    /// 起跑闸实际等待时长与其结束时的水位（毫秒）
    pub gate_ms: u64,
    pub gate_depth_ms: u64,
}

/// 生产端 / 消费端共享的水位与观测。
pub(crate) const MAX_RESERVOIR_BYTES: usize = 8 * 1024 * 1024;

struct ReservoirShared {
    /// 容量（帧数）。
    cap_frames: usize,
    duration_slots: Arc<tokio::sync::Semaphore>,
    byte_slots: Arc<tokio::sync::Semaphore>,
    /// 一帧多少毫秒（水位换算）。
    frame_ms: u64,
    depth_frames: AtomicUsize,
    depth_bytes: AtomicUsize,
    peak_frames: AtomicUsize,
    peak_bytes: AtomicUsize,
    backpressure_ms: AtomicU64,
    starve_events: AtomicUsize,
    starve_ms: AtomicU64,
    gate_ms: AtomicU64,
    gate_depth_ms: AtomicU64,
    /// 水位变化通知（起跑闸等待用；等待侧另有超时兜底，故 `notify_waiters` 足够）。
    notify: Notify,
}

impl ReservoirShared {
    fn new(cap_frames: usize, frame_ms: u64) -> Self {
        Self {
            cap_frames,
            duration_slots: Arc::new(tokio::sync::Semaphore::new(cap_frames)),
            byte_slots: Arc::new(tokio::sync::Semaphore::new(MAX_RESERVOIR_BYTES)),
            frame_ms,
            depth_frames: AtomicUsize::new(0),
            depth_bytes: AtomicUsize::new(0),
            peak_frames: AtomicUsize::new(0),
            peak_bytes: AtomicUsize::new(0),
            backpressure_ms: AtomicU64::new(0),
            starve_events: AtomicUsize::new(0),
            starve_ms: AtomicU64::new(0),
            gate_ms: AtomicU64::new(0),
            gate_depth_ms: AtomicU64::new(0),
            notify: Notify::new(),
        }
    }

    fn depth_frames(&self) -> usize {
        self.depth_frames.load(Ordering::Relaxed)
    }

    fn depth_ms(&self) -> u64 {
        self.depth_frames() as u64 * self.frame_ms
    }

    fn on_push(&self, frames: usize, bytes: usize) {
        if frames == 0 {
            return;
        }
        let depth = self.depth_frames.fetch_add(frames, Ordering::Relaxed) + frames;
        let bytes_now = self.depth_bytes.fetch_add(bytes, Ordering::Relaxed) + bytes;
        self.peak_frames.fetch_max(depth, Ordering::Relaxed);
        self.peak_bytes.fetch_max(bytes_now, Ordering::Relaxed);
    }

    fn on_pop(&self, item: &DownlinkItem) {
        let frames = item_frames(item);
        if frames == 0 {
            return;
        }
        self.depth_frames.fetch_sub(frames, Ordering::Relaxed);
        self.depth_bytes
            .fetch_sub(item_bytes(item), Ordering::Relaxed);
        self.duration_slots.add_permits(frames);
        self.byte_slots.add_permits(item_bytes(item));
    }

    /// 入队失败（读端已放弃）时回滚 [`Self::on_push`] 的记账。
    fn on_rollback(&self, frames: usize, bytes: usize) {
        if frames == 0 {
            return;
        }
        self.depth_frames.fetch_sub(frames, Ordering::Relaxed);
        self.depth_bytes.fetch_sub(bytes, Ordering::Relaxed);
    }

    fn stats(&self) -> ReservoirStats {
        ReservoirStats {
            cap_ms: self.cap_frames as u64 * self.frame_ms,
            peak_ms: self.peak_frames.load(Ordering::Relaxed) as u64 * self.frame_ms,
            peak_bytes: self.peak_bytes.load(Ordering::Relaxed),
            backpressure_ms: self.backpressure_ms.load(Ordering::Relaxed),
            starve_events: self.starve_events.load(Ordering::Relaxed) as u64,
            starve_ms: self.starve_ms.load(Ordering::Relaxed),
            gate_ms: self.gate_ms.load(Ordering::Relaxed),
            gate_depth_ms: self.gate_depth_ms.load(Ordering::Relaxed),
        }
    }
}

fn item_frames(item: &DownlinkItem) -> usize {
    match item {
        DownlinkItem::Frame(_) => 1,
        DownlinkItem::SentenceStart(_) => 0,
    }
}

fn item_bytes(item: &DownlinkItem) -> usize {
    match item {
        DownlinkItem::Frame(f) => f.len(),
        DownlinkItem::SentenceStart(_) => 0,
    }
}

/// 水库写端（填充任务持有）。写端全部丢弃 = 生产端收工。
pub(crate) struct ReservoirSink {
    tx: mpsc::Sender<DownlinkItem>,
    shared: Arc<ReservoirShared>,
}

/// 水库读端（发送端持有）。
pub(crate) struct ReservoirSource {
    rx: mpsc::Receiver<DownlinkItem>,
    shared: Arc<ReservoirShared>,
}

/// 建一座水库：`cap_ms` 为容量（毫秒；0 = 关闭缓冲，仅保留 1 帧前瞻），
/// `frame_ms` 为协商帧长（水位换算）。
pub(crate) fn new_reservoir(cap_ms: u32, frame_ms: u32) -> (ReservoirSink, ReservoirSource) {
    let frame_ms = frame_ms.max(1) as u64;
    let cap_frames = if cap_ms == 0 {
        1
    } else {
        (cap_ms as u64 / frame_ms).max(1) as usize
    };
    // 通道槽位 = 帧容量 + 流水线深度（每句一个 `SentenceStart` 槽位，最多领先 4 句）
    let cap_items = (cap_frames + PREFETCH_SENTENCES).max(2);
    let (tx, rx) = mpsc::channel::<DownlinkItem>(cap_items);
    let shared = Arc::new(ReservoirShared::new(cap_frames, frame_ms));
    (
        ReservoirSink {
            tx,
            shared: shared.clone(),
        },
        ReservoirSource { rx, shared },
    )
}

impl ReservoirSink {
    /// 入队一项；水库满则等待（**背压**）。返回 `false` = 读端已放弃（打断/本轮结束），
    /// 调用方应立即收尾，不要再合成/编码。
    pub(crate) async fn push(&self, item: DownlinkItem) -> bool {
        let frames = item_frames(&item);
        let bytes = item_bytes(&item);
        let t = Instant::now();
        if bytes > MAX_RESERVOIR_BYTES {
            return false;
        }
        let Ok(duration) = self
            .shared
            .duration_slots
            .clone()
            .acquire_many_owned(frames as u32)
            .await
        else {
            return false;
        };
        let Ok(byte_guard) = self
            .shared
            .byte_slots
            .clone()
            .acquire_many_owned(bytes as u32)
            .await
        else {
            return false;
        };
        // ⚠️ **先记账再入队**：`send().await` 返回时消费者可能已经 `recv()` 取走该项，
        // 若在 send 之后才自增，读端的自减会先发生 → 水位计数下溢成天文数字
        // （起跑闸会误判"水库已满"，观测也全错）。失败/取消由 `PushGuard` 回滚。
        self.shared.on_push(frames, bytes);
        let guard = PushGuard {
            shared: &self.shared,
            frames,
            bytes,
            committed: false,
            duration: Some(duration),
            byte_guard: Some(byte_guard),
        };
        if self.tx.send(item).await.is_err() {
            return false; // guard 析构即回滚记账
        }
        guard.commit();
        // 只有"因为满而等待"才计入背压（正常入队 < 1ms）
        let waited = t.elapsed().as_millis() as u64;
        if waited > 0 {
            self.shared
                .backpressure_ms
                .fetch_add(waited, Ordering::Relaxed);
        }
        self.shared.notify.notify_waiters();
        true
    }
}

/// [`ReservoirSink::push`] 的记账守卫：入队成功才"提交"，否则（含 future 被取消）
/// 析构时回滚水位计数——保证水位不会因失败/取消而下溢。
struct PushGuard<'a> {
    shared: &'a ReservoirShared,
    frames: usize,
    bytes: usize,
    committed: bool,
    duration: Option<tokio::sync::OwnedSemaphorePermit>,
    byte_guard: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl PushGuard<'_> {
    fn commit(mut self) {
        self.committed = true;
        self.duration.take().expect("duration reservation").forget();
        self.byte_guard.take().expect("byte reservation").forget();
    }
}

impl Drop for PushGuard<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.shared.on_rollback(self.frames, self.bytes);
        }
    }
}

impl Drop for ReservoirSource {
    fn drop(&mut self) {
        self.shared.duration_slots.close();
        self.shared.byte_slots.close();
    }
}

impl ReservoirSource {
    /// 取出一项；水库空则等待。返回 `None` = **生产端已收工且水库已抽空**。
    ///
    /// 记录"等空"（上游断供）次数与时长：发送端的实时节拍掩盖不了它——
    /// 水位见底 + 设备队列 ~lead 一起被吃光就是听感断音。
    pub(crate) async fn recv(&mut self) -> Option<DownlinkItem> {
        let empty = self.shared.depth_frames() == 0;
        let t = Instant::now();
        let item = self.rx.recv().await;
        if let Some(it) = &item {
            self.shared.on_pop(it);
        }
        if empty && item.is_some() {
            let waited = t.elapsed().as_millis() as u64;
            if waited > 0 {
                self.shared.starve_ms.fetch_add(waited, Ordering::Relaxed);
                if waited >= self.shared.frame_ms {
                    self.shared.starve_events.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        item
    }

    /// 当前水位（帧数；单测断言用，生产只看 [`Self::depth_ms`]）。
    #[cfg(test)]
    pub(crate) fn depth_frames(&self) -> usize {
        self.shared.depth_frames()
    }

    pub(crate) fn depth_ms(&self) -> u64 {
        self.shared.depth_ms()
    }

    /// 补记"水库见底"（发送端用 200ms 轮询超时取消 `recv` 时，等空不会经过
    /// [`Self::recv`] 的记账路径——长断供会被轮询切成多段，必须由发送端补记，
    /// 否则"上游断供"观测在真正长断供时反而为 0）。
    pub(crate) fn note_starve(&self, waited: Duration) {
        if self.shared.depth_frames() != 0 {
            return;
        }
        let ms = waited.as_millis() as u64;
        self.shared.starve_ms.fetch_add(ms, Ordering::Relaxed);
        if ms >= self.shared.frame_ms {
            self.shared.starve_events.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// 生产端是否已收工（写端全部丢弃；水库里可能还有存货）。
    pub(crate) fn producer_done(&self) -> bool {
        self.rx.is_closed()
    }

    /// 等水位变化（最多 `timeout`）。返回是否被通知。
    pub(crate) async fn wait_depth_change(&self, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, self.shared.notify.notified())
            .await
            .is_ok()
    }

    /// 记录起跑闸等待（观测口径：等待时长 + 结束时水位）。
    pub(crate) fn record_gate(&self, waited: Duration) {
        self.shared
            .gate_ms
            .store(waited.as_millis() as u64, Ordering::Relaxed);
        self.shared
            .gate_depth_ms
            .store(self.shared.depth_ms(), Ordering::Relaxed);
    }

    /// 观测快照（本轮结束打印）。
    pub(crate) fn stats(&self) -> ReservoirStats {
        self.shared.stats()
    }
}

/// 启动**水库填充任务**：消费预取成品（句子流），重采样 + Opus 分帧后逐帧入水库。
///
/// 每句的顺序是「先 `SentenceStart`，再该句各帧」；完整合成（引擎正常结束且未被
/// 打断/未被读端放弃）的句子顺便写进结果缓存（残缺句不入缓存，见 `workflow::pipeline::tts_cache`）。
///
/// 本任务**不感知下载缓冲**：额度随分片（[`crate::workflow::pipeline::prefetch::PcmChunk`]）走，
/// 编码完即 Drop 归还（见 `workflow::pipeline::prefetch` 模块文档的三段缓冲分工）。
///
/// 任务在下列情况收尾：句子流结束、`abort` 置位、读端放弃（`push` 返回 false）。
/// 收尾即丢弃写端 → 读端 `recv()` 抽空后返回 `None`。
pub(crate) fn spawn_reservoir_filler(
    abort: Arc<AtomicBool>,
    cache: Arc<TtsCache>,
    params: SessionParams,
    mut prepared: mpsc::Receiver<PreparedSentence>,
    sink: ReservoirSink,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(sentence) = prepared.recv().await {
            if abort.load(Ordering::Relaxed) {
                break;
            }
            if !push_sentence(&sink, &cache, &params, &abort, sentence).await {
                break;
            }
        }
        drop(sink);
    })
}

/// 单句路径（AIUI 全链路 / 测试台）：一句现成成品直接入水库，链路与逐句预取完全一致。
pub(crate) fn spawn_single_sentence_filler(
    abort: Arc<AtomicBool>,
    cache: Arc<TtsCache>,
    params: SessionParams,
    sink: ReservoirSink,
    text: String,
    audio: PreparedAudio,
    cache_key: u64,
) -> tokio::task::JoinHandle<()> {
    let (tx, rx) = mpsc::channel::<PreparedSentence>(1);
    // 容量 1 且此刻为空 → 必然成功；失败只可能是逻辑错误（不应发生）
    if tx
        .try_send(PreparedSentence {
            text,
            audio,
            cache_key,
        })
        .is_err()
    {
        tracing::warn!("单句水库填充入队失败（容量 1 却拒收，视为内部错误）");
    }
    drop(tx);
    spawn_reservoir_filler(abort, cache, params, rx, sink)
}

/// 把一句成品推入水库。返回 `false` = 收尾（读端放弃 / 打断），调用方别再推下一句。
async fn push_sentence(
    sink: &ReservoirSink,
    cache: &TtsCache,
    params: &SessionParams,
    abort: &AtomicBool,
    sentence: PreparedSentence,
) -> bool {
    let PreparedSentence {
        text,
        audio,
        cache_key,
    } = sentence;
    // 句首通知先入队：顺序即播出顺序（发送端据此在音频前下发 sentence_start）
    if !sink.push(DownlinkItem::SentenceStart(text.clone())).await {
        return false;
    }
    match audio {
        PreparedAudio::Cached(frames) => {
            for frame in frames.iter() {
                if abort.load(Ordering::Relaxed) {
                    return false;
                }
                if !sink.push(DownlinkItem::Frame(frame.clone())).await {
                    return false;
                }
            }
            true
        }
        PreparedAudio::Stream {
            sr,
            mut chunks,
            done,
        } => {
            let mut framer = match StreamingFramer::new_format(sr, params.output_format()) {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!("建立 Opus 分帧器失败（本句丢弃，{text}）: {e:#}");
                    drop(chunks); // 停止上游，不能持接收端等待已被背压的合成退出
                    let _ = done.await;
                    return true; // 单句失败不终止整轮（与旧实现一致：只丢这一句）
                }
            };
            let mut collected: Option<SentenceCollector> =
                (cache_key != 0).then(SentenceCollector::new);
            let mut intact = true;
            while let Some(chunk) = chunks.recv().await {
                if abort.load(Ordering::Relaxed) {
                    intact = false;
                    break;
                }
                // 先编码，再**立刻丢弃分片**：Drop 即把下载额度还给上游（见 `PcmChunk`）。
                // 于是水库的背压不会反向摁住 TTS 下载——两段缓冲各按自己的口径吃满
                // （水库多秒 Opus + 下载缓冲多秒 PCM，见 `workflow::pipeline::prefetch` 模块文档）。
                // 缓存收集（下行帧副本 + 引擎原样 PCM，不重采样）也只在 Drop 之前做。
                let encoded = framer.push(&chunk.pcm);
                if let Some(c) = collected.as_mut() {
                    c.on_chunk(&chunk.pcm);
                }
                drop(chunk);
                match encoded {
                    Ok(frames) => {
                        for frame in frames {
                            if !sink.push(DownlinkItem::Frame(frame.clone())).await {
                                return false;
                            }
                            if let Some(c) = collected.as_mut() {
                                c.on_frame(&frame);
                            }
                        }
                    }
                    Err(e) => {
                        // 编码失败 = 整句作废：不再继续该句、也不写缓存（帧不完整）
                        tracing::warn!("Opus 编码失败（整句丢弃，{text}）: {e:#}");
                        intact = false;
                        break;
                    }
                }
            }
            if intact {
                match framer.finish() {
                    Ok(Some(tail)) => {
                        if !sink.push(DownlinkItem::Frame(tail.clone())).await {
                            return false;
                        }
                        if let Some(c) = collected.as_mut() {
                            c.on_frame(&tail);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!("Opus 编码失败（流式尾帧，本句不入缓存）: {e:#}");
                        intact = false;
                    }
                }
            }
            // 完整句才入缓存（引擎正常结束 + 未被中断 + 编码全程成功）。收集器缺席
            //（`cache_key == 0`：AIUI 云端回传音频不可复现 / 测试台直连合成，与旧实现
            // 传 `cache_sink: None` 同义）或中途超限放弃时，`take()` 给 None = 不入缓存。
            drop(chunks); // 编码失败/打断时解除上游背压，避免等待 done 死锁
            if intact
                && matches!(done.await, Ok(true))
                && !abort.load(Ordering::Relaxed)
                && !sink.tx.is_closed()
            {
                if let Some((frames, pcm)) = collected.take().and_then(|c| c.take()) {
                    cache.insert_async(cache_key, frames, Some(pcm)).await;
                }
            }
            true
        }
    }
}

mod collect;
#[cfg(test)]
mod tests;
