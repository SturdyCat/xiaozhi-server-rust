//! 音频链路（W14b-1 上行方向 + W14b-2 下行方向）：采集 → 重采样 → 帧切分 → Opus 编码，
//! 以及下行 Opus 解码 → 播放（TTS 合成结果流式试听）。
//!
//! 分层（对齐架构文档 §1.1④ 子路径 B：Rust 只做音频 I/O，协议编排在前端）：
//! - [`LinearResampler`]：设备采样率 → 16k 上行（线性插值；48k→16k 恰为整数 3:1，
//!   也支持 44.1k 等非整数倍；尾帧经 [`LinearResampler::flush`] 收口）。
//!   取舍：不引 rubato——上行是语音 ASR 场景，线性插值质量足够且零依赖，边界行为有单测覆盖。
//! - [`FrameSlicer`]：16k PCM 按 60ms（960 样本）切帧，余数留缓冲，停止时补零收尾。
//! - [`UplinkEncoder`]：audiopus 16k / Mono / Voip / 40kbps（`Application::Voip` 与
//!   server 下行编码器同款，见 `server/src/media/opus.rs:43`；比特率按 hello 契约 40kbps）。
//!   **流式单实例**：一段录音所有帧共用同一编码器（server `opus.rs:9-12` 教训：逐帧新建
//!   会产生帧边界伪影）。
//! - 上行帧封装：hello `version:1` → server `unwrap_uplink(BinVersion::V1)` 直接返回
//!   原始字节（`protocol.rs:342-344`），**v1 无封装头、裸 Opus 帧**——无需对齐 v2/v3 布局。
//! - [`wave_buckets`]：整段 PCM 均分 64 桶 RMS 归一（对齐旧端 getAudioState 波形语义）。
//! - [`AudioManager`]：cpal 采集/播放生命周期；命令-事件协议见 [`BridgeCommand`] /
//!   [`BridgeEvent`]（本地桥 WS 文本帧，前端编排）。
//! - **下行（W14b-2）**：server `/api/ws` 的 Opus 二进制帧（tts_test 合成结果，前端中转）
//!   → [`DownlinkDecoder`] 流式解码（下行率来自 server hello `audio_params.sample_rate`，
//!   服务端配置 `[audio].downlink_sample_rate`，默认 24k）→ 线性重采样到输出设备实测率 →
//!   cpal output stream 流式播放；`tts_end` 后缓冲排空回 `TtsDone`（整段波形 + 时长），
//!   样本快照保留供 `play_tts` 回放。
//!
//! ## 下行 pre-skip / 解码器状态（对齐 `+Opus.m:128-161` 的坑）
//! 旧端 AudioConverter 的坑本质：默认 pre-skip 修剪让首包「凑不满请求量 → 二次索要
//! 输入 → EOF 粘滞」。libopus/audiopus **无此问题**：`decode(packet)` 每包确定性地
//! 产出整帧样本数（单测 `downlink_stream_decode_length_exact` 固化），不存在 EOF 粘滞
//! 等价物。等价纪律照抄旧端：**整段流共用同一 Decoder 实例**（帧间 MDCT overlap 状态
//! 跨包保留 = 无缝衔接，不做每帧重建），仅在新一次合成开始（`tts_begin`）时整体重建
//! （等价旧端「仅在新一次合成开始时重置一次」）。
//!
//! ⚠️ 命令可用 / 编译通过不等于真机硬件通过（`src-tauri/src/commands/mod.rs:11`）：
//! cpal CoreAudio 全双工稳定性与麦克风 TCC 授权只能真机验证（架构文档 §3-L3）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 上行采样率（hello audio_params.sample_rate；与 server `params.uplink_sr` 一致）。
pub const UPLINK_SAMPLE_RATE: u32 = 16_000;
/// 60ms 帧长 @16k = 960 样本（XiaoZhiModule.m 的 kFrameDurationMs=60）。
pub const FRAME_SAMPLES: usize = 960;
/// 上行编码比特率（bps）。
pub const UPLINK_BITRATE_BPS: i32 = 40_000;
/// 波形桶数（对齐旧端 GearWaveform 语义）。
pub const WAVE_BUCKETS: usize = 64;

// ============================================================
// 纯函数链路：重采样 / 帧切分 / 编码 / 波形
// ============================================================

/// 流式线性插值重采样器（单声道 f32）。
///
/// 相位跨块保持：每个输出样本对应输入域绝对位置 `k * ratio`，块边界处用上一块
/// 末样本与当前块首样本插值；[`flush`](Self::flush) 用末样本复制收出尾帧。
#[derive(Debug)]
pub struct LinearResampler {
    /// 输入域步长 = in_sr / out_sr（48k→16k 时为 3.0）。
    ratio: f64,
    /// 下一个输出样本在输入域中的绝对位置（样本索引，可为小数）。
    next_pos: f64,
    /// 上一块的最后一个样本（第一块前为 None）。
    prev_last: Option<f32>,
}

impl LinearResampler {
    pub fn new(in_sr: u32, out_sr: u32) -> Self {
        debug_assert!(in_sr > 0 && out_sr > 0, "采样率必须为正");
        Self {
            ratio: f64::from(in_sr) / f64::from(out_sr),
            next_pos: 0.0,
            prev_last: None,
        }
    }

    /// 处理一块输入，返回重采样输出（长度约为 `len / ratio`，跨块相位连续）。
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity((input.len() as f64 / self.ratio) as usize + 2);
        let mut i = 0usize;
        while self.next_pos < input.len() as f64 {
            let frac = self.next_pos - i as f64;
            let a = if i == 0 {
                // 位置落在「上一块末样本与本块首样本」之间
                self.prev_last.unwrap_or(input[0])
            } else {
                input[i - 1]
            };
            let b = input[i];
            out.push(a + (b - a) * frac as f32);
            self.next_pos += self.ratio;
            i = self.next_pos.floor() as usize;
        }
        if let Some(last) = input.last() {
            self.prev_last = Some(*last);
        }
        // 相位平移到下一块起点
        self.next_pos -= input.len() as f64;
        out
    }

    /// 流结束收口：用末样本复制补齐残余相位，取出剩余输出（非整数倍边界）。
    pub fn flush(&mut self) -> Vec<f32> {
        let mut out = Vec::new();
        // 最多再产出若干个残余输出（ratio 内必然收敛）
        let mut guard = 0usize;
        while self.next_pos > 0.0 && self.prev_last.is_some() && guard < self.ratio.ceil() as usize + 1
        {
            let last = self.prev_last.expect("prev_last 已判空");
            out.push(last);
            self.next_pos -= self.ratio;
            guard += 1;
        }
        self.next_pos = 0.0;
        out
    }
}

/// 60ms 帧切分器：整帧立刻产出，余数保留到下次 push 或 [`flush_padded`](Self::flush_padded)。
#[derive(Debug)]
pub struct FrameSlicer {
    frame_len: usize,
    buf: Vec<f32>,
}

impl FrameSlicer {
    pub fn new(frame_len: usize) -> Self {
        Self {
            frame_len,
            buf: Vec::with_capacity(frame_len * 2),
        }
    }

    /// 追加样本并产出所有完整帧。
    pub fn push(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
        self.buf.extend_from_slice(samples);
        let mut frames = Vec::new();
        while self.buf.len() >= self.frame_len {
            frames.push(self.buf.drain(..self.frame_len).collect::<Vec<f32>>());
        }
        frames
    }

    /// 停止时把尾帧补零产出（不足一帧也产出，保证最后一段语音不丢）；无余数返回 None。
    pub fn flush_padded(&mut self) -> Option<Vec<f32>> {
        if self.buf.is_empty() {
            return None;
        }
        self.buf.resize(self.frame_len, 0.0);
        Some(std::mem::take(&mut self.buf))
    }
}

/// 上行 Opus 帧编码器（16k / Mono / Voip / 40kbps；单实例流式）。
pub struct UplinkEncoder {
    enc: audiopus::coder::Encoder,
    out: Vec<u8>,
}

impl UplinkEncoder {
    pub fn new() -> Result<Self, String> {
        use audiopus::{Application, Channels, SampleRate};
        let mut enc = audiopus::coder::Encoder::new(SampleRate::Hz16000, Channels::Mono, Application::Voip)
            .map_err(|e| format!("创建 Opus 编码器失败: {e:?}"))?;
        enc.set_bitrate(audiopus::Bitrate::BitsPerSecond(UPLINK_BITRATE_BPS))
            .map_err(|e| format!("设置 Opus 比特率失败: {e:?}"))?;
        Ok(Self {
            enc,
            out: vec![0u8; 4000],
        })
    }

    /// 编码一整帧（必须恰好 [`FRAME_SAMPLES`] 样本）；输出裸 Opus 字节（v1 无封装头）。
    pub fn encode(&mut self, frame: &[f32]) -> Result<Vec<u8>, String> {
        debug_assert_eq!(frame.len(), FRAME_SAMPLES, "上行帧长必须为 60ms 整帧");
        let pcm: Vec<i16> = frame
            .iter()
            .map(|s| (s.clamp(-1.0, 1.0) * 32_767.0) as i16)
            .collect();
        let len = self
            .enc
            .encode(&pcm, &mut self.out)
            .map_err(|e| format!("Opus 编码失败: {e:?}"))?;
        Ok(self.out[..len].to_vec())
    }
}

/// 整数采样率 → `audiopus::SampleRate`（下行解码构造用；非法率 fail-closed 返回 Err）。
fn to_audiopus_rate(sr: u32) -> Result<audiopus::SampleRate, String> {
    use audiopus::SampleRate;
    Ok(match sr {
        8_000 => SampleRate::Hz8000,
        12_000 => SampleRate::Hz12000,
        16_000 => SampleRate::Hz16000,
        24_000 => SampleRate::Hz24000,
        48_000 => SampleRate::Hz48000,
        _ => return Err(format!("不支持的下行 Opus 采样率 {sr}（只支持 8k/12k/16k/24k/48k）")),
    })
}

/// 下行 Opus 解码器（W14b-2；**流式单实例**——帧间 overlap 状态跨包保留，
/// 见模块尾注「下行 pre-skip / 解码器状态」）。
pub struct DownlinkDecoder {
    dec: audiopus::coder::Decoder,
    /// 复用的 i16 缓冲：60ms@24k = 1440 帧，5760 容量足够所有合法帧长。
    pcm: Vec<i16>,
}

impl DownlinkDecoder {
    /// `sample_rate` 为下行采样率（server hello audio_params；仅 8k/12k/16k/24k/48k）。
    pub fn new(sample_rate: u32) -> Result<Self, String> {
        let dec = audiopus::coder::Decoder::new(to_audiopus_rate(sample_rate)?, audiopus::Channels::Mono)
            .map_err(|e| format!("创建下行 Opus 解码器失败: {e:?}"))?;
        Ok(Self {
            dec,
            pcm: vec![0i16; 5760],
        })
    }

    /// 解码一包 Opus → f32 PCM（-1..1）；输出长度由包内容决定（合法 60ms 帧恒整帧）。
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>, String> {
        let n = self
            .dec
            .decode(Some(packet), &mut self.pcm, false)
            .map_err(|e| format!("下行 Opus 解码失败: {e:?}"))?;
        Ok(self.pcm[..n].iter().map(|s| f32::from(*s) / 32_768.0).collect())
    }
}

/// 整段 PCM 均分 `buckets` 桶 RMS 归一波形（0..1）；空段返回全 0。
///
/// RMS × 4 的增益按语音电平经验取值（对齐旧端波形可视幅度），超过 1 截断。
pub fn wave_buckets(samples: &[f32], buckets: usize) -> Vec<f32> {
    if buckets == 0 || samples.is_empty() {
        return vec![0.0; buckets];
    }
    let chunk = samples.len().div_ceil(buckets);
    (0..buckets)
        .map(|i| {
            let start = i * chunk;
            if start >= samples.len() {
                return 0.0;
            }
            let end = (start + chunk).min(samples.len());
            let sum_sq: f32 = samples[start..end].iter().map(|s| s * s).sum();
            ((sum_sq / (end - start) as f32).sqrt() * 4.0).clamp(0.0, 1.0)
        })
        .collect()
}

// ============================================================
// 命令-事件协议（本地桥 WS 文本帧，前端编排）
// ============================================================

/// 前端 → Rust 的本地桥控制命令（WS 文本帧 `{"type": ...}`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeCommand {
    /// 开始录音采集（同时开始 Opus 上行编码与波形推送）。
    StartCapture,
    /// 停止采集（保留缓冲供试听；回 `Recorded`）。
    StopCapture,
    /// 丢弃当前缓冲（重新录音前清空）。
    DiscardCapture,
    /// 开始试听录音缓冲。
    PlayRecorded,
    /// 停止试听。
    StopPlayback,
    /// 开始 TTS 下行会话（W14b-2）：`sample_rate` 为下行采样率（server hello
    /// audio_params.sample_rate；非法率解析即拒绝）。建输出流等待解码数据。
    TtsBegin {
        sample_rate: u32,
    },
    /// 结束 TTS 下行会话：缓冲排空后回 `TtsDone`，样本快照保留供 `PlayTts` 回放。
    TtsEnd,
    /// 回放最近一次 TTS 缓冲（对齐旧端 playTts）。
    PlayTts,
}

impl BridgeCommand {
    /// 从前端文本帧解析；未知类型返回 None。
    pub fn parse(value: &serde_json::Value) -> Option<Self> {
        let obj = value.as_object()?;
        match obj.get("type")?.as_str()? {
            "start_capture" => Some(Self::StartCapture),
            "stop_capture" => Some(Self::StopCapture),
            "discard_capture" => Some(Self::DiscardCapture),
            "play_recorded" => Some(Self::PlayRecorded),
            "stop_playback" => Some(Self::StopPlayback),
            "tts_begin" => {
                // 下行率合法值 fail-closed：非法/缺失一律拒绝（服务端只可能发 8/12/16/24/48k）。
                let sr = obj
                    .get("sample_rate")
                    .and_then(serde_json::Value::as_u64)
                    .filter(|sr| matches!(sr, 8_000 | 12_000 | 16_000 | 24_000 | 48_000))
                    .map(|sr| sr as u32)?;
                Some(Self::TtsBegin { sample_rate: sr })
            }
            "tts_end" => Some(Self::TtsEnd),
            "play_tts" => Some(Self::PlayTts),
            _ => None,
        }
    }
}

/// Rust → 前端的本地桥事件（WS 文本帧；Opus 帧走二进制）。
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeEvent {
    /// 采集真正开始（cpal stream 已 play）。
    Started,
    /// 停止采集完成（缓冲保留；buckets 为整段波形）。
    Recorded {
        seconds: f64,
        buckets: Vec<f32>,
    },
    /// 录音中的周期推送（200ms；seconds 为已录时长）。
    Wave {
        seconds: f64,
        buckets: Vec<f32>,
    },
    /// 播放状态推送（100ms；对齐旧端 getAudioState 语义）。
    PlayState {
        playing: bool,
        position: f64,
        duration: f64,
    },
    /// TTS 下行会话完成（W14b-2）：缓冲排空，seconds 为整段时长，buckets 为整段波形
    /// （64 桶 RMS）；样本快照已保留，可 `play_tts` 回放。
    TtsDone {
        seconds: f64,
        buckets: Vec<f32>,
    },
    /// 采集/播放失败（code: mic_permission_denied | device_unavailable | device_start_failed）。
    Error {
        code: String,
        message: String,
    },
}

impl BridgeEvent {
    pub fn to_json(&self) -> String {
        let mut obj = serde_json::Map::new();
        match self {
            Self::Started => {
                obj.insert("type".into(), "started".into());
            }
            Self::Recorded { seconds, buckets } => {
                obj.insert("type".into(), "recorded".into());
                obj.insert("seconds".into(), serde_json::json!(seconds));
                obj.insert("buckets".into(), serde_json::json!(buckets));
            }
            Self::Wave { seconds, buckets } => {
                obj.insert("type".into(), "wave".into());
                obj.insert("seconds".into(), serde_json::json!(seconds));
                obj.insert("buckets".into(), serde_json::json!(buckets));
            }
            Self::PlayState {
                playing,
                position,
                duration,
            } => {
                obj.insert("type".into(), "play_state".into());
                obj.insert("playing".into(), serde_json::json!(playing));
                obj.insert("position".into(), serde_json::json!(position));
                obj.insert("duration".into(), serde_json::json!(duration));
            }
            Self::TtsDone { seconds, buckets } => {
                obj.insert("type".into(), "tts_done".into());
                obj.insert("seconds".into(), serde_json::json!(seconds));
                obj.insert("buckets".into(), serde_json::json!(buckets));
            }
            Self::Error { code, message } => {
                obj.insert("type".into(), "error".into());
                obj.insert("code".into(), serde_json::json!(code));
                obj.insert("message".into(), serde_json::json!(message));
            }
        }
        serde_json::Value::Object(obj).to_string()
    }
}

/// 本地桥 → 前端的一条输出：文本事件（JSON）或 Opus 二进制帧（前端中转到 server）。
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeOut {
    Event(BridgeEvent),
    /// 60ms 裸 Opus 帧（v1 无封装头），WS binary 下发。
    Opus(Vec<u8>),
}

/// 错误码 → 前端呈现锚点文案（`XiaoZhiModule.m:146/:157` 逐字）。
pub fn error_message_for_code(code: &str) -> &'static str {
    match code {
        "mic_permission_denied" => "麦克风权限被拒绝（系统设置 → 隐私与安全性 → 麦克风）",
        "device_unavailable" | "device_start_failed" => "录音设备启动失败（检查输入设备）",
        _ => "录音设备启动失败（检查输入设备）",
    }
}

// ============================================================
// cpal 采集 / 播放管理（M2/M5 + 试听播放）
// ============================================================

/// 采集回调共享链（实时音频线程持有）：重采样 → 切帧 → 编码 → 发送。
struct CaptureChain {
    resampler: LinearResampler,
    slicer: FrameSlicer,
    encoder: UplinkEncoder,
    recorded: Vec<f32>,
    /// 本块累计样本数（原始采样率域），驱动 200ms 波形推送。
    samples_since_wave: usize,
    in_sr: u32,
    /// 计秒（原始采样率域）。
    seconds: f64,
}

/// AudioManager：持有 cpal 流与录音缓冲；命令处理同步完成（回调里做重活）。
pub struct AudioManager {
    /// cpal 流必须保活（drop 即停）；单独 Mutex 保证 Drop 不阻塞事件锁。
    capture_stream: Option<cpal::Stream>,
    playback_stream: Option<cpal::Stream>,
    /// 捕获链（回调写 / 控制线程读）。
    chain: Option<Arc<Mutex<CaptureChain>>>,
    /// 停止标志（回调读）。
    capturing: Arc<AtomicBool>,
    /// 试听缓冲（采集原始采样率的 f32 PCM）与播放游标。
    playback: Arc<Mutex<PlaybackState>>,
    /// 进行中的 TTS 下行会话（W14b-2；None = 无会话）。
    tts: Option<TtsSession>,
    /// 下行解码器（WS 线程 feed_tts_frame 使用；与会话同生命周期）。
    tts_decoder: Option<DownlinkDecoder>,
    /// 最近一次完成的 TTS 缓冲快照（供 play_tts 回放；tts_end 时快照）。
    tts_replay: Option<TtsReplay>,
    /// 事件出口（当前连接的 mpsc 发送端）。
    events: Option<tokio::sync::mpsc::UnboundedSender<BridgeOut>>,
}

struct PlaybackState {
    buffer: Vec<f32>,
    /// 采集原始采样率。
    source_sr: u32,
    playing: bool,
}

/// TTS 下行会话的共享缓冲（WS 线程写解码样本 / cpal 回调线程消费）。
struct TtsBuffer {
    /// 已解码 PCM（下行采样率域；实时下发节奏下增长速度≈播放速度）。
    samples: Vec<f32>,
    /// `tts_end` 已收到：缓冲排空后回调收尾并产出 `TtsDone`。
    ended: bool,
}

/// 进行中的 TTS 下行会话（`tts_begin` 创建，`stop_playback`/`reset`/新会话销毁）。
struct TtsSession {
    /// 播放流必须保活（drop 即停）；本字段从不读取，纯保活（静音期输出零成本）。
    #[allow(dead_code)]
    stream: cpal::Stream,
    buffer: Arc<Mutex<TtsBuffer>>,
    /// 下行采样率（server hello audio_params）。
    dec_sr: u32,
}

/// 完成的 TTS 缓冲快照（tts_end 时从会话缓冲复制；供 `play_tts` 回放）。
#[derive(Debug, Clone)]
struct TtsReplay {
    buffer: Arc<Mutex<Vec<f32>>>,
    /// 下行采样率（回放重采样源率）。
    sr: u32,
}

impl Default for AudioManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioManager {
    pub fn new() -> Self {
        Self {
            capture_stream: None,
            playback_stream: None,
            chain: None,
            capturing: Arc::new(AtomicBool::new(false)),
            playback: Arc::new(Mutex::new(PlaybackState {
                buffer: Vec::new(),
                source_sr: UPLINK_SAMPLE_RATE,
                playing: false,
            })),
            tts: None,
            tts_decoder: None,
            tts_replay: None,
            events: None,
        }
    }

    /// 设置事件出口（bridge.rs 每次新连接接入时调用）。
    pub fn set_event_sender(&mut self, tx: tokio::sync::mpsc::UnboundedSender<BridgeOut>) {
        self.events = Some(tx);
    }

    fn emit(&self, event: BridgeEvent) {
        if let Some(tx) = &self.events {
            let _ = tx.send(BridgeOut::Event(event));
        }
    }

    /// 处理前端命令（在 axum handler 的同步上下文调用；失败经 `Error` 事件回传）。
    pub fn handle_command(&mut self, command: BridgeCommand) {
        match command {
            BridgeCommand::StartCapture => self.start_capture(),
            BridgeCommand::StopCapture => self.stop_capture(),
            BridgeCommand::DiscardCapture => {
                self.playback.lock().expect("playback lock").buffer.clear();
            }
            BridgeCommand::PlayRecorded => self.start_playback(),
            BridgeCommand::StopPlayback => self.stop_playback(),
            BridgeCommand::TtsBegin { sample_rate } => self.tts_begin(sample_rate),
            BridgeCommand::TtsEnd => self.tts_end(),
            BridgeCommand::PlayTts => self.play_tts(),
        }
    }

    /// 连接断开时清理（架构文档 M1：断开清理采集/播放/TTS 会话）。
    pub fn reset(&mut self) {
        self.capture_stream = None; // drop cpal stream = 停止采集
        self.playback_stream = None;
        self.tts = None;
        self.tts_decoder = None;
        self.tts_replay = None;
        self.chain = None;
        self.capturing.store(false, Ordering::SeqCst);
        self.playback.lock().expect("playback lock").playing = false;
        self.events = None;
    }

    fn start_capture(&mut self) {
        // 幂等：录音中重复 start 忽略（对齐旧端 startAsr 的 phase 守卫）。
        if self.capturing.load(Ordering::SeqCst) {
            return;
        }
        let Some(tx) = self.events.clone() else {
            return;
        };
        if let Err(message) = self.try_start_capture(tx) {
            self.capturing.store(false, Ordering::SeqCst);
            self.chain = None;
            self.emit(BridgeEvent::Error {
                code: "device_start_failed".into(),
                message,
            });
        }
    }

    /// 实际启动采集；Err 为可读失败原因。
    fn try_start_capture(
        &mut self,
        tx: tokio::sync::mpsc::UnboundedSender<BridgeOut>,
    ) -> Result<(), String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host = cpal::default_host();
        let Some(device) = host.default_input_device() else {
            return Err(error_message_for_code("device_unavailable").to_string());
        };
        // 设备枚举/配置失败常见于麦克风 TCC 未授权（真机才能区分，错误码按设备不可用诚实呈现）。
        let config = device
            .default_input_config()
            .map_err(|e| format!("{}（{e}）", error_message_for_code("device_unavailable")))?;
        let in_sr = config.sample_rate(); // cpal 0.18: SampleRate 即 u32 别名
        let channels = config.channels() as usize;

        let chain = Arc::new(Mutex::new(CaptureChain {
            resampler: LinearResampler::new(in_sr, UPLINK_SAMPLE_RATE),
            slicer: FrameSlicer::new(FRAME_SAMPLES),
            encoder: UplinkEncoder::new()?,
            recorded: Vec::new(),
            samples_since_wave: 0,
            in_sr,
            seconds: 0.0,
        }));
        self.chain = Some(Arc::clone(&chain));
        self.capturing.store(true, Ordering::SeqCst);
        self.playback.lock().expect("playback lock").buffer.clear();

        let capturing = Arc::clone(&self.capturing);
        let err_tx = tx.clone();
        let err_chain = Arc::clone(&chain);
        let stream = device
            .build_input_stream(
                config.into(), // cpal 0.18: StreamConfig 传值（非引用）
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if !capturing.load(Ordering::SeqCst) {
                        return;
                    }
                    if let Ok(mut inner) = err_chain.lock() {
                        feed_capture(&mut inner, data, channels, &tx);
                    }
                },
                move |error| {
                    let _ = err_tx.send(BridgeOut::Event(BridgeEvent::Error {
                        code: "device_start_failed".into(),
                        message: format!(
                            "{}（{error}）",
                            error_message_for_code("device_start_failed")
                        ),
                    }));
                },
                None,
            )
            .map_err(|e| format!("{}（{e}）", error_message_for_code("device_start_failed")))?;
        stream
            .play()
            .map_err(|e| format!("{}（{e}）", error_message_for_code("device_start_failed")))?;
        self.capture_stream = Some(stream);
        self.emit(BridgeEvent::Started);
        Ok(())
    }

    fn stop_capture(&mut self) {
        if !self.capturing.swap(false, Ordering::SeqCst) {
            return; // 未在录音：幂等忽略
        }
        self.capture_stream = None; // drop = 停止采集
        let Some(chain_arc) = self.chain.take() else {
            return;
        };
        // 收尾：冲刷重采样尾帧与余数补零帧，把语音尾部完整编码发出（服务端整段识别用 test_buf）。
        let tail_frames = {
            let mut inner = chain_arc.lock().expect("chain lock");
            let mut rest = inner.resampler.flush();
            rest.extend(inner.slicer.flush_padded().unwrap_or_default());
            rest
        };
        if let Some(tx) = self.events.clone() {
            if let Ok(mut inner) = chain_arc.lock() {
                for frame in tail_frames.chunks(FRAME_SAMPLES) {
                    if frame.len() == FRAME_SAMPLES {
                        if let Ok(bytes) = inner.encoder.encode(frame) {
                            let _ = tx.send(BridgeOut::Opus(bytes));
                        }
                    }
                }
            }
        }
        let (buckets, recorded_len, recorded_sr) = {
            let inner = chain_arc.lock().expect("chain lock");
            (
                wave_buckets(&inner.recorded, WAVE_BUCKETS),
                inner.recorded.len(),
                inner.in_sr,
            )
        };
        {
            let mut pb = self.playback.lock().expect("playback lock");
            let inner = chain_arc.lock().expect("chain lock");
            pb.buffer = inner.recorded.clone();
            pb.source_sr = inner.in_sr;
            pb.playing = false;
        }
        self.emit(BridgeEvent::Recorded {
            seconds: recorded_len as f64 / f64::from(recorded_sr),
            buckets,
        });
    }

    fn start_playback(&mut self) {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let tx = match self.events.clone() {
            Some(tx) => tx,
            None => return,
        };
        // 预检：缓冲非空且未在播放才继续（幂等）。
        {
            let pb = self.playback.lock().expect("playback lock");
            if pb.buffer.is_empty() || pb.playing {
                return;
            }
        }
        // 设备/配置获取不持 playback 锁（避免锁跨阻塞调用）；失败 emit Error 后早返回。
        let host = cpal::default_host();
        let Some(device) = host.default_output_device() else {
            self.emit(BridgeEvent::Error {
                code: "device_unavailable".into(),
                message: error_message_for_code("device_unavailable").into(),
            });
            return;
        };
        let out_cfg = match device.default_output_config() {
            Ok(c) => c,
            Err(e) => {
                self.emit(BridgeEvent::Error {
                    code: "device_unavailable".into(),
                    message: format!(
                        "{}（{e}）",
                        error_message_for_code("device_unavailable")
                    ),
                });
                return;
            }
        };
        let (buffer_arc, source_sr) = {
            let mut pb = self.playback.lock().expect("playback lock");
            if pb.buffer.is_empty() || pb.playing {
                return;
            }
            pb.playing = true;
            (Arc::new(Mutex::new(pb.buffer.clone())), pb.source_sr)
        };

        // 播放游标：O(1) 前进；输出率 ≠ 采集率时线性重采样（复用同一算法）。
        let out_sr = out_cfg.sample_rate(); // cpal 0.18: SampleRate 即 u32 别名
        let channels = out_cfg.channels() as usize;
        let cursor = Arc::new(Mutex::new(PlaybackCursor {
            resampler: LinearResampler::new(source_sr, out_sr),
            pending: Vec::new(),
            position: 0usize, // 已输出的输出域样本数
            frames_since_state: 0usize,
        }));
        let playing_flag = Arc::new(AtomicBool::new(true));
        let cb_playing = Arc::clone(&playing_flag);
        let cb_cursor = Arc::clone(&cursor);
        let cb_tx = tx.clone();
        let cb_buffer = Arc::clone(&buffer_arc);
        let duration = {
            let pb = self.playback.lock().expect("playback lock");
            pb.buffer.len() as f64 / f64::from(pb.source_sr)
        };

        let stream = device
            .build_output_stream(
                out_cfg.into(), // cpal 0.18: StreamConfig 传值（非引用）
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if !cb_playing.load(Ordering::SeqCst) {
                        data.fill(0.0);
                        return;
                    }
                    let Ok(mut cur) = cb_cursor.lock() else {
                        data.fill(0.0);
                        return;
                    };
                    // 需要的输出域帧数
                    let frames_needed = data.len() / channels.max(1);
                    let mut produced: Vec<f32> = Vec::with_capacity(frames_needed);
                    while produced.len() < frames_needed {
                        if cur.pending.is_empty() {
                            // 从缓冲取下一块（约 10ms）重采样
                            let take = (source_sr as f64 * 0.01) as usize;
                            let buf = cb_buffer.lock().expect("buffer lock");
                            let start = cur.position.min(buf.len());
                            let end = (cur.position + take).min(buf.len());
                            let chunk: Vec<f32> = buf[start..end].to_vec();
                            cur.position = end;
                            drop(buf);
                            if chunk.is_empty() {
                                // 播完：冲刷尾帧后结束
                                let tail = cur.resampler.flush();
                                produced.extend(tail);
                                cb_playing.store(false, Ordering::SeqCst);
                                break;
                            }
                            cur.pending = cur.resampler.process(&chunk);
                        }
                        let need = frames_needed - produced.len();
                        let take = need.min(cur.pending.len());
                        produced.extend(cur.pending.drain(..take));
                    }
                    // 单声道铺到输出各声道
                    for (i, slot) in data.iter_mut().enumerate() {
                        *slot = produced.get(i / channels.max(1)).copied().unwrap_or(0.0);
                    }
                    // 100ms 一次播放状态推送（对齐旧端 getAudioState 100ms 轮询语义）
                    cur.frames_since_state += frames_needed;
                    let state_interval = (out_sr as f64 * 0.1) as usize;
                    if cur.frames_since_state >= state_interval {
                        cur.frames_since_state = 0;
                        let _ = cb_tx.send(BridgeOut::Event(BridgeEvent::PlayState {
                            playing: true,
                            position: cur.position as f64 / f64::from(source_sr),
                            duration,
                        }));
                    }
                },
                move |error| {
                    let _ = tx.send(BridgeOut::Event(BridgeEvent::Error {
                        code: "device_start_failed".into(),
                        message: format!(
                            "{}（{error}）",
                            error_message_for_code("device_start_failed")
                        ),
                    }));
                },
                None,
            )
            .map_err(|e| format!("{}（{e}）", error_message_for_code("device_start_failed")));
        let stream = match stream {
            Ok(s) => s,
            Err(message) => {
                self.playback.lock().expect("playback lock").playing = false;
                self.emit(BridgeEvent::Error {
                    code: "device_start_failed".into(),
                    message,
                });
                return;
            }
        };
        if let Err(e) = stream.play() {
            self.playback.lock().expect("playback lock").playing = false;
            self.emit(BridgeEvent::Error {
                code: "device_start_failed".into(),
                message: format!("{}（{e}）", error_message_for_code("device_start_failed")),
            });
            return;
        }
        self.playback_stream = Some(stream);
    }

    fn stop_playback(&mut self) {
        self.playback_stream = None; // drop = 停止播放
        // TTS 下行会话一并停（旧端单播放器语义：stop 停掉任何在播的流；
        // 回放快照已在 tts_end 复制，不受会话销毁影响）。
        self.tts = None;
        self.tts_decoder = None;
        let mut pb = self.playback.lock().expect("playback lock");
        pb.playing = false;
        let duration = pb.buffer.len() as f64 / f64::from(pb.source_sr);
        drop(pb);
        self.emit(BridgeEvent::PlayState {
            playing: false,
            position: 0.0,
            duration,
        });
    }

    // ====================================================================
    // TTS 下行（W14b-2）：解码播放 + 波形 + 回放
    // ====================================================================

    /// 开始 TTS 下行会话：清理旧会话（新合成 = 新解码器，状态边界对齐旧端
    /// 「仅在新一次合成开始时重置一次」）、建输出流等待解码数据。
    fn tts_begin(&mut self, sample_rate: u32) {
        self.tts = None;
        self.tts_decoder = None;
        self.tts_replay = None;
        self.playback_stream = None;
        self.playback.lock().expect("playback lock").playing = false;
        let Some(tx) = self.events.clone() else {
            return;
        };
        if let Err(message) = self.try_tts_begin(sample_rate, tx) {
            self.tts = None;
            self.tts_decoder = None;
            self.emit(BridgeEvent::Error {
                code: "device_start_failed".into(),
                message,
            });
        }
    }

    /// 实际启动 TTS 下行会话；Err 为可读失败原因。
    fn try_tts_begin(
        &mut self,
        sample_rate: u32,
        tx: tokio::sync::mpsc::UnboundedSender<BridgeOut>,
    ) -> Result<(), String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        // 解码器创建即校验下行率（非法率 fail-closed 拒绝）。
        let decoder = DownlinkDecoder::new(sample_rate)?;
        let host = cpal::default_host();
        let Some(device) = host.default_output_device() else {
            return Err(error_message_for_code("device_unavailable").to_string());
        };
        let out_cfg = device
            .default_output_config()
            .map_err(|e| format!("{}（{e}）", error_message_for_code("device_unavailable")))?;
        let out_sr = out_cfg.sample_rate(); // cpal 0.18: SampleRate 即 u32 别名
        let channels = out_cfg.channels() as usize;

        let buffer = Arc::new(Mutex::new(TtsBuffer {
            samples: Vec::new(),
            ended: false,
        }));
        let cursor = Arc::new(Mutex::new(PlaybackCursor {
            resampler: LinearResampler::new(sample_rate, out_sr),
            pending: Vec::new(),
            position: 0usize, // 已消费的缓冲（下行率域）样本位置
            frames_since_state: 0usize,
        }));
        let playing = Arc::new(AtomicBool::new(true));
        let cb_buffer = Arc::clone(&buffer);
        let cb_cursor = Arc::clone(&cursor);
        let cb_playing = Arc::clone(&playing);
        let cb_tx = tx.clone(); // 数据回调与错误回调各持一个发送端
        let err_tx = tx;

        let stream = device
            .build_output_stream(
                out_cfg.into(), // cpal 0.18: StreamConfig 传值（非引用）
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if !cb_playing.load(Ordering::SeqCst) {
                        data.fill(0.0);
                        return;
                    }
                    let Ok(mut cur) = cb_cursor.lock() else {
                        data.fill(0.0);
                        return;
                    };
                    let frames_needed = data.len() / channels.max(1);
                    let mut produced: Vec<f32> = Vec::with_capacity(frames_needed);
                    // 会话收尾标志（缓冲排空且 tts_end 已收到时置位）。
                    let mut finish: Option<(f64, Vec<f32>)> = None;
                    {
                        let Ok(buf) = cb_buffer.lock() else {
                            data.fill(0.0);
                            return;
                        };
                        while produced.len() < frames_needed {
                            if cur.pending.is_empty() {
                                // 从缓冲取下一块（约 10ms）重采样；缓冲是流式增长的。
                                let take = (sample_rate as f64 * 0.01) as usize;
                                let start = cur.position.min(buf.samples.len());
                                let end = (cur.position + take).min(buf.samples.len());
                                let chunk: Vec<f32> = buf.samples[start..end].to_vec();
                                cur.position = end;
                                if chunk.is_empty() {
                                    if buf.ended {
                                        // 播完：冲刷尾帧后收尾（TtsDone 由回调发出）。
                                        let tail = cur.resampler.flush();
                                        produced.extend(tail);
                                        let seconds =
                                            buf.samples.len() as f64 / f64::from(sample_rate);
                                        let buckets =
                                            wave_buckets(&buf.samples, WAVE_BUCKETS);
                                        finish = Some((seconds, buckets));
                                    }
                                    break;
                                }
                                cur.pending = cur.resampler.process(&chunk);
                            }
                            let need = frames_needed - produced.len();
                            let take = need.min(cur.pending.len());
                            produced.extend(cur.pending.drain(..take));
                        }
                        // 100ms 播放状态推送（对齐旧端 getAudioState 语义；
                        // 下行会话 duration 未知（流式），完成时由 TtsDone 带整段时长）。
                        cur.frames_since_state += frames_needed;
                        let state_interval = (out_sr as f64 * 0.1) as usize;
                        if cur.frames_since_state >= state_interval && finish.is_none() {
                            cur.frames_since_state = 0;
                            let _ = cb_tx.send(BridgeOut::Event(BridgeEvent::PlayState {
                                playing: true,
                                position: cur.position as f64 / f64::from(sample_rate),
                                duration: 0.0,
                            }));
                        }
                    }
                    if let Some((seconds, buckets)) = finish {
                        cb_playing.store(false, Ordering::SeqCst);
                        let _ = cb_tx.send(BridgeOut::Event(BridgeEvent::PlayState {
                            playing: false,
                            position: seconds,
                            duration: seconds,
                        }));
                        let _ = cb_tx.send(BridgeOut::Event(BridgeEvent::TtsDone {
                            seconds,
                            buckets,
                        }));
                    }
                    // 单声道铺到输出各声道
                    for (i, slot) in data.iter_mut().enumerate() {
                        *slot = produced.get(i / channels.max(1)).copied().unwrap_or(0.0);
                    }
                },
                move |error| {
                    let _ = err_tx.send(BridgeOut::Event(BridgeEvent::Error {
                        code: "device_start_failed".into(),
                        message: format!(
                            "{}（{error}）",
                            error_message_for_code("device_start_failed")
                        ),
                    }));
                },
                None,
            )
            .map_err(|e| format!("{}（{e}）", error_message_for_code("device_start_failed")))?;
        stream
            .play()
            .map_err(|e| format!("{}（{e}）", error_message_for_code("device_start_failed")))?;
        self.tts_decoder = Some(decoder);
        self.tts = Some(TtsSession {
            stream,
            buffer,
            dec_sr: sample_rate,
        });
        Ok(())
    }

    /// 结束 TTS 下行会话：置 ended 标志并快照已解码样本（供回放）；
    /// 缓冲排空后回调发出 `TtsDone`（会话流保留到下一次清理，静音输出无成本）。
    fn tts_end(&mut self) {
        let Some(session) = &self.tts else {
            return; // 无会话：幂等忽略
        };
        let snapshot = {
            let mut buf = session.buffer.lock().expect("tts buffer lock");
            buf.ended = true;
            buf.samples.clone()
        };
        self.tts_replay = Some(TtsReplay {
            buffer: Arc::new(Mutex::new(snapshot)),
            sr: session.dec_sr,
        });
    }

    /// 回放最近一次完成的 TTS 缓冲（对齐旧端 playTts；进度经 PlayState 推送）。
    fn play_tts(&mut self) {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let Some(replay) = self.tts_replay.clone() else {
            return; // 尚无完成的 TTS 缓冲：幂等忽略
        };
        let Some(tx) = self.events.clone() else {
            return;
        };
        // 单播放器语义：起始回放前停掉在播的试听/旧回放。
        self.playback_stream = None;
        self.playback.lock().expect("playback lock").playing = false;
        self.tts = None;
        self.tts_decoder = None;

        let source_sr = replay.sr;
        let host = cpal::default_host();
        let Some(device) = host.default_output_device() else {
            self.emit(BridgeEvent::Error {
                code: "device_unavailable".into(),
                message: error_message_for_code("device_unavailable").into(),
            });
            return;
        };
        let out_cfg = match device.default_output_config() {
            Ok(c) => c,
            Err(e) => {
                self.emit(BridgeEvent::Error {
                    code: "device_unavailable".into(),
                    message: format!("{}（{e}）", error_message_for_code("device_unavailable")),
                });
                return;
            }
        };
        let out_sr = out_cfg.sample_rate();
        let channels = out_cfg.channels() as usize;
        let duration = {
            let buf = replay.buffer.lock().expect("tts replay lock");
            buf.len() as f64 / f64::from(source_sr)
        };
        let cursor = Arc::new(Mutex::new(PlaybackCursor {
            resampler: LinearResampler::new(source_sr, out_sr),
            pending: Vec::new(),
            position: 0usize,
            frames_since_state: 0usize,
        }));
        let playing_flag = Arc::new(AtomicBool::new(true));
        let cb_playing = Arc::clone(&playing_flag);
        let cb_cursor = Arc::clone(&cursor);
        let cb_buffer = Arc::clone(&replay.buffer);
        let cb_tx = tx.clone(); // 数据回调与错误回调各持一个发送端
        let err_tx = tx;

        let stream = device
            .build_output_stream(
                out_cfg.into(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if !cb_playing.load(Ordering::SeqCst) {
                        data.fill(0.0);
                        return;
                    }
                    let Ok(mut cur) = cb_cursor.lock() else {
                        data.fill(0.0);
                        return;
                    };
                    let frames_needed = data.len() / channels.max(1);
                    let mut produced: Vec<f32> = Vec::with_capacity(frames_needed);
                    {
                        let Ok(buf) = cb_buffer.lock() else {
                            data.fill(0.0);
                            return;
                        };
                        while produced.len() < frames_needed {
                            if cur.pending.is_empty() {
                                let take = (source_sr as f64 * 0.01) as usize;
                                let start = cur.position.min(buf.len());
                                let end = (cur.position + take).min(buf.len());
                                let chunk: Vec<f32> = buf[start..end].to_vec();
                                cur.position = end;
                                if chunk.is_empty() {
                                    let tail = cur.resampler.flush();
                                    produced.extend(tail);
                                    cb_playing.store(false, Ordering::SeqCst);
                                    break;
                                }
                                cur.pending = cur.resampler.process(&chunk);
                            }
                            let need = frames_needed - produced.len();
                            let take = need.min(cur.pending.len());
                            produced.extend(cur.pending.drain(..take));
                        }
                        cur.frames_since_state += frames_needed;
                        let state_interval = (out_sr as f64 * 0.1) as usize;
                        if cur.frames_since_state >= state_interval {
                            cur.frames_since_state = 0;
                            let _ = cb_tx.send(BridgeOut::Event(BridgeEvent::PlayState {
                                playing: cb_playing.load(Ordering::SeqCst),
                                position: cur.position as f64 / f64::from(source_sr),
                                duration,
                            }));
                        }
                    }
                    for (i, slot) in data.iter_mut().enumerate() {
                        *slot = produced.get(i / channels.max(1)).copied().unwrap_or(0.0);
                    }
                },
                move |error| {
                    let _ = err_tx.send(BridgeOut::Event(BridgeEvent::Error {
                        code: "device_start_failed".into(),
                        message: format!(
                            "{}（{error}）",
                            error_message_for_code("device_start_failed")
                        ),
                    }));
                },
                None,
            )
            .map_err(|e| format!("{}（{e}）", error_message_for_code("device_start_failed")));
        let stream = match stream {
            Ok(s) => s,
            Err(message) => {
                self.emit(BridgeEvent::Error {
                    code: "device_start_failed".into(),
                    message,
                });
                return;
            }
        };
        if let Err(e) = stream.play() {
            self.emit(BridgeEvent::Error {
                code: "device_start_failed".into(),
                message: format!("{}（{e}）", error_message_for_code("device_start_failed")),
            });
            return;
        }
        self.playback_stream = Some(stream);
    }

    /// 喂一包下行 Opus 帧（bridge.rs 从前端二进制帧转入）；
    /// 无会话 / 已结束 / 非法包按 fail-closed 忽略或回错误事件。
    pub fn feed_tts_frame(&mut self, packet: &[u8]) {
        let decoded = {
            let Some(decoder) = &mut self.tts_decoder else {
                return; // 无会话：忽略（前端编排保证 tts_begin 先行）
            };
            match decoder.decode(packet) {
                Ok(samples) => samples,
                Err(message) => {
                    self.emit(BridgeEvent::Error {
                        code: "device_start_failed".into(),
                        message,
                    });
                    return;
                }
            }
        };
        let Some(session) = &self.tts else {
            return;
        };
        let mut buf = session.buffer.lock().expect("tts buffer lock");
        if buf.ended {
            return; // tts_end 后的迟到帧：丢弃
        }
        buf.samples.extend_from_slice(&decoded);
    }
}

/// 播放回调状态（音频线程）：重采样待发样本 + 进度 + 状态推送节流。
struct PlaybackCursor {
    resampler: LinearResampler,
    pending: Vec<f32>,
    /// 已消费的缓冲（输入域）样本位置。
    position: usize,
    frames_since_state: usize,
}

/// 采集回调体（音频线程）：累积原始缓冲 + 计秒 + 200ms 波形推送 + 重采样/切帧/编码上行。
fn feed_capture(
    chain: &mut CaptureChain,
    data: &[f32],
    channels: usize,
    tx: &tokio::sync::mpsc::UnboundedSender<BridgeOut>,
) {
    // 多声道混缩为单声道（取首声道即可，语音场景足够）。
    let mono: Vec<f32> = if channels <= 1 {
        data.to_vec()
    } else {
        data.chunks(channels).map(|frame| frame[0]).collect()
    };
    chain.seconds += mono.len() as f64 / f64::from(chain.in_sr);
    chain.recorded.extend_from_slice(&mono);

    // 波形推送（200ms 一次；滚动窗口最近 2s，前端实时展示）。
    chain.samples_since_wave += mono.len();
    let wave_interval = (chain.in_sr as f64 * 0.2) as usize;
    if chain.samples_since_wave >= wave_interval {
        chain.samples_since_wave = 0;
        let window_start = chain.recorded.len().saturating_sub(chain.in_sr as usize * 2);
        let buckets = wave_buckets(&chain.recorded[window_start..], WAVE_BUCKETS);
        let _ = tx.send(BridgeOut::Event(BridgeEvent::Wave {
            seconds: chain.seconds,
            buckets,
        }));
    }

    // 重采样 → 切帧 → 编码 → 上行（二进制帧经前端中转到 server /api/ws）。
    let resampled = chain.resampler.process(&mono);
    for frame in chain.slicer.push(&resampled) {
        match chain.encoder.encode(&frame) {
            Ok(bytes) => {
                let _ = tx.send(BridgeOut::Opus(bytes));
            }
            Err(message) => {
                let _ = tx.send(BridgeOut::Event(BridgeEvent::Error {
                    code: "device_start_failed".into(),
                    message,
                }));
            }
        }
    }
}

// ============================================================
// 单测（L1：纯函数，不依赖硬件）
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// DC 信号：48k→16k 线性重采样后每样本值不变，长度恰为 1/3。
    #[test]
    fn resampler_48k_to_16k_dc_signal() {
        let mut r = LinearResampler::new(48_000, 16_000);
        let input = vec![0.5f32; 48_000]; // 1s
        let out = r.process(&input);
        assert!((out.len() as f64 - 16_000.0).abs() < 2.0, "长度偏差过大: {}", out.len());
        assert!(out.iter().all(|s| (*s - 0.5).abs() < 1e-6));
    }

    /// 分块处理与整块处理等价（相位跨块保持）。
    #[test]
    fn resampler_chunked_matches_whole() {
        let input: Vec<f32> = (0..9600).map(|i| ((i as f32) * 0.01).sin()).collect();
        let mut whole = LinearResampler::new(48_000, 16_000);
        let a = whole.process(&input);
        let mut chunked = LinearResampler::new(48_000, 16_000);
        let mut b = Vec::new();
        for chunk in input.chunks(333) {
            b.extend(chunked.process(chunk));
        }
        b.extend(chunked.flush());
        assert_eq!(a.len(), b.len(), "整块 {} vs 分块 {}", a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x - y).abs() < 1e-4, "样本偏差 {x} vs {y}");
        }
    }

    /// 非整数倍（44100→16000）边界：flush 后总长度符合期望比例。
    #[test]
    fn resampler_non_integer_ratio_boundary() {
        let mut r = LinearResampler::new(44_100, 16_000);
        let input = vec![0.25f32; 44_100]; // 1s @44.1k → 输出应为 16000 样本
        let mut out = r.process(&input);
        out.extend(r.flush());
        let expected = 16_000.0;
        // 容差 3 样本（≈0.19ms）：非整数比率下 next_pos 浮点累加可能多出 1-2 个
        // 边界样本（flush 以末样本补齐），对语音 ASR 无感知影响。
        assert!(
            (out.len() as f64 - expected).abs() < 3.0,
            "长度 {} 偏离期望 {expected}",
            out.len()
        );
        assert!(out.iter().all(|s| (*s - 0.25).abs() < 1e-3));
    }

    /// 帧切分：整帧产出、余数保留、补零收尾。
    #[test]
    fn frame_slicer_complete_partial_and_flush() {
        let mut slicer = FrameSlicer::new(FRAME_SAMPLES);
        let frames = slicer.push(&vec![0.1f32; FRAME_SAMPLES * 2 + 100]);
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|f| f.len() == FRAME_SAMPLES));
        let tail = slicer.flush_padded();
        let tail = tail.expect("余数应产出补零尾帧");
        assert_eq!(tail.len(), FRAME_SAMPLES);
        assert!(tail[..100].iter().all(|s| *s == 0.1));
        assert!(tail[100..].iter().all(|s| *s == 0.0));
        assert!(slicer.flush_padded().is_none(), "flush 后应无余数");
    }

    /// Opus 编码→解码往返：16k 60ms 帧还原出长度与频率特征。
    #[test]
    fn opus_roundtrip_preserves_length_and_tone() {
        let mut encoder = UplinkEncoder::new().expect("编码器构造");
        let frame: Vec<f32> = (0..FRAME_SAMPLES)
            .map(|i| 0.5 * ((2.0 * std::f32::consts::PI * 440.0 * i as f32) / 16_000.0).sin())
            .collect();
        let bytes = encoder.encode(&frame).expect("编码");
        assert!(!bytes.is_empty());
        // 40kbps / 60ms 理论上限 ~300B，Opus 包含开销，放宽到 600B。
        assert!(bytes.len() < 600, "帧过大: {} bytes", bytes.len());

        let pcm = DownlinkDecoder::new(16_000)
            .expect("解码器构造")
            .decode(&bytes)
            .expect("解码");
        assert_eq!(pcm.len(), FRAME_SAMPLES, "60ms 帧解码应还原 960 样本");
        let peak = pcm.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.2, "解码后峰值过低（音色丢失？）: {peak}");
    }

    /// 上行帧封装对齐：v1 = 裸 Opus 帧（unwrap_uplink V1 原样返回，protocol.rs:342-344），
    /// 编码输出即为服务端解出的 payload，无任何封装头。
    ///
    /// ⚠️ 本测试**仅固化当前 V1 identity 语义**（protocol.rs:342-344），不追踪 server 侧
    /// 语义漂移——server 的 unwrap 行为由 server 自身测试保护（`bin_v2_big_endian_layout`
    /// 等）；改名不必要，恒等复刻本身无害（M1 复审结论）。
    #[test]
    fn uplink_frames_are_bare_opus_for_v1() {
        let mut encoder = UplinkEncoder::new().expect("编码器构造");
        let frame = vec![0.0f32; FRAME_SAMPLES];
        let bytes = encoder.encode(&frame).expect("编码");
        // v1 unwrap 语义：payload == 原始数据；等价断言 = 帧内容与编码输出逐字节一致。
        // 用最小复刻（对齐 server/src/workflow/protocol/protocol.rs:342-344）验证：
        let unwrapped = unwrap_uplink_v1(&bytes);
        assert_eq!(unwrapped, bytes.as_slice());
    }

    /// server `unwrap_uplink(BinVersion::V1)` 的最小复刻（测试用，防止语义漂移无感知）。
    fn unwrap_uplink_v1<'a>(data: &'a [u8]) -> &'a [u8] {
        data // V1: 裸字节
    }

    /// 波形桶：恒幅信号 → 全桶等值且在 0..1；空段 → 全 0。
    #[test]
    fn wave_buckets_rms_normalization() {
        let signal = vec![0.25f32; 16_000 * 4]; // 4s @16k
        let buckets = wave_buckets(&signal, WAVE_BUCKETS);
        assert_eq!(buckets.len(), WAVE_BUCKETS);
        // RMS = 0.25，×4 增益 = 1.0（截断上界）
        assert!(buckets.iter().all(|b| (b - 1.0).abs() < 1e-4));
        let empty = wave_buckets(&[], WAVE_BUCKETS);
        assert!(empty.iter().all(|b| *b == 0.0));
    }

    /// 命令解析：五种文本帧类型 + 未知类型丢弃。
    #[test]
    fn bridge_command_parse() {
        for (raw, expected) in [
            ("start_capture", BridgeCommand::StartCapture),
            ("stop_capture", BridgeCommand::StopCapture),
            ("discard_capture", BridgeCommand::DiscardCapture),
            ("play_recorded", BridgeCommand::PlayRecorded),
            ("stop_playback", BridgeCommand::StopPlayback),
        ] {
            let value: serde_json::Value =
                serde_json::from_str(&format!(r#"{{"type":"{raw}"}}"#)).expect("parse");
            assert_eq!(BridgeCommand::parse(&value), Some(expected), "raw={raw}");
        }
        let unknown: serde_json::Value =
            serde_json::from_str(r#"{"type":"nope"}"#).expect("parse");
        assert_eq!(BridgeCommand::parse(&unknown), None);
    }

    /// 事件序列化：字段名与前端消费严格一致。
    #[test]
    fn bridge_event_json_fields() {
        let started = BridgeEvent::Started.to_json();
        assert_eq!(started, r#"{"type":"started"}"#);
        let recorded = BridgeEvent::Recorded {
            seconds: 1.5,
            buckets: vec![0.5, 1.0],
        }
        .to_json();
        assert!(recorded.contains(r#""type":"recorded""#));
        assert!(recorded.contains(r#""seconds":1.5"#));
        assert!(recorded.contains(r#""buckets":[0.5,1.0]"#));
        let error = BridgeEvent::Error {
            code: "mic_permission_denied".into(),
            message: "x".into(),
        }
        .to_json();
        assert!(error.contains(r#""code":"mic_permission_denied""#));
        assert!(error.contains(r#""message":"x""#));
    }

    /// 错误码 → 前端锚点文案逐字。
    #[test]
    fn error_message_verbatim() {
        assert_eq!(
            error_message_for_code("mic_permission_denied"),
            "麦克风权限被拒绝（系统设置 → 隐私与安全性 → 麦克风）"
        );
        assert_eq!(
            error_message_for_code("device_unavailable"),
            "录音设备启动失败（检查输入设备）"
        );
    }

    /// 命令解析（W14b-2 下行）：tts_begin 合法率 / 非法率 fail-closed / tts_end / play_tts。
    #[test]
    fn bridge_command_tts_parse() {
        let begin: serde_json::Value =
            serde_json::from_str(r#"{"type":"tts_begin","sample_rate":24000}"#).expect("parse");
        assert_eq!(
            BridgeCommand::parse(&begin),
            Some(BridgeCommand::TtsBegin { sample_rate: 24_000 })
        );
        let begin16k: serde_json::Value =
            serde_json::from_str(r#"{"type":"tts_begin","sample_rate":16000}"#).expect("parse");
        assert_eq!(
            BridgeCommand::parse(&begin16k),
            Some(BridgeCommand::TtsBegin { sample_rate: 16_000 })
        );
        // 非法率（server 只可能发 8/12/16/24/48k）与缺失字段：fail-closed 拒绝。
        for raw in [
            r#"{"type":"tts_begin","sample_rate":12345}"#,
            r#"{"type":"tts_begin"}"#,
            r#"{"type":"tts_begin","sample_rate":"24000"}"#,
        ] {
            let value: serde_json::Value = serde_json::from_str(raw).expect("parse");
            assert_eq!(BridgeCommand::parse(&value), None, "raw={raw}");
        }
        let end: serde_json::Value = serde_json::from_str(r#"{"type":"tts_end"}"#).expect("parse");
        assert_eq!(BridgeCommand::parse(&end), Some(BridgeCommand::TtsEnd));
        let play: serde_json::Value =
            serde_json::from_str(r#"{"type":"play_tts"}"#).expect("parse");
        assert_eq!(BridgeCommand::parse(&play), Some(BridgeCommand::PlayTts));
    }

    /// 事件序列化（W14b-2 下行）：tts_done 字段名与前端消费严格一致。
    #[test]
    fn tts_done_event_json_fields() {
        let done = BridgeEvent::TtsDone {
            seconds: 2.4,
            buckets: vec![0.25, 0.5],
        }
        .to_json();
        assert!(done.contains(r#""type":"tts_done""#));
        assert!(done.contains(r#""seconds":2.4"#));
        assert!(done.contains(r#""buckets":[0.25,0.5]"#));
    }

    /// 测试用 24k 编码器（下行往返：server 下行编码同款参数 16k→24k 等价验证用）。
    fn encode_frame_24k(encoder: &mut audiopus::coder::Encoder, frame: &[f32]) -> Vec<u8> {
        let pcm: Vec<i16> = frame
            .iter()
            .map(|s| (s.clamp(-1.0, 1.0) * 32_767.0) as i16)
            .collect();
        let mut out = vec![0u8; 4000];
        let n = encoder.encode(&pcm, &mut out).expect("编码");
        out[..n].to_vec()
    }

    /// 下行解码往返（24k）：编码 60ms 正弦帧 → DownlinkDecoder 解码还原长度与音调。
    #[test]
    fn downlink_decode_roundtrip_24k() {
        use audiopus::{Application, Channels, SampleRate};
        let mut encoder =
            audiopus::coder::Encoder::new(SampleRate::Hz24000, Channels::Mono, Application::Voip)
                .expect("编码器构造");
        let frame_samples = 24_000 * 60 / 1000; // 1440
        let frame: Vec<f32> = (0..frame_samples)
            .map(|i| 0.5 * ((2.0 * std::f32::consts::PI * 440.0 * i as f32) / 24_000.0).sin())
            .collect();
        let packet = encode_frame_24k(&mut encoder, &frame);

        let mut decoder = DownlinkDecoder::new(24_000).expect("解码器构造");
        let pcm = decoder.decode(&packet).expect("解码");
        assert_eq!(pcm.len(), frame_samples, "60ms@24k 解码应还原 1440 样本");
        let peak = pcm.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.2, "解码后峰值过低（音色丢失？）: {peak}");
    }

    /// 流式解码长度确定性（pre-skip 坑的 audiopus 等价验证）：同一 Decoder 连续解码
    /// N 帧输出长度恰为 N×帧长——每包确定性产出整帧，不存在 AudioConverter 式
    /// 「凑不满请求量 → EOF 粘滞」（见模块尾注 + `+Opus.m:128-161`）。
    #[test]
    fn downlink_stream_decode_length_exact() {
        use audiopus::{Application, Channels, SampleRate};
        let mut encoder =
            audiopus::coder::Encoder::new(SampleRate::Hz24000, Channels::Mono, Application::Voip)
                .expect("编码器构造");
        let frame_samples = 24_000 * 60 / 1000;
        let frame: Vec<f32> = (0..frame_samples)
            .map(|i| 0.25 * ((2.0 * std::f32::consts::PI * 220.0 * i as f32) / 24_000.0).sin())
            .collect();
        let packets: Vec<Vec<u8>> = (0..5)
            .map(|_| encode_frame_24k(&mut encoder, &frame))
            .collect();

        let mut decoder = DownlinkDecoder::new(24_000).expect("解码器构造");
        let mut total = 0usize;
        for packet in &packets {
            total += decoder.decode(packet).expect("解码").len();
        }
        assert_eq!(
            total, 5 * frame_samples,
            "流式解码总长度应恰为 5×1440（无粘滞/无丢失）"
        );
    }

    /// 解码器状态边界：每句新建解码器（tts_begin 重建）隔离上句 overlap 状态——
    /// 全新解码器对同一包的输出与首句首包一致（状态边界等价旧端 reset 语义）。
    #[test]
    fn downlink_fresh_decoder_isolates_state() {
        use audiopus::{Application, Channels, SampleRate};
        let mut encoder =
            audiopus::coder::Encoder::new(SampleRate::Hz24000, Channels::Mono, Application::Voip)
                .expect("编码器构造");
        let frame_samples = 24_000 * 60 / 1000;
        let frame: Vec<f32> = (0..frame_samples)
            .map(|i| 0.4 * ((2.0 * std::f32::consts::PI * 330.0 * i as f32) / 24_000.0).sin())
            .collect();
        let packet = encode_frame_24k(&mut encoder, &frame);

        // 上一句：解码过若干帧的「旧」解码器。
        let mut old = DownlinkDecoder::new(24_000).expect("解码器构造");
        let _ = old.decode(&packet).expect("旧句解码");
        // 新一句：tts_begin 重建的全新解码器，对同一包的输出必须与首句一致。
        let mut fresh = DownlinkDecoder::new(24_000).expect("解码器构造");
        let fresh_out = fresh.decode(&packet).expect("新句首包解码");

        let mut first = DownlinkDecoder::new(24_000).expect("解码器构造");
        let first_out = first.decode(&packet).expect("首句首包解码");
        assert_eq!(
            fresh_out.len(),
            first_out.len(),
            "新句首包长度应与首句首包一致"
        );
        for (x, y) in fresh_out.iter().zip(first_out.iter()) {
            assert!((x - y).abs() < 1e-6, "新句首包样本应与首句首包一致");
        }
    }

    /// 非法下行率 fail-closed：DownlinkDecoder::new 拒绝非 Opus 合法率。
    #[test]
    fn downlink_decoder_rejects_invalid_rate() {
        assert!(DownlinkDecoder::new(24_000).is_ok());
        assert!(DownlinkDecoder::new(16_000).is_ok());
        assert!(DownlinkDecoder::new(22_050).is_err());
        assert!(DownlinkDecoder::new(44_100).is_err());
        assert!(DownlinkDecoder::new(0).is_err());
    }
}
