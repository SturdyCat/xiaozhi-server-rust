//! 讯飞系共享基础（TTS 插件内部模块）：应用凭据结构 + WebSocket HMAC-SHA256 签名 + 错误提示。
//!
//! 真正的合成实现见 [`super::aiui`]（AIUI 主动合成 API）；本模块只放**讯飞系共用**的部分：
//! - [`XfyunTtsConfig`]：凭据与音色（`[tts.xfyun]` 段；AIUI 合成与音色探测共用）；
//! - [`signed_ws_url`]：服务鉴权（官方 doc-404：`host/date/request-line` → hmac-sha256(api_secret)
//!   → base64 → `authorization` 查询参数），讯飞系三个端点同款（AIUI 合成/AIUI 交互/v2 合成）；
//! - [`xfyun_error_hint`]：授权类错误码的可行动提示（11200/10005 等）。

use base64::Engine as _;
use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::SystemTime;

/// 讯飞在线合成凭据与音色（`[tts.xfyun]` 段）。
/// 在 AIUI 平台 / 讯飞开放平台创建应用后取得 APPID/APPKEY/APPSECRET。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct XfyunTtsConfig {
    /// 应用 APPID。
    #[serde(default)]
    pub app_id: String,
    /// APPKEY（签名 api_key=）。
    #[serde(default)]
    pub api_key: String,
    /// APPSECRET（HMAC-SHA256 签名密钥）。
    #[serde(default)]
    pub api_secret: String,
    /// 发音人（vcn）：普通 x2_* / 超拟人 x4_* / 极速超拟人 x5_/x6_ 全系；默认 xiaoyan。
    #[serde(default = "default_xfyun_voice")]
    pub voice: String,
}

fn default_xfyun_voice() -> String {
    "xiaoyan".into()
}

/// 讯飞 WebSocket HMAC 签名 URL（服务鉴权 doc-404）：
/// `host: {host}\ndate: {date}\nGET {path} HTTP/1.1` → hmac-sha256(api_secret) → base64
/// → `api_key="…",algorithm="hmac-sha256",headers="host date request-line",signature="…"` 再 base64。
pub(crate) fn signed_ws_url(host: &str, path: &str, api_key: &str, api_secret: &str) -> String {
    let date = httpdate::fmt_http_date(SystemTime::now());
    let signature_origin = format!("host: {host}\ndate: {date}\nGET {path} HTTP/1.1");
    let signature = hmac_sha256_base64(api_secret.as_bytes(), signature_origin.as_bytes());
    let authorization_origin = format!(
        "api_key=\"{}\", algorithm=\"hmac-sha256\", headers=\"host date request-line\", signature=\"{}\"",
        api_key, signature
    );
    let authorization = base64::engine::general_purpose::STANDARD.encode(authorization_origin);
    format!(
        "wss://{host}{path}?authorization={}&date={}&host={}",
        percent_encode(&authorization),
        percent_encode(&date),
        percent_encode(host),
    )
}

/// 讯飞错误码 → 可读的"下一步怎么做"提示（官方错误码表 + FAQ 高频原因，避免每次现场查表）。
///
/// 授权类错误（11200/10005）鉴权握手已通过（有 sid），卡的是**服务/发音人授权**，
/// 与网络和密钥格式无关——排查方向是讯飞控制台而非本服务。
pub(crate) fn xfyun_error_hint(code: i64, voice: &str) -> String {
    match code {
        // 官方 FAQ：在线合成报 11200 一般是使用了未授权的发音人（其次服务未开通/授权过期）。
        11200 => format!(
            "。最常见原因：发音人未授权或服务未开通——到讯飞控制台「在线语音合成」领取服务并确认发音人已授权；\
             当前 vcn={voice}"
        ),
        // 10005 licc fail：appid 授权失败（appid 与密钥不匹配 / 未开通合成服务）
        10005 => "。appid 授权失败：检查 app_id 是否正确、该应用是否已开通在线语音合成服务".to_string(),
        _ => String::new(),
    }
}

/// HMAC-SHA256 → base64（讯飞签名步骤）。
fn hmac_sha256_base64(key: &[u8], data: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC 接受任意长度密钥");
    mac.update(data);
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// RFC3986 percent-encode（保留 unreserved：ALPHA / DIGIT / `-._~`）。
/// 签名参数含 `+` `/` `=`（base64）与空格（date），不编码会被查询串误解析。
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3 / 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
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
}
