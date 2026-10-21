//! 逐句音频**预取**：让「合成」跑在「播放」前面，消除句间气泡。
//!
//! ## 为什么需要（2026-10 真机反馈：长回复 TTS「断断续续」）
//!
//! 旧实现是**严格串行**的：`stream_response` 逐句 `synthesize_stream` → 边收边按实时
//! 节拍下发 → 该句最后一帧发完，才去**启动**下一句的合成。于是每一句的合成启动延迟都
//! 落在「上一句已播完、下一句还没有数据」的空窗里：
//! - 在线合成（讯飞 AIUI 主动合成）每次调用都是**一条短连接**（DNS/TCP/TLS/WS 握手 +
//!   服务端首帧），实测数百 ms 起（见 `plugins/tts/aiui.rs` 的"oneshot 语义"）；
//! - 本地 Kokoro 也要等 `spawn_blocking` 调度 + 首段推理。
//!
//! 设备解码队列只有 `[audio].downlink_lead_ms` 的存货（默认 1000ms，硬上限 20 包 ≈1.2s），
//! 覆盖不了这个空窗 → **每到一个句边界就断一次音**。短回复只有一句，永远看不到这个问题；
//! 长回复句越多断得越密——与真机现象（背乘法口诀表，从中间某一句开始断断续续）一致。
//!
//! 预取把「下一句的合成」提前到「上一句还在播」的时间窗内完成：合成仍然**串行**
//!（同一时刻只跑一句——本地引擎并发只会过订阅本就紧张的 CPU，在线引擎也没有并发收益），
//! 但**不再等播放**。句边界处数据早已就绪，节拍器
//!（[`crate::app::downlink::DownlinkPacer`]）可以无缝续上。
//!
//! ## 与节拍器的分工（勿越界）
//! - 本模块只负责「尽快把句子变成 PCM 分片 / 已编码帧」，**不做任何节流**；
//! - 节流 + 真空重锚定全在 [`DownlinkPacer`]：预取再快也灌不爆 ESP 解码队列。
//! - 背压：[`PREFETCH_SENTENCES`] 条成品上限——预取最多领先播放这么多句，内存/CPU
//!   都不会无界跑飞；打断（`abort`）后回调立刻返回 false，不再白合成。
//!
//! ## 服务器侧「下载缓冲」：[`DownloadBudget`]（内存换性能的**下载**那一半）
//!
//! 完整链路是**三段缓冲**，各管一件事（勿互相替代）：
//!
//! ```text
//! TTS 接口 ──流式下载──▶ 下载缓冲(PCM, 秒级预算) ──重采样+Opus分帧──▶ 水库(Opus, 多秒) ──▶ 实时节拍 ──▶ ESP
//!            DownloadBudget                  reservoir                DownlinkPacer
//! ```
//!
//! - **下载缓冲**（本模块）：上游 PCM 是 96KB/s（16kHz f32），比水库里的 Opus 帧大 ~50 倍，
//!   不能无界下载；但也不能「用一片取一片」——那会把流式下载退化成逐片节拍，上游一慢
//!   就直接打穿到播放。故按**音频秒数**给下载定预算：积压达到预算就**暂停下载**
//!   （`[audio].downlink_buffer_ms` 派生，见 [`crate::config::AudioConfig::download_budget_ms`]），
//!   消费（编码入水库）每取走一片就归还额度。于是上游能多快就多快，而积压永远 ≤ 预算。
//! - **水库**（`app::downlink::reservoir`）：已编码帧备多秒，吸收上游停顿。
//! - **节拍器**（`app::downlink::DownlinkPacer`）：按实时下发，保护 ESP 20 包解码队列。
//!
//! ## 已知取舍（有意为之，勿当 bug 修）
//! - **句内 PCM 暂存有界**：内存口径 = [`DownloadBudget`] 的**秒数预算**（一轮回复共享，
//!   跨句累计），通道槽位（[`PREFETCH_PCM_SLOTS`]）只是兜底。水库
//!   （`app::downlink::reservoir`）满了之后消费者停止取分片，预算随即把合成一起摁住。
//! - **同一轮里重复出现的句子不再命中结果缓存**：查缓存在 worker（上一句合成一结束就发生），
//!   写缓存由消费者在下行完成后做（要拿到编码帧）——同一轮内的重复句会再合成一次；
//!   跨轮次命中不受影响。
//!
//! [`DownlinkPacer`]: crate::app::downlink::DownlinkPacer

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};

use crate::app::hot_audio::HotAudioKey;
use crate::app::tts_cache::TtsCache;
use crate::plugins::tts::TtsEngine;

/// 预取领先播放的成品句数上限（背压）。3 已足够覆盖"在线合成短连接握手 + 服务端首帧"，
/// 又不至于把待播音频堆太多（打断时白合成的量有限）。
pub(crate) const PREFETCH_SENTENCES: usize = 3;

/// 流式合成路径的分片通道**槽位数**：只是兜底，**不是**内存口径
///（真正的内存口径是 [`DownloadBudget`] 的秒数预算）。取 256 是为了让预算先于槽位生效
/// ——常见 40~100ms 分片下 256 槽 ≈ 10~25s > 预算上限 10s。
pub(crate) const PREFETCH_PCM_SLOTS: usize = 256;

/// 一次性回传路径（AIUI 全链路 / 测试台）的分片通道容量：音频**已在内存**里，
/// 这里只是把它搬进与流式路径同形的通道，容量小一些即可（100ms 片 → 6.4s）。
pub(crate) const PREFETCH_PCM_CHUNKS: usize = 64;

/// 一片 PCM 在**下载缓冲**里占用的音频时长（毫秒，至少 1）。
///
/// 生产端（TTS 回调）与消费端（编码入水库）必须用**同一个**口径记账，故两侧都调它；
/// `sr` 由生产端捕获并随成品投递给消费端（见 [`PreparedAudio::Stream`]），保证一致。
pub(crate) fn chunk_ms(samples: usize, sr: u32) -> u64 {
    (samples as u64)
        .saturating_mul(1_000)
        .div_ceil(sr.max(1) as u64)
        .max(1)
}

/// 一轮回复的**下载缓冲预算**：按音频秒数给「TTS 流下载」定上限。
///
/// 语义（= "用服务器内存换性能" + "根据缓存需要控制下载流速"）：
/// - 生产端（TTS 回调）在 `积压 + 本片 > 预算` 时**暂停下载**（5ms 粒度自旋，同时复查
///   `abort` 与"读端是否已放弃"）——这就是**下载限速**；
/// - 消费端（编码入水库）**编码完一片就丢弃它**，[`PcmChunk`] 的 Drop 把额度还回来
///   （见下），下载随即恢复；
/// - 于是上游能多快就多快（预取领先播放），但积压**永远 ≤ 预算**：内存可算。
///
/// 预算是**一轮回复共享**的（跨句累计）。若每句各给一份，4 句在飞就是 4 倍内存；
/// 共享之后每会话 PCM 内存只由 `[audio].downlink_buffer_ms` 派生的
/// [`crate::config::AudioConfig::download_budget_ms`] 决定（默认 10s ≈ 640KB @16kHz f32）。
///
/// ⚠️ 额度归还不是"手工记账"：分片一律包在 [`PcmChunk`] 里，**Drop 即归还**。
/// 于是正常消费、打断、编码失败、通道被整个丢弃、发送端在重试中退出——任何一条路径
/// 都不会泄漏额度（漏还的后果是同一轮后续句子的下载被虚高水位**永久摁住**）。
pub(crate) struct DownloadBudget {
    /// 预算（毫秒）；至少 1 以免把下载掐死
    budget_ms: u64,
    /// 当前积压（毫秒）
    queued_ms: Mutex<(u64, usize)>,
    /// 峰值积压（毫秒）
    peak_ms: AtomicU64,
    /// 因预算而**暂停下载**的累计等待（毫秒）与次数
    throttle_ms: AtomicU64,
    throttle_events: AtomicU64,
}

/// 一轮下载缓冲的观测（诊断用）：见 `session::reply` 的每轮汇总日志。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DownloadStats {
    /// 预算（毫秒）
    pub budget_ms: u64,
    /// 峰值积压（毫秒）
    pub peak_ms: u64,
    /// 下载被限速的累计等待（毫秒）
    pub throttle_ms: u64,
    /// 下载被限速的次数
    pub throttle_events: u64,
}

impl DownloadBudget {
    /// 按预算（毫秒）建一轮共享的下载缓冲。
    pub(crate) fn new(budget_ms: u32) -> Arc<Self> {
        Arc::new(Self {
            budget_ms: (budget_ms as u64).max(1),
            queued_ms: Mutex::new((0, 0)),
            peak_ms: AtomicU64::new(0),
            throttle_ms: AtomicU64::new(0),
            throttle_events: AtomicU64::new(0),
        })
    }

    /// 生产端：为本片 PCM 预留额度。返回 `false` = 停止下载（打断 / 读端已放弃）。
    ///
    /// Large provider callbacks are sliced before reservation; a callback never
    /// enlarges either ceiling.
    #[cfg(test)]
    pub(crate) fn reserve(&self, ms: u64, abort: &AtomicBool, closed: &dyn Fn() -> bool) -> bool {
        self.reserve_bytes(ms, 0, abort, closed)
    }
    fn reserve_bytes(
        &self,
        ms: u64,
        bytes: usize,
        abort: &AtomicBool,
        closed: &dyn Fn() -> bool,
    ) -> bool {
        if ms > self.budget_ms || bytes > MAX_DOWNLOAD_BYTES {
            return false;
        }
        let limit = self.budget_ms;
        let t0 = Instant::now();
        loop {
            if abort.load(Ordering::Relaxed) || closed() {
                return false;
            }
            {
                let mut q = self.queued_ms.lock().unwrap_or_else(|e| e.into_inner());
                if q.0.saturating_add(ms) <= limit
                    && q.1.saturating_add(bytes) <= MAX_DOWNLOAD_BYTES
                {
                    q.0 += ms;
                    q.1 += bytes;
                    self.peak_ms.fetch_max(q.0, Ordering::Relaxed);
                    break;
                }
            }
            // 积压到顶 = 按预算**暂停下载**（这就是"控制下载流速"）
            std::thread::sleep(Duration::from_millis(5));
        }
        let waited = t0.elapsed().as_millis() as u64;
        if waited > 0 {
            self.throttle_ms.fetch_add(waited, Ordering::Relaxed);
            self.throttle_events.fetch_add(1, Ordering::Relaxed);
        }
        true
    }

    /// 消费端：取走一片后归还额度（多余的归还按 0 处理，不会变负）。
    #[cfg(test)]
    pub(crate) fn release(&self, ms: u64) {
        self.release_bytes(ms, 0);
    }
    fn release_bytes(&self, ms: u64, bytes: usize) {
        let mut q = self.queued_ms.lock().unwrap_or_else(|e| e.into_inner());
        q.0 = q.0.saturating_sub(ms);
        q.1 = q.1.saturating_sub(bytes);
    }

    /// 本轮下载缓冲观测。
    pub(crate) fn stats(&self) -> DownloadStats {
        DownloadStats {
            budget_ms: self.budget_ms,
            peak_ms: self.peak_ms.load(Ordering::Relaxed),
            throttle_ms: self.throttle_ms.load(Ordering::Relaxed),
            throttle_events: self.throttle_events.load(Ordering::Relaxed),
        }
    }

    /// 当前积压（毫秒）：单测与诊断用。
    #[cfg(test)]
    pub(crate) fn queued_ms(&self) -> u64 {
        self.queued_ms.lock().unwrap_or_else(|e| e.into_inner()).0
    }
}

/// 下载缓冲里的一个 PCM 分片：**Drop 即归还额度**。
///
/// 用 RAII 而不是"手工 release"是为了让额度归还不依赖任何一条退出路径的正确性：
/// 正常消费、提前打断、编码失败、接收端被丢弃、发送端在重试中退出，Drop 都会跑。
pub(crate) struct PcmChunk {
    /// 音频样本（f32，引擎输出采样率）
    pub pcm: Vec<f32>,
    /// 该片占用的下载缓冲额度（毫秒）；0 = 未记账
    ms: u64,
    bytes: usize,
    /// 归还目标；`None` = 未记账（一次性回传路径：音频已在内存，不经下载缓冲）
    budget: Option<Arc<DownloadBudget>>,
}

impl PcmChunk {
    /// Streaming path: duration AND bytes reserved; Drop returns both.
    pub(crate) fn budgeted(pcm: Vec<f32>, ms: u64, budget: Arc<DownloadBudget>) -> Self {
        Self {
            bytes: pcm.len() * std::mem::size_of::<f32>(),
            pcm,
            ms,
            budget: Some(budget),
        }
    }

    /// 一次性回传路径（AIUI 全链路 / 测试台）：音频已在内存，不占下载缓冲。
    pub(crate) fn unbudgeted(pcm: Vec<f32>) -> Self {
        Self {
            pcm,
            ms: 0,
            bytes: 0,
            budget: None,
        }
    }
}

impl Drop for PcmChunk {
    fn drop(&mut self) {
        if let Some(budget) = &self.budget {
            budget.release_bytes(self.ms, self.bytes);
        }
    }
}

/// 把 PCM 分片推入通道，并**按 [`DownloadBudget`] 控制下载流速**：
/// 积压到预算则自旋等待（同时轮询打断 / 消费者退出）。
///
/// ⚠️ 不能用 `mpsc::Sender::blocking_send`：消费者退出后它会**永久阻塞**，把合成线程
/// （`spawn_blocking` 线程池）钉死；这里 5ms 粒度复查两个退出条件，保证合成一定会收尾。
pub(crate) fn push_pcm(
    budget: &Arc<DownloadBudget>,
    tx: &mpsc::Sender<PcmChunk>,
    chunk: Vec<f32>,
    sr: u32,
    abort: &AtomicBool,
) -> bool {
    push_pcm_slice(budget, tx, &chunk, sr, abort)
}

pub(crate) const MAX_DOWNLOAD_BYTES: usize = 4 * 1024 * 1024;

fn push_pcm_slice(
    budget: &Arc<DownloadBudget>,
    tx: &mpsc::Sender<PcmChunk>,
    chunk: &[f32],
    sr: u32,
    abort: &AtomicBool,
) -> bool {
    if sr == 0 {
        return false;
    }
    let samples = ((budget.budget_ms * sr as u64 / 1000) as usize).clamp(1, MAX_DOWNLOAD_BYTES / 4);
    for part in chunk.chunks(samples) {
        if !push_pcm_part(budget, tx, part, sr, abort) {
            return false;
        }
    }
    true
}
fn push_pcm_part(
    budget: &Arc<DownloadBudget>,
    tx: &mpsc::Sender<PcmChunk>,
    chunk: &[f32],
    sr: u32,
    abort: &AtomicBool,
) -> bool {
    let ms = chunk_ms(chunk.len(), sr);
    // 先预留额度：额度不够就在这里等（= 下载被限速），而不是把 PCM 无界灌进来
    if !budget.reserve_bytes(ms, std::mem::size_of_val(chunk), abort, &|| tx.is_closed()) {
        return false;
    }
    // 预留成功起，额度就由 `item` 的 Drop 负责归还（含下面每一条提前返回路径）
    let mut item = PcmChunk::budgeted(chunk.to_vec(), ms, budget.clone());
    loop {
        match tx.try_send(item) {
            Ok(()) => return true,
            // 消费者已退出（打断/本轮结束/会话关闭）：`item` 随函数返回被 Drop → 归还额度
            Err(mpsc::error::TrySendError::Closed(_)) => return false,
            Err(mpsc::error::TrySendError::Full(v)) => {
                if abort.load(Ordering::Relaxed) {
                    return false;
                }
                item = v;
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

/// 一句的音频来源（已按会话级最新配置解析）。
pub(crate) enum PreparedAudio {
    /// 正在流式合成：`chunks` 按序给出增量 PCM 分片（引擎输出采样率 `sr`），
    /// 通道关闭后由 `done` 告知合成是否正常结束（决定该句能否写入结果缓存）。
    Stream {
        sr: u32,
        /// 增量 PCM 分片（槽位有界，见 [`PREFETCH_PCM_SLOTS`]；**内存口径**是
        /// [`DownloadBudget`] 的秒数预算——分片带额度，Drop 即归还，见 [`PcmChunk`]）
        chunks: mpsc::Receiver<PcmChunk>,
        done: oneshot::Receiver<bool>,
    },
    /// 结果缓存命中：已编码的下行 Opus 帧负载（零合成，直接按节拍下发；磁盘命中
    /// 的帧由 `TtsCache::get_async` 按会话下行格式现场编码，见其模块文档）。
    Cached(Arc<Vec<Vec<u8>>>),
}

/// 预取成品：一句原文 + 它的音频来源 + 缓存键（供调用方在完整合成后写缓存）。
pub(crate) struct PreparedSentence {
    pub text: String,
    pub audio: PreparedAudio,
    /// 结果缓存键；**0 = 不入缓存**（AIUI 云端回传音频不可复现 / 测试台直连合成）
    pub cache_key: u64,
}

/// 每句的合成来源：由调用方按**会话级最新配置**解析（引擎/音色/语速/签名在句间可热切换）。
pub(crate) struct SentenceSource {
    pub tts: Arc<dyn TtsEngine>,
    pub cache_key: u64,
    /// 生产独立 TTS 的完整身份；测试台强制合成等不可复用路径填 None。
    pub hot_audio_key: Option<HotAudioKey>,
    pub speed: f32,
    pub speaker: i32,
}

/// 启动预取 worker：句子流 → 成品流（顺序 = 播出顺序）。
///
/// `resolve` 在每句合成**之前**调用一次，让"配置热切换在新会话/新句子生效"的语义与旧实现
/// 完全一致（发音人、语速、引擎签名、缓存键都取当句的最新值）。
///
/// `budget` = 本轮**下载缓冲**（跨句共享，见 [`DownloadBudget`]）：合成回调据此控制下载流速。
///
/// `tts_gate`（缺口①）= TTS 推理闸门：**缓存未命中**才过闸（`acquire_with_abort`，排队期
/// 响应打断），permit move 进 `spawn_blocking` 闭包随推理结束归还；缓存命中零合成不过闸。
/// ⚠️ **不要**在等上一句 `prev` 时持许可——上一句在飞时先 `await` 它收尾，许可只覆盖
/// 本句自己的推理窗口。
///
/// `output_format` = 本会话的下行音频格式（会话开始时固定）：内存缓存键与磁盘命中的
/// 编码都依赖它（磁盘存的是 24k PCM，见 `app::hot_audio`）。
///
/// `first_pcm`（缺口②）= 首个**非缓存**句的首个 PCM 分片回调（TTFA 分段采集的
/// `tts_first_pcm` 埋点）；`None` = 不采集。first-only 语义由调用方闭包用 `AtomicBool`
/// 实现（每分片一次原子读，无锁），本模块不 import [`crate::app::events`] 类型——
/// 保持通用流水线与观测解耦。
///
/// 返回的接收端被丢弃（或 abort 置位）后，worker 会在当前句合成收尾后自行退出，
/// 不留 detached 任务。
#[allow(clippy::too_many_arguments)] // 流水线参数（闸门/埋点）与既有入参同性质
pub(crate) fn spawn_sentence_prefetch<F>(
    abort: Arc<AtomicBool>,
    cache: Arc<TtsCache>,
    budget: Arc<DownloadBudget>,
    tts_gate: crate::app::inference::GateHandle,
    first_pcm: Option<Arc<dyn Fn() + Send + Sync>>,
    output_format: crate::app::audio::media::AudioFormat,
    mut resolve: F,
    mut sentences: mpsc::UnboundedReceiver<String>,
) -> mpsc::Receiver<PreparedSentence>
where
    F: FnMut(&str) -> SentenceSource + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<PreparedSentence>(PREFETCH_SENTENCES);
    tokio::spawn(async move {
        // 上一句的合成任务：启动下一句之前先等它收尾（合成串行），但**不等它播完**——
        // 这正是预取的收益点：等待发生在播放进行中，而不是播放结束之后。
        let mut prev: Option<(String, tokio::task::JoinHandle<anyhow::Result<()>>)> = None;
        while let Some(sentence) = sentences.recv().await {
            if abort.load(Ordering::Relaxed) {
                break;
            }
            // 等上一句收尾时**不持许可**（见函数文档）：许可只覆盖本句自己的推理窗口。
            if let Some((text, handle)) = prev.take() {
                log_synth_result(&text, handle.await);
            }
            if abort.load(Ordering::Relaxed) {
                break;
            }
            let SentenceSource {
                tts,
                cache_key,
                hot_audio_key,
                speed,
                speaker,
            } = resolve(&sentence);
            if let Some(identity) = hot_audio_key {
                if cache_key != 0 && identity.mem_key(output_format) == cache_key {
                    cache.register_identity(identity, cache_key);
                }
            }
            // 先内存、再 blocking 读盘（读盘命中含 PCM→帧编码）；命中均零合成、不过推理闸门。
            let hit = cache.get_async(cache_key, output_format).await;
            // 等 IO 期间也可能已打断/消费者关闭，不再启动新合成或投递旧轮音频。
            if abort.load(Ordering::Relaxed) || tx.is_closed() {
                break;
            }
            if let Some(hit) = hit {
                tracing::debug!("TTS 缓存命中（预取 {} 帧）", hit.opus_frames.len());
                let item = PreparedSentence {
                    text: sentence,
                    audio: PreparedAudio::Cached(hit.opus_frames),
                    cache_key,
                };
                if tx.send(item).await.is_err() {
                    break;
                }
                continue;
            }
            // 推理闸门（缺口①）：缓存未命中才过闸排队（abort 感知：打断后不白等）。
            let Some(permit) = tts_gate.acquire_with_abort(&abort).await else {
                break; // 已打断：本轮不再合成
            };
            let sr = tts.output_sample_rate();
            let (chunk_tx, chunk_rx) = mpsc::channel::<PcmChunk>(PREFETCH_PCM_SLOTS);
            let (done_tx, done_rx) = oneshot::channel::<bool>();
            // 合成用文本 + 日志用文本各持一份（`sentence` 本体要进成品投递给消费者）
            let synth_text = sentence.clone();
            let log_text = sentence.clone();
            let abort_cb = abort.clone();
            let budget_cb = budget.clone();
            // 消费者消失（打断后 `prepared` 被丢弃、或本轮结束）也要能掐断在飞合成：
            // 只靠 abort 标志不够——**新一轮回复会把 abort 复位**，上一轮残留的合成
            // 就会白跑（在线合成要白白建一条短连接、本地合成白烧 CPU）。
            let tx_cb = tx.clone();
            // 缺口②：首个 PCM 分片回调（first-only 由调用方闭包保证）。
            let first_pcm_cb = first_pcm.clone();
            let handle = tokio::task::spawn_blocking(move || {
                // 推理闸门 permit：RAII——闭包退出（正常结束/回调取消收尾/panic 展开）即归还。
                let _gate_permit = permit;
                let callback_ok = Arc::new(AtomicBool::new(true));
                let callback_ok_cb = callback_ok.clone();
                let r = tts.synthesize_stream(
                    &synth_text,
                    speed,
                    speaker,
                    Box::new(move |actual_sr, chunk| {
                        let accepted = if abort_cb.load(Ordering::Relaxed)
                            || tx_cb.is_closed()
                            || actual_sr != sr
                        {
                            false
                        } else {
                            if let Some(hook) = first_pcm_cb.as_ref() {
                                hook();
                            }
                            push_pcm_slice(&budget_cb, &chunk_tx, chunk, sr, &abort_cb)
                        };
                        if !accepted {
                            callback_ok_cb.store(false, Ordering::Release);
                        }
                        accepted
                    }),
                );
                // 合成是否正常结束决定该句能否写入缓存（残缺句不入缓存）
                let _ = done_tx.send(r.is_ok() && callback_ok.load(Ordering::Acquire));
                r
            });
            let item = PreparedSentence {
                text: sentence,
                audio: PreparedAudio::Stream {
                    sr,
                    chunks: chunk_rx,
                    done: done_rx,
                },
                cache_key,
            };
            if tx.send(item).await.is_err() {
                // 消费者已退出（打断/会话结束）：等当前句收尾再退出
                if let Ok(Err(e)) = handle.await {
                    tracing::warn!("句子合成失败: {e:#}");
                }
                break;
            }
            prev = Some((log_text, handle));
        }
        // 收尾：等最后一句合成结束（错误照样只记日志，对话不受影响）
        if let Some((text, handle)) = prev.take() {
            log_synth_result(&text, handle.await);
        }
    });
    rx
}

/// 合成结果统一收口：失败只 `warn`（语音场景由上层兜底话术负责，不能让合成错误打断会话）。
fn log_synth_result(text: &str, result: Result<anyhow::Result<()>, tokio::task::JoinError>) {
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!("句子合成失败（{text}）: {e:#}"),
        Err(e) => tracing::warn!("句子合成任务异常（{text}）: {e}"),
    }
}

#[cfg(test)]
mod tests;
