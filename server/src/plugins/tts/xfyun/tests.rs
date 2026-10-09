//! `server/src/plugins/tts/xfyun.rs` 的单元测试。
//!
//! 自该文件的内联 `#[cfg(test)] mod tests` 外移（AGENTS.md §5.10：测试一律独立文件），
//! 外层只保留 `#[cfg(test)]` 模块声明。

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
