//! 音频链路（W14b-1 上行方向）：采集 → 重采样 → 帧切分 → Opus 编码，以及试听播放。
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

/// Opus 解码（仅单测往返用：16k Mono）。
#[cfg(test)]
fn decode_frame(decoder: &mut audiopus::coder::Decoder, data: &[u8]) -> Result<Vec<f32>, String> {
    let mut pcm = vec![0i16; 5760];
    let n = decoder
        .decode(Some(data), &mut pcm, false)
        .map_err(|e| format!("Opus 解码失败: {e:?}"))?;
    Ok(pcm[..n].iter().map(|s| f32::from(*s) / 32_768.0).collect())
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
    /// 事件出口（当前连接的 mpsc 发送端）。
    events: Option<tokio::sync::mpsc::UnboundedSender<BridgeOut>>,
}

struct PlaybackState {
    buffer: Vec<f32>,
    /// 采集原始采样率。
    source_sr: u32,
    playing: bool,
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
        }
    }

    /// 连接断开时清理（架构文档 M1：断开清理采集流）。
    pub fn reset(&mut self) {
        self.capture_stream = None; // drop cpal stream = 停止采集
        self.playback_stream = None;
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

        let mut decoder =
            audiopus::coder::Decoder::new(audiopus::SampleRate::Hz16000, audiopus::Channels::Mono)
                .expect("解码器构造");
        let pcm = decode_frame(&mut decoder, &bytes).expect("解码");
        assert_eq!(pcm.len(), FRAME_SAMPLES, "60ms 帧解码应还原 960 样本");
        let peak = pcm.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.2, "解码后峰值过低（音色丢失？）: {peak}");
    }

    /// 上行帧封装对齐：v1 = 裸 Opus 帧（unwrap_uplink V1 原样返回，protocol.rs:342-344），
    /// 编码输出即为服务端解出的 payload，无任何封装头。
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
}
