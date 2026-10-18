//! AIUI **主动合成**（TTS 主动合成 API）——TTS 插件的讯飞在线实现。
//!
//! 协议（官方文档 https://aiui-doc.xf-yun.com/project-1/doc-788/，鉴权同 doc-404）：
//! - 端点 `wss://aiui.xf-yun.com/v3/aiint/sos`，HMAC-SHA256 鉴权（[`super::xfyun::signed_ws_url`]）。
//! - 文本主动合成：`header.scene = "IFLYTEK.tts"`、`interact_mode = "oneshot"`、
//!   `header.status = 3` 且 `payload.text.status = 3`（文本一帧发完）；
//!   `parameter.tts.vcn` 指定发音人（普通 x2_* / 超拟人 x4_* / 极速超拟人 x5_/x6_ 全系）。
//! - 响应：JSON 帧 `payload.tts.audio`（base64 PCM，流式分帧）；`header.status == 2` 收尾；
//!   `header.code != 0` 为错误（连接应重建）。
//! - 相比旧 `tts-api.xfyun.cn/v2/tts`：AIUI 链路**支持更广的音色**（x4 超拟人在
//!   v2/tts 报 11200 未授权、在 AIUI 链路免费开放，2026-10-07 实测）。
//!
//! ⚠️ 同步阻塞实现（网络 IO + 分帧读取），调用方必须 `spawn_blocking` 隔离。
//! 每次调用一条短连接（oneshot 语义），不持有跨请求状态。

use std::net::TcpStream;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use base64::Engine as _;
use serde_json::json;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use super::xfyun::{signed_ws_url, XfyunTtsConfig};
use super::{TtsChunkCallback, TtsEngine};

const HOST: &str = "aiui.xf-yun.com";
const PATH: &str = "/v3/aiint/sos";
/// 文本主动合成的固定情景模式（官方文档：普通/超拟人合成均取此值）。
const SCENE_TTS: &str = "IFLYTEK.tts";
/// 单次合成**活跃**超时（连接 + 收完整段音频）。
///
/// ⚠️ 这是"**上游不给数据**"的预算，**不含**服务端**故意**等待的时间：
/// 下载缓冲（`app::prefetch::DownloadBudget`）到预算后会在回调里把下载暂停，
/// 那是我们主动的流控，不是上游慢。若把它算进总超时，**缓冲越深越容易"合成超时"**
/// ——表现是长回复中间整句丢失（比断续更难排查）。故用 [`ActiveDeadline`] 记账。
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
/// 分帧间隔读取超时。
const READ_TIMEOUT: Duration = Duration::from_secs(15);
/// 连接/握手超时（TCP + TLS + WS 握手）：`tungstenite::connect` 是同步阻塞且
/// **不暴露 connect 超时**，云网络黑洞时整条合成链会挂死在握手前——这是"测试台
/// 无限卡住"的根因之一，故用独立线程 + `recv_timeout` 兜底（10s 量级）。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// "活跃耗时"预算：`tick` 把**非等待**时间从预算里扣掉，`after_wait` 把"刚发生的
/// 故意等待"排除在外（调用方在回调返回后立刻调它）。
///
/// 语义 = "上游 30s 内不给够数据就放弃"，而不是"墙上时钟 30s"。
#[derive(Debug)]
struct ActiveDeadline {
    /// 剩余活跃预算
    left: Duration,
    /// 上次记账时刻
    last: Instant,
}

impl ActiveDeadline {
    fn new(total: Duration) -> Self {
        Self { left: total, last: Instant::now() }
    }

    /// 推进到"现在"，把 `last` 以来的时长从预算里扣掉（在非等待点调用）。
    fn tick(&mut self) {
        let now = Instant::now();
        self.left = self.left.saturating_sub(now.saturating_duration_since(self.last));
        self.last = now;
    }

    /// 刚发生过一次**故意等待**（回调里的下载限速/背压）：不计入活跃预算。
    fn after_wait(&mut self) {
        self.last = Instant::now();
    }

    fn expired(&self) -> bool {
        self.left.is_zero()
    }
}

/// AIUI 主动合成引擎（无本地状态；每次合成一次短连接）。
pub struct AiuiTts {
    cfg: XfyunTtsConfig,
}

impl AiuiTts {
    pub fn new(cfg: &XfyunTtsConfig) -> Result<Self> {
        if cfg.app_id.trim().is_empty() || cfg.api_key.trim().is_empty() || cfg.api_secret.trim().is_empty()
        {
            bail!(
                "在线合成需要 [tts.xfyun] 的 app_id/api_key/api_secret（讯飞开放平台应用三要素）"
            );
        }
        Ok(Self { cfg: cfg.clone() })
    }
}

impl TtsEngine for AiuiTts {
    /// 文本主动合成：一次性发送整段文本（status=3），服务端流式回传音频帧，
    /// 逐帧 base64 解码后回调（首帧即出声）；回调返回 `false` 提前关连接止损。
    fn synthesize_stream(
        &self,
        text: &str,
        speed: f32,
        speaker: i32,
        chunk_cb: TtsChunkCallback,
    ) -> Result<()> {
        let _ = speaker; // 音色由 [tts.xfyun].voice 决定（AIUI 无 sid 概念）
        let text = text.trim();
        if text.is_empty() {
            bail!("待合成文本为空");
        }
        let mut chunk_cb = chunk_cb;
        let mut got_audio = false;
        synthesize_frames(&self.cfg, text, speed, &mut |pcm: &[f32]| {
            let keep = chunk_cb(16_000, pcm);
            if keep {
                got_audio = true;
            }
            keep
        })
        .with_context(|| format!("AIUI 合成失败（vcn={}）", effective_voice(&self.cfg)))?;
        if !got_audio {
            bail!(
                "AIUI 合成返回空音频（vcn={}）——音色可能未授权或文本被审核拦截",
                effective_voice(&self.cfg)
            );
        }
        Ok(())
    }

    fn output_sample_rate(&self) -> u32 {
        16_000 // 请求固定 auf/tts.sample_rate=16000
    }

    fn name(&self) -> &'static str {
        "xfyun"
    }
}

/// 实际使用的发音人（配置为空回退 xiaoyan）。
pub(crate) fn effective_voice(cfg: &XfyunTtsConfig) -> &str {
    if cfg.voice.trim().is_empty() {
        "xiaoyan"
    } else {
        cfg.voice.trim()
    }
}

/// 音色可用性探测（发音人目录用）：用给定凭据 + vcn 真实合成一段短文本。
/// 成功返回合成的 PCM 字节数；失败原样带出错误（未授权/不存在等）。
/// **只测不存**，不读写服务端配置。
pub fn probe_voice(cfg: &XfyunTtsConfig, vcn: &str) -> Result<u64> {
    let mut cfg = cfg.clone();
    cfg.voice = vcn.to_string();
    let mut bytes: u64 = 0;
    synthesize_frames(&cfg, "你好", 1.0, &mut |pcm: &[f32]| {
        bytes += (pcm.len() * 2) as u64;
        true
    })
    .with_context(|| format!("音色 {vcn} 探测失败"))?;
    if bytes == 0 {
        bail!("音色 {vcn} 未返回音频（未授权或不可用）");
    }
    Ok(bytes)
}

/// 凭据连通性测试（管理页「测试凭据」按钮）：用给定三要素真实发起一次短合成。
/// 成功 = 三要素有效且该发音人可用；返回合成耗时（毫秒）。**只测不存**。
pub fn test_credentials(cfg: &XfyunTtsConfig) -> Result<u64> {
    let engine = AiuiTts::new(cfg)?;
    let t0 = Instant::now();
    engine.synthesize_stream("你好", 1.0, 0, Box::new(|_sr, _chunk| true))?;
    Ok(t0.elapsed().as_millis() as u64)
}

/// 合成核心：连接 → 发文本帧 → 收音频帧（回调 f32 PCM）→ 收尾。
/// `on_pcm` 返回 `false` 提前终止（关连接）。
fn synthesize_frames(
    cfg: &XfyunTtsConfig,
    text: &str,
    speed: f32,
    on_pcm: &mut dyn FnMut(&[f32]) -> bool,
) -> Result<()> {
    let url = signed_ws_url(HOST, PATH, &cfg.api_key, &cfg.api_secret);
    let mut ws = connect_with_timeout(&url)?;
    apply_timeouts(&ws);

    // 请求：文本一帧发完（header.status = payload.text.status = 3）
    let speed_i = (speed.clamp(0.1, 3.0) * 50.0).round().clamp(0.0, 100.0) as i64;
    let request = json!({
        "header": {
            "sn": "tts-probe",
            "appid": cfg.app_id,
            "stmid": "text-1",
            "interact_mode": "oneshot",
            "status": 3,
            "scene": SCENE_TTS,
        },
        "parameter": {
            "tts": {
                "vcn": effective_voice(cfg),
                "speed": speed_i,
                "volume": 50,
                "pitch": 50,
                "tts": { "channels": 1, "sample_rate": 16_000, "bit_depth": 16, "encoding": "raw" }
            }
        },
        "payload": {
            "text": {
                "compress": "raw",
                "format": "plain",
                "text": base64::engine::general_purpose::STANDARD.encode(text.as_bytes()),
                "encoding": "utf8",
                "status": 3,
            }
        }
    })
    .to_string();
    ws.send(Message::text(request)).context("发送 AIUI 合成请求失败")?;

    let mut deadline = ActiveDeadline::new(TOTAL_TIMEOUT);
    loop {
        deadline.tick();
        if deadline.expired() {
            bail!(
                "AIUI 合成超时（上游 {}s 内未给够数据；服务端主动限速的等待不计入）",
                TOTAL_TIMEOUT.as_secs()
            );
        }
        let msg = ws.read().context("读取 AIUI 合成响应失败（网络或读超时）")?;
        let text = match &msg {
            Message::Text(t) => t.to_string(),
            Message::Binary(b) => String::from_utf8_lossy(b).into_owned(),
            Message::Close(_) => bail!("AIUI 合成连接被对端关闭（未收到结束帧）"),
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
        };
        let Ok(frame) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue; // 非 JSON 帧（握手响应等）：忽略
        };
        let header = frame.get("header").cloned().unwrap_or(serde_json::Value::Null);
        let code = header.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
        if code != 0 {
            let sid = header.get("sid").and_then(|s| s.as_str()).unwrap_or("");
            let message = header.get("message").and_then(|m| m.as_str()).unwrap_or("");
            let _ = ws.close(None);
            bail!(
                "AIUI 错误 {code}: {message}（sid={sid}, vcn={}）{}",
                effective_voice(cfg),
                super::xfyun::xfyun_error_hint(code, effective_voice(cfg))
            );
        }
        if let Some(tts) = frame.get("payload").and_then(|p| p.get("tts")) {
            if let Some(audio) = tts.get("audio").and_then(|a| a.as_str()) {
                if !audio.is_empty() {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(audio)
                        .context("解码 AIUI 音频失败")?;
                    if !on_pcm(&pcm_i16le_to_f32(&bytes)) {
                        let _ = ws.close(None);
                        return Ok(()); // 调用方取消：正常返回
                    }
                    // 回调里可能因**下载缓冲预算**而故意等待（下载限速）：不计入活跃预算，
                    // 否则"缓冲越深越容易合成超时"（见 `ActiveDeadline` 文档）。
                    deadline.after_wait();
                }
            }
        }
        // 结束条件：header.status == 2（接收最后一帧结果，官方 demo 同语义）
        if header.get("status").and_then(|s| s.as_i64()) == Some(2) {
            break;
        }
    }
    let _ = ws.close(None);
    Ok(())
}

/// 16-bit LE PCM → f32（[-1,1)）；尾部奇数字节丢弃。
pub(crate) fn pcm_i16le_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect()
}

/// 带超时的连接/握手（见 [`CONNECT_TIMEOUT`] 文档）：连接在独立线程完成，
/// 超时后本调用立即报错返回；未完成的线程随后随 TCP 自身超时自然消亡（有界），
/// 不阻塞调用方。成功路径与 `tungstenite::connect` 完全一致（握手响应不外传）。
fn connect_with_timeout(url: &str) -> Result<WebSocket<MaybeTlsStream<TcpStream>>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let url = url.to_string();
    std::thread::spawn(move || {
        let r = tungstenite::connect(&url)
            .map(|(ws, _resp)| ws)
            .context("连接 AIUI 合成服务失败（检查网络/时钟/密钥）");
        let _ = tx.send(r);
    });
    match rx.recv_timeout(CONNECT_TIMEOUT) {
        Ok(r) => r,
        Err(_) => bail!("连接 AIUI 合成服务超时（{CONNECT_TIMEOUT:?} 内未完成握手；检查网络/防火墙）"),
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
