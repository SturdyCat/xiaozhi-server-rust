//! S3v：guest 内 base64 解码（插件配置命名空间的 `noise_psk` 读取专用）。
//!
//! 为什么手写：guest 依赖面越少越好，且只需要「严格解码 + 长度上限」这一个方向
//! （编码由用户在管理台粘贴 ESPHome 生成的 key，格式与宿主 `[esp_home].encryption_key`
//! 同为 base64 的 32 字节）。
//!
//! 口径：标准字母表（A–Z a–z 0–9 + /）、`=` 填充按 4 字符组对齐；任何非法字符、
//! 错位填充、残留非零位都是损坏的密钥材料——宁可显式失败，不做宽松修复
//! （fail-closed，绝不把坏 key 降级成明文会话）。

/// 严格 base64 解码：解码结果不得超过 `cap` 字节；错误信息带 `what` 前缀。
pub(crate) fn decode(input: &str, cap: usize, what: &str) -> Result<Vec<u8>, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(format!("{what} base64 为空"));
    }
    if trimmed.len() % 4 != 0 {
        return Err(format!("{what} base64 长度非法（须为 4 的倍数）"));
    }
    // 去填充：尾部 `=` 数量（0/1/2）决定末组的有效字节数。
    let (body, pad) = match trimmed.strip_suffix('=') {
        None => (trimmed, 0usize),
        Some(rest) => match rest.strip_suffix('=') {
            None => (rest, 1usize),
            Some(rest2) => (rest2, 2usize),
        },
    };
    if body.len() + pad != trimmed.len() {
        return Err(format!("{what} base64 填充符位置非法"));
    }
    if body.len() < 2 {
        return Err(format!("{what} base64 过短"));
    }
    let mut out = Vec::with_capacity(body.len() / 4 * 3 + 2);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for ch in body.chars() {
        let value = match ch {
            'A'..='Z' => ch as u32 - 'A' as u32,
            'a'..='z' => ch as u32 - 'a' as u32 + 26,
            '0'..='9' => ch as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            other => return Err(format!("{what} base64 含非法字符 {other:?}")),
        };
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    // 末组残留位必须为 0（标准 base64 的要求；非零 = 数据损坏）。
    if bits > 0 && (acc & ((1 << bits) - 1)) != 0 {
        return Err(format!("{what} base64 末组残留位非零（数据损坏）"));
    }
    if out.len() > cap {
        return Err(format!(
            "{what} base64 解码后超限（{} > {cap} 字节）",
            out.len()
        ));
    }
    Ok(out)
}

/// guest 内自检（不触网、不读写 kv）：黄金向量 + 非法输入显式失败。
pub(crate) fn self_check() -> Result<(), String> {
    // RFC 4648 测试向量（覆盖 1/2 字节残留组）。
    for (text, want) in [
        ("Zg==", "f"),
        ("Zm8=", "fo"),
        ("Zm9v", "foo"),
        ("Zm9vYmFy", "foobar"),
    ] {
        let got = decode(text, 32, "selftest").map_err(|e| format!("{text}: {e}"))?;
        if got != want.as_bytes() {
            return Err(format!("{text} 解码不一致：{got:02x?}"));
        }
    }
    // 非法字符 / 坏填充 / 超限都必须显式失败。
    for bad in ["A", "A===", "Zg=A", "Zg!=", "AAAA AAAA"] {
        if decode(bad, 32, "selftest").is_ok() {
            return Err(format!("非法 base64 {bad:?} 未被拒绝"));
        }
    }
    if decode("Zm9vYmFy", 4, "selftest").is_ok() {
        return Err("超限 base64 未被拒绝".to_string());
    }
    Ok(())
}
