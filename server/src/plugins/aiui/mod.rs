//! AIUI 全链路（极速超拟人交互链路）客户端：设备流水线的"整体对话引擎"选项。
//!
//! 启用 `[aiui].enabled` 后，设备流水线由「本地 ASR → LLM → TTS」级联切换为
//! 「服务端 VAD 切段 → AIUI 交互 API（识别 + 大模型 + 云端合成闭环）→ 下行」，
//! 本地三级引擎闲置；再改回 `[aiui].enabled=false` 即恢复级联，引擎无需重建。
//!
//! ## 协议（https://aiui-doc.xf-yun.com/project-1/doc-584/）
//! - `wss://aiui.xf-yun.com/v3/aiint/sos`，HMAC 鉴权与 v2/tts 同款（doc-405）。
//! - **oneshot 模式**（配合服务端本地 VAD 切段）：每轮对话 stmid 必须递增
//!   （"audio-1"/"audio-2"…），首帧携带全量参数（parameter），中/尾帧精简；
//!   header.status 与 payload.audio.status 每轮 0→1→2。
//! - 结果为 JSON 文本帧：`payload.$sub.text` 为 base64（tts 的数据字段是 `audio`）；
//!   event.key = Bos/Eos/Silence（云端 VAD 事件）；iat 为识别结果；nlp 为大模型回复
//!   （流式增量）；tts.audio 为合成音频（流式，按请求参数 raw/16k/16bit）。
//!
//! ## 硬约束（务必遵守，勿"优化"掉）
//! - **60 秒无请求数据服务端主动断连**；单连接最长 30 分钟 → 断连后下一轮自动重建。
//! - **对话历史按连接保留**（不受 new_session 影响）——重建连接 = 上下文清零。
//! - 收到任何非 0 错误码必须重建连接，否则后续持续报错。
//! - 上行音频：raw PCM 16000Hz / 单声道 / 16bit，base64；分帧建议 40ms/1280 字节。

use serde::{Deserialize, Serialize};
/// AIUI 全链路接入配置（协议见 https://aiui-doc.xf-yun.com/project-1/doc-584/）。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AiuiConfig {
    /// 是否启用 AIUI 全链路模式（默认关闭，走本地级联流水线）。
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub appid: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub api_secret: String,
    /// 平台上 appid 下创建的情景模式（main / main_box，main_box 为测试环境）。
    #[serde(default = "default_aiui_scene")]
    pub scene: String,
    /// 设备唯一标识前缀（与设备 MAC 关联，用于云端个性化/上下文绑定）。
    #[serde(default = "default_aiui_sn_prefix")]
    pub sn_prefix: String,
    /// 合成发音人（极速超拟人目录见 AIUI 文档 3.9）。
    #[serde(default = "default_aiui_voice")]
    pub voice: String,
    /// TTS 语速/音量/音调（讯飞原生 0~100，50 为常速）。
    #[serde(default = "default_aiui_speed")]
    pub speed: i32,
    #[serde(default = "default_aiui_volume")]
    pub volume: i32,
    #[serde(default = "default_aiui_pitch")]
    pub pitch: i32,
    /// 自定义人设 prompt（可空；对应 nlp.prompt）。
    #[serde(default)]
    pub prompt: String,
    /// 音频分帧推送间隔（毫秒）。云端流式 VAD 按实时节奏处理，瞬时灌入整段
    /// 会被判 Silence（实测）；10ms/1280B ≈ 4 倍速实测可用，0 = 不等待（不推荐）。
    #[serde(default = "default_aiui_pace_ms")]
    pub pace_ms: u64,
}

fn default_aiui_pace_ms() -> u64 {
    10
}

fn default_aiui_scene() -> String {
    // 平台新建 AIUI 应用默认自带 main_box（测试环境）情景模式；生产情景按平台实际配置。
    "main_box".into()
}
fn default_aiui_sn_prefix() -> String {
    "xiaozhi".into()
}
fn default_aiui_voice() -> String {
    "x6_dongmanshaonv_pro".into()
}
fn default_aiui_speed() -> i32 {
    50
}
fn default_aiui_volume() -> i32 {
    50
}
fn default_aiui_pitch() -> i32 {
    50
}

impl Default for AiuiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            appid: String::new(),
            api_key: String::new(),
            api_secret: String::new(),
            scene: default_aiui_scene(),
            sn_prefix: default_aiui_sn_prefix(),
            voice: default_aiui_voice(),
            speed: default_aiui_speed(),
            volume: default_aiui_volume(),
            pitch: default_aiui_pitch(),
            prompt: String::new(),
            pace_ms: default_aiui_pace_ms(),
        }
    }
}

use std::net::TcpStream;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::plugins::tts::xfyun::signed_ws_url;

mod protocol;
use protocol::{{first_frame, next_frame, parse_result_frame, pcm_i16_as_bytes}};

const HOST: &str = "aiui.xf-yun.com";
const PATH: &str = "/v3/aiint/sos";
/// 单轮对话总超时（发送整段 + 云端识别 + 大模型 + 合成回传）。
const TURN_TIMEOUT: Duration = Duration::from_secs(60);
/// 单次读取超时：结果帧间隔不应超过该值（大模型首 token 可能较慢，留足余量）。
const READ_TIMEOUT: Duration = Duration::from_secs(15);
/// 上行分帧：40ms @16k/16bit = 1280 字节（官方建议值）。
const FRAME_BYTES: usize = 1280;

/// 全链路轮次结果（与具体厂商解耦：识别文本 + 回复文本 + 合成音频）。
#[derive(Debug, Default, Clone)]
pub struct FullChainTurn {
    /// 识别文本（用户说的话）。
    pub stt: String,
    /// 大模型回复文本。
    pub reply: String,
    /// 合成音频：raw PCM 16000Hz 单声道 16bit LE 字节流。
    pub audio: Vec<u8>,
}

/// 全链路引擎接口：一段（VAD 切好的）语音 → 识别 + 对话 + 合成闭环。
/// 实现为**有状态**对象（内部维护连接与上下文），调用方须 `spawn_blocking` + 互斥。
/// 未来其他厂商的"整体对话引擎"实现此 trait 即可插入 session 流水线。
pub trait FullChainEngine: Send {
    fn turn(&mut self, pcm_i16: &[i16]) -> Result<FullChainTurn>;
}

/// 一轮对话的结果。
#[derive(Debug, Default, Clone)]
pub struct AiuiTurn {
    /// 识别文本（用户说的话；静音轮为空）。
    pub stt: String,
    /// 大模型回复文本（流式增量拼接）。
    pub reply: String,
    /// 合成音频：raw PCM 16000Hz 单声道 16bit LE 字节流。
    pub audio: Vec<u8>,
}

/// AIUI 交互会话：跨轮次持有的长连接（上下文按连接保留）。
/// **阻塞 IO**（tungstenite 同步客户端），调用方必须放在 `spawn_blocking` 中；
/// 线程安全由外层 `Mutex` 保证（见 `session.rs`）。
pub struct AiuiSession {
    cfg: AiuiConfig,
    sn: String,
    ws: Option<WebSocket<MaybeTlsStream<TcpStream>>>,
    /// 每轮递增的会话 id（重连后从 1 重新计数）。
    stmid: u32,
}

impl AiuiSession {
    pub fn new(cfg: AiuiConfig, sn: String) -> Self {
        Self {
            cfg,
            sn,
            ws: None,
            stmid: 0,
        }
    }

    fn connect(&mut self) -> Result<()> {
        if self.cfg.appid.trim().is_empty()
            || self.cfg.api_key.trim().is_empty()
            || self.cfg.api_secret.trim().is_empty()
        {
            bail!("[aiui].enabled 需要 appid/api_key/api_secret（AIUI 平台应用三要素）");
        }
        let url = signed_ws_url(HOST, PATH, &self.cfg.api_key, &self.cfg.api_secret);
        let (ws, _resp) = tungstenite::connect(&url)
            .context("连接 AIUI 交互服务失败（检查网络/时钟/密钥）")?;
        apply_timeouts(&ws);
        self.ws = Some(ws);
        self.stmid = 0; // 新连接 stmid 从 1 重新计数（不重复即可）
        Ok(())
    }

    /// 执行一轮对话：整段 PCM（16k/16bit LE，i16）→ 识别 + 大模型 + 合成。
    /// 连接按需懒建；任何协议错误都会置空连接（下一轮重建，上下文随之清零）。
    pub fn turn(&mut self, pcm_i16: &[i16]) -> Result<AiuiTurn> {
        if self.ws.is_none() {
            self.connect().context("重建 AIUI 连接失败")?;
        }
        self.stmid += 1;
        let stmid = format!("audio-{}", self.stmid);
        let deadline = Instant::now() + TURN_TIMEOUT;

        // ---- 发送：首帧全量参数 → 中间帧精简 → 尾帧 ----
        let bytes = pcm_i16_as_bytes(pcm_i16);
        let chunks: Vec<&[u8]> = bytes.chunks(FRAME_BYTES).collect();
        let total = chunks.len().max(1);
        for (i, chunk) in chunks.iter().map(Some).chain(std::iter::once(None)).enumerate() {
            // 空段防御：chunks 为空时补一个空尾帧（仅 i==total-1 的 None 项），否则云端永远等数据
            let Some(chunk) = chunk else { continue };
            let first = i == 0;
            let last = i + 1 == total;
            let status = match (first, last) {
                (true, true) => 2, // 单帧轮：首帧即尾帧
                (true, false) => 0,
                (_, true) => 2,
                (_, false) => 1,
            };
            let frame = if first {
                first_frame(&self.cfg, &self.sn, &stmid, chunk, status)
            } else {
                next_frame(&self.cfg, &self.sn, &stmid, chunk, status)
            };
            let msg = Message::Text(frame.into());
            match self.ws.as_mut().unwrap().send(msg) {
                Ok(()) => {}
                Err(e) => {
                    self.ws = None; // 发送失败：连接作废，下轮重建
                    bail!("发送 AIUI 请求失败: {e}");
                }
            }
            // 云端流式 VAD 按实时节奏处理：瞬时灌入整段会被判 Silence（实测），
            // 按配置间隔推帧（默认 10ms ≈ 4 倍速）
            if self.cfg.pace_ms > 0 {
                std::thread::sleep(Duration::from_millis(self.cfg.pace_ms));
            }
        }

        // ---- 接收：直到 nlp 完成 + tts 完成 / Silence / 错误 / 超时 ----
        let mut out = AiuiTurn::default();
        let mut nlp_done = false;
        let mut tts_done = false;
        loop {
            if Instant::now() > deadline {
                bail!("AIUI 轮次超时（{}s 内未完成）", TURN_TIMEOUT.as_secs());
            }
            let ws = self.ws.as_mut().unwrap();
            let msg = match ws.read() {
                Ok(m) => m,
                Err(e) => {
                    self.ws = None;
                    bail!("读取 AIUI 结果失败: {e}");
                }
            };
            let text = match &msg {
                Message::Text(t) => t.to_string(),
                Message::Binary(b) => String::from_utf8_lossy(b).into_owned(),
                Message::Close(_) => {
                    self.ws = None;
                    bail!("AIUI 连接被对端关闭");
                }
                Message::Ping(_) | Message::Pong(_) => continue,
                Message::Frame(_) => continue,
            };
            let frame = match parse_result_frame(&text) {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!("AIUI 结果帧解析失败（忽略）: {e}");
                    continue;
                }
            };
            if frame.code != 0 {
                // 错误码后连接状态不可信，必须重建（见模块注释）
                self.ws = None;
                bail!("AIUI 错误 {}: {}（连接已重建标记）", frame.code, frame.message);
            }
            if let Some(key) = frame.event_key {
                if key == "Silence" {
                    // 云端判定无人说话：本轮没有有效语音，直接结束
                    if out.stt.is_empty() {
                        return Ok(out);
                    }
                }
            }
            if let Some(t) = frame.iat_text {
                if !t.trim().is_empty() {
                    out.stt = t; // 识别结果逐帧覆盖（非流式时最后一帧即全量）
                }
            }
            if let Some(delta) = frame.nlp_delta {
                out.reply.push_str(&delta);
            }
            if let Some(audio) = frame.tts_audio {
                out.audio.extend_from_slice(&audio);
            }
            if let Some(s) = frame.nlp_status {
                nlp_done |= s == 2;
            }
            if let Some(s) = frame.tts_status {
                tts_done |= s == 2;
            }
            if frame.header_session_end {
                // header.status==2：本轮会话终止（云端显式收尾）
                break;
            }
            if nlp_done && tts_done && frame.nlp_seen && frame.tts_seen {
                break;
            }
        }
        Ok(out)
    }
}

impl FullChainEngine for AiuiSession {
    fn turn(&mut self, pcm_i16: &[i16]) -> Result<FullChainTurn> {
        let t = AiuiSession::turn(self, pcm_i16)?;
        Ok(FullChainTurn {
            stt: t.stt,
            reply: t.reply,
            audio: t.audio,
        })
    }
}

/// 握手后对底层 TCP 设置读写超时（tungstenite connect 不暴露超时参数）。
fn apply_timeouts(ws: &WebSocket<MaybeTlsStream<TcpStream>>) {
    let sock: Option<&TcpStream> = match ws.get_ref() {
        MaybeTlsStream::Plain(s) => Some(s),
        MaybeTlsStream::Rustls(s) => Some(&s.sock),
        _ => None,
    };
    if let Some(s) = sock {
        let _ = s.set_read_timeout(Some(READ_TIMEOUT));
        let _ = s.set_write_timeout(Some(READ_TIMEOUT));
    }
}

#[cfg(test)]
mod tests;
