//! S3t：guest 内 hex 编解码（事件载荷 / kv 状态序列化共用单源）。

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// 严格 hex 解码：长度必须偶数、解码后不超过 `cap` 字节；错误信息带 `what` 前缀。
pub(crate) fn hex_decode(hex: &str, cap: usize, what: &str) -> Result<Vec<u8>, String> {
    let hex = hex.trim();
    if hex.len() % 2 != 0 {
        return Err(format!("{what} hex 长度必须为偶数"));
    }
    if hex.len() / 2 > cap {
        return Err(format!(
            "{what} 解码后超限（{} > {cap} 字节）",
            hex.len() / 2
        ));
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| format!("{what} hex 非法：{e}"))
        })
        .collect()
}
