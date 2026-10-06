//! 科大讯飞**在线语音合成**（WebSocket `wss://tts-api.xfyun.cn/v2/tts`）客户端。
//!
//! 与本地 Kokoro（sherpa）并列的第二种 TTS 引擎，配置 `[tts].backend = "xfyun"` 启用。
//! 协议与鉴权按官方文档实现（https://www.xfyun.cn/doc/tts/online_tts/API.html）：
//!
//! - **鉴权**：URL 查询参数 `host`/`date`/`authorization`（HMAC-SHA256 签名）。
//!   `signature_origin = "host: {host}\ndate: {date}\nGET /v2/tts HTTP/1.1"`，
//!   再用 api_secret 做 HMAC-SHA256、base64 得 signature；authorization 为
//!   `api_key="…", algorithm="hmac-sha256", headers="host date request-line", signature="…"`
//!   的 base64。date 必须 RFC1123/GMT，服务端容忍 ±300s 时钟偏差。
//! - **报文**：一次发送整段文本（`data.status=2`，文本 base64），服务端流式回
//!   `{code,message,sid,data:{audio(base64 PCM),status,ced}}`；`data.status=2` 表示合成结束。
//!   官方约定：`code=0` 但 `data` 为空的帧可忽略。
//! - **音频**：请求 `aue=raw` + `auf=audio/L16;rate=16000` → 返回 **16k 单声道 i16 LE PCM**，
//!   这里转成 f32 后交给既有下行链路（重采样到 `[audio].downlink_sample_rate` 再 Opus 编码）。
//!
//! ⚠️ 与本地引擎的差异：合成为**网络 IO + 阻塞等待**（同步实现，跑在 `spawn_blocking` 里），
//! 单次调用全新连接（官方"短连接"语义），不持有跨请求状态。读写均设超时，避免讯飞侧
//! 无响应时把阻塞线程挂死。

use crate::config::XfyunTtsConfig;
use crate::tts::{TtsChunkCallback, TtsEngine};
use anyhow::{bail, Context, Result};
use base64::Engine as _;
use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::net::TcpStream;
use std::time::{Duration, Instant, SystemTime};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::Message;

const HOST: &str = "tts-api.xfyun.cn";
const PATH: &str = "/v2/tts";
/// 单次合成的总超时（连接 + 等待全部音频 + 收尾）。
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
/// 单次读取超时：两个分片之间不应超过该间隔（正常的合成在 1s 内必有下一片）。
const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// 讯飞在线合成引擎（无本地状态；每次 synthesize 建立一次短连接）。
pub struct XfyunTts {
    cfg: XfyunTtsConfig,
}

impl XfyunTts {
    pub fn new(cfg: &XfyunTtsConfig) -> Result<Self> {
        if cfg.app_id.trim().is_empty() || cfg.api_key.trim().is_empty() || cfg.api_secret.trim().is_empty() {
            bail!(
                "backend=xfyun 需要 [tts.xfyun] 的 app_id/api_key/api_secret（讯飞开放平台「在线语音合成」控制台获取）"
            );
        }
        Ok(Self { cfg: cfg.clone() })
    }
}

impl TtsEngine for XfyunTts {
    /// 流式合成：讯飞服务端**分片返回**音频（每片 base64 PCM），收到即解码回调——
    /// 首字延迟 = 网络 RTT + 首片合成时间，无需等整段。回调返回 `false` 时提前收流
    /// 并关闭连接（打断/断开时及时止损）。
    fn synthesize_stream(
        &self,
        text: &str,
        speed: f32,
        speaker: i32,
        chunk_cb: TtsChunkCallback,
    ) -> Result<()> {
        let _ = speaker; // 讯飞音色由 [tts.xfyun].voice 决定，不用 Kokoro sid
        let text = text.trim();
        if text.is_empty() {
            bail!("待合成文本为空");
        }
        let url = assemble_auth_url(&self.cfg);
        let (mut ws, _resp) = tungstenite::connect(&url)
            .context("连接讯飞 TTS 失败（检查网络/时钟/密钥；e.g. HMAC signature does not match 为密钥或时钟偏差）")?;
        apply_timeouts(&mut ws);
        ws.send(Message::text(build_request(&self.cfg, text, speed)))
            .context("发送讯飞 TTS 请求失败")?;

        let deadline = Instant::now() + TOTAL_TIMEOUT;
        let mut got_audio = false;
        let mut chunk_cb = chunk_cb;
        loop {
            if Instant::now() > deadline {
                bail!("讯飞 TTS 超时（{}s 内未完成合成）", TOTAL_TIMEOUT.as_secs());
            }
            let msg = ws.read().context("读取讯飞 TTS 响应失败（网络或读超时）")?;
            match msg {
                Message::Text(t) => {
                    let v: serde_json::Value =
                        serde_json::from_str(t.as_str()).context("解析讯飞 TTS 响应失败")?;
                    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
                    if code != 0 {
                        let message = v.get("message").and_then(|m| m.as_str()).unwrap_or("");
                        let sid = v.get("sid").and_then(|m| m.as_str()).unwrap_or("");
                        // 已下发过部分音频时把错误降级为"提前结束"，避免整句丢失
                        if got_audio {
                            tracing::warn!("讯飞 TTS 中途错误 {code}: {message}（sid={sid}），以已收音频为准");
                            break;
                        }
                        bail!(
                            "讯飞 TTS 错误 {code}: {message}（sid={sid}）{}",
                            xfyun_error_hint(code, effective_voice(&self.cfg))
                        );
                    }
                    // 官方注意事项：code=0 且 data 为空的帧直接忽略
                    let Some(data) = v.get("data").filter(|d| !d.is_null()) else {
                        continue;
                    };
                    if let Some(audio) = data.get("audio").and_then(|a| a.as_str()) {
                        if !audio.is_empty() {
                            let bytes = base64::engine::general_purpose::STANDARD
                                .decode(audio)
                                .context("解码讯飞音频失败")?;
                            // 增量回调（本片即新增）；取消 → 立即收流
                            if !chunk_cb(16_000, &pcm_i16le_to_f32(&bytes)) {
                                let _ = ws.close(None);
                                return Ok(());
                            }
                            got_audio = true;
                        }
                    }
                    let status = data.get("status").and_then(|s| s.as_i64()).unwrap_or(1);
                    if status == 2 {
                        break;
                    }
                }
                // 未开启 output_proto=binary，正常不会来二进制帧；防御性回调
                Message::Binary(b) => {
                    if !chunk_cb(16_000, &pcm_i16le_to_f32(b.as_ref())) {
                        let _ = ws.close(None);
                        return Ok(());
                    }
                    got_audio = true;
                }
                Message::Close(_) => bail!("讯飞 TTS 连接被对端关闭（未收到合成结束帧）"),
                _ => {}
            }
        }
        // 会话结束：按官方建议正常关闭（1000）
        let _ = ws.close(None);
        if !got_audio {
            bail!("讯飞 TTS 返回空音频（检查发音人权限/文本编码）");
        }
        Ok(())
    }

    fn output_sample_rate(&self) -> u32 {
        16_000 // 请求固定 auf=audio/L16;rate=16000
    }

    fn name(&self) -> &'static str {
        "xfyun"
    }
}

/// 组装带鉴权参数的连接 URL（签名算法见模块注释；参数值须 URL 编码）。
fn assemble_auth_url(cfg: &XfyunTtsConfig) -> String {
    let date = httpdate::fmt_http_date(SystemTime::now());
    let signature_origin = format!("host: {HOST}\ndate: {date}\nGET {PATH} HTTP/1.1");
    let signature = hmac_sha256_base64(cfg.api_secret.as_bytes(), signature_origin.as_bytes());
    let authorization_origin = format!(
        "api_key=\"{}\", algorithm=\"hmac-sha256\", headers=\"host date request-line\", signature=\"{}\"",
        cfg.api_key, signature
    );
    let authorization = base64::engine::general_purpose::STANDARD.encode(authorization_origin);
    format!(
        "wss://{HOST}{PATH}?authorization={}&date={}&host={}",
        percent_encode(&authorization),
        percent_encode(&date),
        percent_encode(HOST),
    )
}

/// 实际请求使用的发音人（配置为空时回退 xiaoyan，与 [`build_request`] 一致）。
fn effective_voice(cfg: &XfyunTtsConfig) -> &str {
    if cfg.voice.trim().is_empty() {
        "xiaoyan"
    } else {
        cfg.voice.trim()
    }
}

/// 讯飞错误码 → 可读的"下一步怎么做"提示（官方错误码表 + FAQ 高频原因，避免每次现场查表）。
///
/// 授权类错误（11200/10005）鉴权握手已通过（有 sid），卡的是**服务/发音人授权**，
/// 与网络和密钥格式无关——排查方向是讯飞控制台而非本服务。
fn xfyun_error_hint(code: i64, voice: &str) -> String {
    match code {
        // 官方 FAQ：WebAPI 在线合成报 11200 一般是使用了未授权的发音人（其次服务未开通/授权过期）。
        // 超拟人（x4_*）属单独产品线，未在控制台为其开通授权的经典 v2/tts 调用会命中最常见原因。
        11200 => format!(
            "。最常见原因：发音人未授权或服务未开通——到讯飞控制台「在线语音合成」领取服务并确认发音人已授权；\
             当前 vcn={voice}（x4_* 超拟人音色需单独开通授权）"
        ),
        // 10005 licc fail：appid 授权失败（appid 与密钥不匹配 / 未开通合成服务）
        10005 => "。appid 授权失败：检查 app_id 是否正确、该应用是否已开通在线语音合成服务".to_string(),
        _ => String::new(),
    }
}

/// 请求体：一次发送整段文本（`data.status` 固定 2），raw/16k 输出。
fn build_request(cfg: &XfyunTtsConfig, text: &str, speed: f32) -> String {
    // 内部语速（0.5~2.0，1.0 为常速）→ 讯飞 [0,100]（50 为常速）
    let speed_i = (speed.clamp(0.1, 3.0) * 50.0).round().clamp(0.0, 100.0) as i64;
    let voice = effective_voice(cfg);
    serde_json::json!({
        "common": { "app_id": cfg.app_id },
        "business": {
            "aue": "raw",
            "auf": "audio/L16;rate=16000",
            "vcn": voice,
            "tte": "UTF8",
            "speed": speed_i,
            "volume": 50,
            "pitch": 50,
        },
        "data": {
            "status": 2,
            "text": base64::engine::general_purpose::STANDARD.encode(text.as_bytes()),
        }
    })
    .to_string()
}

/// HMAC-SHA256 → base64（讯飞签名步骤 4~5）。
fn hmac_sha256_base64(key: &[u8], data: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC 接受任意长度密钥");
    mac.update(data);
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// 16-bit LE PCM → f32（[-1,1)）；尾部奇数字节丢弃。
fn pcm_i16le_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect()
}

/// RFC3986 percent-encode（保留 unreserved：ALPHA / DIGIT / `-._~`）。
/// 签名参数含 `+` `/` `=`（base64）与空格（date），不编码会被查询串误解析。
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3 / 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// tungstenite 的 `connect` 不暴露超时参数：握手后对底层 TCP 设置读写超时。
fn apply_timeouts(ws: &mut tungstenite::WebSocket<MaybeTlsStream<TcpStream>>) {
    let sock: Option<&mut TcpStream> = match ws.get_mut() {
        MaybeTlsStream::Plain(s) => Some(s),
        MaybeTlsStream::Rustls(s) => Some(&mut s.sock),
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

    /// 签名算法的黄金测试（防回归）：
    /// 输入取自官方文档的示例（host/date/掩码 secret），期望值按文档算法
    /// （signature_origin 拼接 → HMAC-SHA256 → base64）由独立实现（Python hmac）
    /// 交叉验证得到。⚠️ 文档正文里印的 signature 值用**未打码的真实密钥**计算，
    /// 无法从示例中的掩码密钥复现，故不作为基准。
    #[test]
    fn signature_matches_cross_checked_vector() {
        let secret = "secretxxxxxxxx2df7900c09xxxxxxxx";
        let date = "Thu, 01 Aug 2019 01:53:21 GMT";
        let origin = format!("host: tts-api.xfyun.cn\ndate: {date}\nGET /v2/tts HTTP/1.1");
        let sig = hmac_sha256_base64(secret.as_bytes(), origin.as_bytes());
        assert_eq!(sig, "kwClvIANtqXII8BL2GwDNUexYGZ7eu03FVvFPHs/4OY=");
        // base64 定长：HMAC-SHA256 摘要 32 字节 → 44 字符（含一个 = 填充）
        assert_eq!(sig.len(), 44);
    }

    #[test]
    fn percent_encode_escapes_query_specials() {
        assert_eq!(percent_encode("a b+c/d=e"), "a%20b%2Bc%2Fd%3De");
        assert_eq!(percent_encode("AZaz09-._~"), "AZaz09-._~");
    }

    #[test]
    fn pcm_conversion_roundtrip() {
        // 0 / 满幅正负 / 半幅
        let bytes = [0x00u8, 0x00, 0xFF, 0x7F, 0x00, 0x80, 0x00, 0x40];
        let v = pcm_i16le_to_f32(&bytes);
        assert_eq!(v.len(), 4);
        assert!((v[0]).abs() < 1e-6);
        assert!((v[1] - 32767.0 / 32768.0).abs() < 1e-6);
        assert!((v[2] + 1.0).abs() < 1e-6);
        assert!((v[3] - 0.5).abs() < 1e-6);
        // 奇数字节：丢弃尾巴不 panic
        assert_eq!(pcm_i16le_to_f32(&[1, 2, 3]).len(), 1);
    }

    #[test]
    fn request_encodes_text_and_maps_speed() {
        let cfg = XfyunTtsConfig {
            app_id: "app1".into(),
            api_key: "k".into(),
            api_secret: "s".into(),
            voice: "xiaoyan".into(),
        };
        let req: serde_json::Value = serde_json::from_str(&build_request(&cfg, "你好", 1.0)).unwrap();
        assert_eq!(req["common"]["app_id"], "app1");
        assert_eq!(req["business"]["aue"], "raw");
        assert_eq!(req["business"]["auf"], "audio/L16;rate=16000");
        assert_eq!(req["business"]["speed"], 50);
        assert_eq!(req["data"]["status"], 2);
        assert_eq!(req["data"]["text"], "5L2g5aW9");
        // 2.0 倍速 → 100（上限）
        let fast: serde_json::Value = serde_json::from_str(&build_request(&cfg, "x", 2.0)).unwrap();
        assert_eq!(fast["business"]["speed"], 100);
    }

    /// 11200（licc failed）的提示必须携带实际发音人与可行动的排查方向（实测高频报错）。
    #[test]
    fn error_hint_names_voice_and_action_for_auth_codes() {
        let hint = xfyun_error_hint(11200, "x4_yezi");
        assert!(hint.contains("x4_yezi"), "11200 提示须带 vcn：{hint}");
        assert!(hint.contains("控制台"), "11200 提示须指明排查方向：{hint}");
        assert!(xfyun_error_hint(10005, "xiaoyan").contains("app_id"));
        // 未映射的错误码不给提示（原始 code/message/sid 已足够查表）
        assert!(xfyun_error_hint(10110, "xiaoyan").is_empty());
    }

    /// 发音人回退：配置为空 → xiaoyan（与请求体一致，错误提示才不会报错 vcn）。
    #[test]
    fn effective_voice_falls_back_to_xiaoyan() {
        let empty = XfyunTtsConfig {
            app_id: "a".into(),
            api_key: "k".into(),
            api_secret: "s".into(),
            voice: "  ".into(),
        };
        assert_eq!(effective_voice(&empty), "xiaoyan");
        let set = XfyunTtsConfig {
            voice: " x4_yezi ".into(),
            ..empty
        };
        assert_eq!(effective_voice(&set), "x4_yezi");
    }
}
