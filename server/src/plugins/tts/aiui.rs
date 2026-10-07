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
/// 单次合成总超时（连接 + 收完整段音频）。
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
/// 分帧间隔读取超时。
const READ_TIMEOUT: Duration = Duration::from_secs(15);

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
    let (mut ws, _resp) = tungstenite::connect(&url)
        .context("连接 AIUI 合成服务失败（检查网络/时钟/密钥）")?;
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

    let deadline = Instant::now() + TOTAL_TIMEOUT;
    loop {
        if Instant::now() > deadline {
            bail!("AIUI 合成超时（{}s 内未完成）", TOTAL_TIMEOUT.as_secs());
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
mod tests {
    use super::*;

    /// pcm 转换：0 / 满幅 / 半幅 / 奇数字节防御。
    #[test]
    fn pcm_conversion_roundtrip() {
        let bytes = [0x00u8, 0x00, 0xFF, 0x7F, 0x00, 0x80, 0x00, 0x40];
        let v = pcm_i16le_to_f32(&bytes);
        assert_eq!(v.len(), 4);
        assert!((v[0]).abs() < 1e-6);
        assert!((v[1] - 32767.0 / 32768.0).abs() < 1e-6);
        assert!((v[2] + 1.0).abs() < 1e-6);
        assert!((v[3] - 0.5).abs() < 1e-6);
        assert_eq!(pcm_i16le_to_f32(&[1, 2, 3]).len(), 1);
    }

    /// 音色回退：配置为空 → xiaoyan（请求体与错误提示一致）。
    #[test]
    fn effective_voice_falls_back_to_xiaoyan() {
        let empty = XfyunTtsConfig { app_id: "a".into(), api_key: "k".into(), api_secret: "s".into(), voice: "  ".into() };
        assert_eq!(effective_voice(&empty), "xiaoyan");
        let set = XfyunTtsConfig { voice: " x4_lingxiaoqi_oral ".into(), ..empty };
        assert_eq!(effective_voice(&set), "x4_lingxiaoqi_oral");
    }

    /// 真实连 AIUI 的合成/探测冒烟（**排障工具，日常不跑**）：密钥经环境变量——
    /// `XFYUN_APP_ID=… XFYUN_API_KEY=… XFYUN_API_SECRET=… [XFYUN_VOICE=…] \
    ///  cargo test aiui_tts_live -- --ignored --nocapture`
    #[test]
    #[ignore = "live AIUI 调用：需 XFYUN_APP_ID/XFYUN_API_KEY/XFYUN_API_SECRET 环境变量"]
    fn aiui_tts_live() {
        let (Ok(app_id), Ok(api_key), Ok(api_secret)) = (
            std::env::var("XFYUN_APP_ID"),
            std::env::var("XFYUN_API_KEY"),
            std::env::var("XFYUN_API_SECRET"),
        ) else {
            eprintln!("跳过：未设置 XFYUN_APP_ID/XFYUN_API_KEY/XFYUN_API_SECRET");
            return;
        };
        let cfg = XfyunTtsConfig { app_id, api_key, api_secret, voice: "xiaoyan".into() };
        // 1) 整段合成
        let t0 = Instant::now();
        let tts = AiuiTts::new(&cfg).expect("构造引擎");
        let total = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = total.clone();
        let r = tts.synthesize_stream("你好，今天天气怎么样", 1.0, 0, Box::new(move |_sr, chunk| {
            counter.fetch_add(chunk.len(), std::sync::atomic::Ordering::Relaxed);
            true
        }));
        println!("== 合成: {:?}（{} 样本，{}ms）", r.map(|()| "OK".to_string()).map_err(|e| format!("{e:#}")), total.load(std::sync::atomic::Ordering::Relaxed), t0.elapsed().as_millis());
        // 2) 音色探测：普通 / 超拟人（v2/tts 不可用）
        for vcn in ["xiaoyan", "x4_lingxiaoqi_oral", "x6_dongmanshaonv_pro", "x2_xiaojuan"] {
            let t1 = Instant::now();
            println!("== 探测[{vcn}]: {:?}（{}ms）", probe_voice(&cfg, vcn).map(|b| format!("OK {b}B")).map_err(|e| format!("{e:#}")), t1.elapsed().as_millis());
        }
    }
}
