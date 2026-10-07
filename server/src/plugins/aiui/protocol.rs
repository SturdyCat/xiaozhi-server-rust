//! AIUI 交互协议的编解码（收发帧构造与结果帧解析）——[`super`] 的连接/轮次逻辑专用。
//!
//! 协议契约见官方文档 https://aiui-doc.xf-yun.com/project-1/doc-584/ ：
//! - oneshot 模式：每轮 `stmid` 递增、首帧带全量 parameter（中/尾帧精简）、
//!   `header.status` 与 `payload.audio.status` 每轮 0→1→2；
//! - 结果帧：`payload.$sub.text` 为 base64（tts 子段的数据字段名为 `audio`）；
//!   `event.key` = Bos/Eos/Silence（云端 VAD 事件）；iat 为流式听写；
//!   nlp 为大模型流式增量；tts.audio 为合成音频；`header.status==2` 收尾。


use anyhow::{Context, Result};
use base64::Engine as _;
use serde_json::json;

use super::AiuiConfig;

/// 一轮结果帧的解析产物。
pub(super) struct TurnFrame {
    pub(super) code: i64,
    pub(super) message: String,
    /// payload.event.key（Bos/Eos/Silence）。
    pub(super) event_key: Option<String>,
    /// 识别文本（解码后）。
    pub(super) iat_text: Option<String>,
    /// 大模型回复增量（解码后）。
    pub(super) nlp_delta: Option<String>,
    pub(super) nlp_status: Option<i64>,
    pub(super) nlp_seen: bool,
    /// 合成音频（解码后字节）。
    pub(super) tts_audio: Option<Vec<u8>>,
    pub(super) tts_status: Option<i64>,
    pub(super) tts_seen: bool,
    /// header.status==2：本轮会话终止。
    pub(super) header_session_end: bool,
}

/// 解析一帧结果 JSON（文本字段 base64 解码）。
pub(super) fn parse_result_frame(text: &str) -> Result<TurnFrame> {
    let v: serde_json::Value =
        serde_json::from_str(text).context("结果帧不是合法 JSON")?;
    let header = v.get("header").cloned().unwrap_or(serde_json::Value::Null);
    let code = header.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    let message = header
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let header_session_end = header.get("status").and_then(|s| s.as_i64()) == Some(2);

    let mut f = TurnFrame {
        code,
        message,
        event_key: None,
        iat_text: None,
        nlp_delta: None,
        nlp_status: None,
        nlp_seen: false,
        tts_audio: None,
        tts_status: None,
        tts_seen: false,
        header_session_end,
    };
    // 排障开关：AIUI_DEBUG=1 时紧凑打印每帧子段（子段名(状态)），避免 base64 刷屏
    if std::env::var("AIUI_DEBUG").is_ok() {
        let subs: Vec<String> = v
            .get("payload")
            .and_then(|p| p.as_object())
            .map(|m| {
                m.iter()
                    .map(|(k, sub)| {
                        format!(
                            "{k}(s={})",
                            sub.get("status").and_then(|x| x.as_i64()).unwrap_or(-1)
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        eprintln!("AIUI 帧 subs={subs:?}");
    }
    if let Some(payload) = v.get("payload") {
        if let Some(ev) = payload.get("event") {
            if let Some(t) = ev.get("text").and_then(|t| t.as_str()) {
                if let Ok(decoded) = base64_decode_string(t) {
                    if let Ok(j) = serde_json::from_str::<serde_json::Value>(&decoded) {
                        f.event_key = j
                            .get("key")
                            .and_then(|k| k.as_str())
                            .map(str::to_string);
                    }
                }
            }
        }
        if let Some(iat) = payload.get("iat") {
            if let Some(t) = iat.get("text").and_then(|t| t.as_str()) {
                if let Ok(decoded) = base64_decode_string(t) {
                    f.iat_text = Some(iat_extract_text(&decoded));
                }
                f.tts_seen = true; // 占位无害：iat 帧的存在让空 nlp/tts 的纯识别轮可判定结束
            }
        }
        if let Some(nlp) = payload.get("nlp") {
            f.nlp_seen = true;
            f.nlp_status = nlp.get("status").and_then(|s| s.as_i64());
            if let Some(t) = nlp.get("text").and_then(|t| t.as_str()) {
                f.nlp_delta = base64_decode_string(t).ok();
            }
        }
        if let Some(tts) = payload.get("tts") {
            f.tts_seen = true;
            f.tts_status = tts.get("status").and_then(|s| s.as_i64());
            if let Some(a) = tts.get("audio").and_then(|a| a.as_str()) {
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(a) {
                    f.tts_audio = Some(bytes);
                }
            }
        }
    }
    Ok(f)
}

/// 识别结果 JSON → 明文：标准听写格式 `ws[].cw[].w` 拼接。
/// 兼容两种形态：`{"ws":[…]}` 与极速链路的 `{"text":{…ws…}}`（text 为嵌套对象）。
pub(super) fn iat_extract_text(decoded: &str) -> String {
    let Ok(j) = serde_json::from_str::<serde_json::Value>(decoded) else {
        return decoded.to_string();
    };
    let root = match j.get("text") {
        Some(t) if t.is_object() => t, // 嵌套：取 text 对象内部
        _ => &j,
    };
    let mut out = String::new();
    if let Some(ws) = root.get("ws").and_then(|w| w.as_array()) {
        for w in ws {
            if let Some(cw) = w.get("cw").and_then(|c| c.as_array()) {
                for c in cw {
                    if let Some(word) = c.get("w").and_then(|x| x.as_str()) {
                        out.push_str(word);
                    }
                }
            }
        }
    }
    out
}

pub(super) fn base64_decode_string(s: &str) -> Result<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s)
        .context("base64 解码失败")?;
    String::from_utf8(bytes).context("解码结果非 UTF-8")
}

/// 首帧：全量参数（header + parameter + payload.audio）。
pub(super) fn first_frame(cfg: &AiuiConfig, sn: &str, stmid: &str, audio: &[u8], status: i64) -> String {
    json!({
        "header": frame_header(cfg, sn, stmid, status),
        "parameter": {
            "iat": {
                "iat": { "encoding": "utf8", "compress": "raw", "format": "json" },
                "vgap": 800
            },
            "nlp": {
                "nlp": { "encoding": "utf8", "compress": "raw", "format": "json" },
                "new_session": "false",
                "prompt": cfg.prompt,
            },
            "tts": {
                "vcn": cfg.voice,
                "speed": cfg.speed,
                "volume": cfg.volume,
                "pitch": cfg.pitch,
                "tts": { "encoding": "raw", "sample_rate": 16000, "channels": 1, "bit_depth": 16 }
            }
        },
        "payload": {
            "audio": {
                "status": status,
                "audio": base64::engine::general_purpose::STANDARD.encode(audio),
                "encoding": "raw",
                "sample_rate": 16000,
                "channels": 1,
                "bit_depth": 16
            }
        }
    })
    .to_string()
}

/// 中/尾帧：精简（仅 header + payload.audio）。
pub(super) fn next_frame(cfg: &AiuiConfig, sn: &str, stmid: &str, audio: &[u8], status: i64) -> String {
    json!({
        "header": frame_header(cfg, sn, stmid, status),
        "payload": {
            "audio": {
                "status": status,
                "audio": base64::engine::general_purpose::STANDARD.encode(audio),
                "encoding": "raw",
                "sample_rate": 16000,
                "channels": 1,
                "bit_depth": 16
            }
        }
    })
    .to_string()
}

pub(super) fn frame_header(cfg: &AiuiConfig, sn: &str, stmid: &str, status: i64) -> serde_json::Value {
    json!({
        "appid": cfg.appid,
        "sn": sn,
        "status": status,
        "stmid": stmid,
        "scene": cfg.scene,
        "interact_mode": "oneshot"
    })
}

pub(super) fn pcm_i16_as_bytes(pcm: &[i16]) -> Vec<u8> {
    // i16 → 小端字节序列（显式 to_le_bytes，平台无关）
    let mut out = Vec::with_capacity(pcm.len() * 2);
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}
