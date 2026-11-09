//! S3n/S3o：Tuya LAN **TCP 流式帧切分**（粘包/半包重组）+ Established 数据面帧解密。
//!
//! 边界口径：TCP 只给原始字节（宿主纯净性），分帧是协议语义 → 在插件侧。
//! 缓冲经会话状态 kv 持久化（事件间无内存态，纯函数 + kv 模式）。
//! 两代布局（S3o 起共用一套切分器）：
//! - 0x55AA（v3.3/v3.4）：头 16B，len 字段在 `[12..16]` = payload + 尾差（全部在
//!   len 内，`total = 16 + len`）；
//! - 0x6699（v3.5）：头 18B，len 字段在 `[14..18]` = iv+ct+tag，suffix 4B 在 len
//!   之外（`total = 18 + len + 4`）。
//! 前导垃圾按 rustuya `scan_for_prefix` 同语义逐字节滑动丢弃。

/// 接收缓冲上限（防内存放大；正常帧远小于此）。
pub(crate) const MAX_RX_BUF: usize = 64 * 1024;
/// 单帧总长上限（头 + 声明长度 + suffix；与 lan_codec::MAX_PAYLOAD_LEN 同口径防线）。
const MAX_FRAME_LEN: usize = 16 + 256 * 1024 + 64;

/// 一种 wire 帧布局：前缀 4B、头长、len 字段之后不计入 len 的尾字节数。
struct FrameLayout {
    prefix: [u8; 4],
    header_len: usize,
    tail_len: usize,
}

const LAYOUT_55AA: FrameLayout = FrameLayout {
    prefix: [0x00, 0x00, 0x55, 0xAA],
    header_len: 16,
    tail_len: 0,
};
const LAYOUT_6699: FrameLayout = FrameLayout {
    prefix: [0x00, 0x00, 0x66, 0x99],
    header_len: 18,
    tail_len: 4,
};

/// 通用切分：喂入流字节，切出全部完整帧（原始帧字节，交上层按 cmd 解）。
/// `buf` 为跨事件持久缓冲（调用方从 kv 读入、处理后写回）。
/// 超限/长度谎报 → 显式报错（调用方应关闭连接，防内存放大）。
fn split_frames(
    buf: &mut Vec<u8>,
    data: &[u8],
    layout: &FrameLayout,
) -> Result<Vec<Vec<u8>>, String> {
    buf.extend_from_slice(data);
    if buf.len() > MAX_RX_BUF {
        buf.clear();
        return Err(format!("接收缓冲超限（> {MAX_RX_BUF}），已清空防放大"));
    }
    let mut frames = Vec::new();
    loop {
        // 前导垃圾：滑动到完整 32-bit 前缀（wire 上前 2 字节是 0x00，只查 2 字节
        // 尾半段会把头错位——S3n 编排对拍实证）。
        while !buf.is_empty() && (buf.len() < 4 || buf[0..4] != layout.prefix) {
            buf.remove(0);
        }
        if buf.len() < layout.header_len {
            break; // 头不完整
        }
        let off = layout.header_len - 4; // len 字段紧跟 cmd，是头的最后 4 字节
        let len = u32::from_be_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]) as usize;
        let total = layout
            .header_len
            .checked_add(len)
            .and_then(|t| t.checked_add(layout.tail_len))
            .ok_or("帧长度溢出")?;
        if total > MAX_FRAME_LEN {
            buf.clear();
            return Err(format!("帧声明长度谎报（{len}），已清空防放大"));
        }
        if buf.len() < total {
            break; // 半帧，等下一个 data 事件
        }
        // 防御式切片：get(..total) 防任何越界（trap 会中断整个事件处理）。
        let frame = buf.get(..total).ok_or("帧切片越界")?.to_vec();
        frames.push(frame);
        buf.drain(..total);
    }
    Ok(frames)
}

/// 0x55AA（v3.3/v3.4）流式切分。
pub(crate) fn feed_split_55aa(buf: &mut Vec<u8>, data: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    split_frames(buf, data, &LAYOUT_55AA)
}

/// 0x6699（v3.5）流式切分。
pub(crate) fn feed_split_v35(buf: &mut Vec<u8>, data: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    split_frames(buf, data, &LAYOUT_6699)
}

/// v3.4 Established 数据面帧解密（密钥 = 会话密钥库 blob）：
/// exact 解帧（HMAC under blob，不剥 retcode）→ ECB 解密去填充 → 剥 "3.4"
/// 版本头（非裸头命令带 15B 头）。返回 (cmd, 明文载荷)。
pub(crate) fn data_frame_decode(
    frame_bytes: &[u8],
    session_key: &[u8],
) -> Result<(u32, Vec<u8>), String> {
    use crate::lan_codec::parse_frame_55aa_exact;
    use crate::lan_crypto::aes_ecb_decrypt_strip;

    let frame = parse_frame_55aa_exact(frame_bytes, Some(session_key))?;
    let mut plain = aes_ecb_decrypt_strip(session_key, &frame.payload)?;
    // v3.4：非裸头命令的载荷带 "3.4" + 12×0 头（15B），解密后剥离。
    if plain.len() >= 15 && &plain[..3] == b"3.4" {
        plain.drain(..15);
    }
    Ok((frame.cmd, plain))
}

/// v3.5 Established 数据面帧解密（密钥 = 会话密钥库 blob）：
/// 0x6699 exact 解帧（GCM under blob，IV 随帧、不剥 retcode）→ 剥 "3.5" 版本头
/// （非裸头命令带 15B 头；GCM 已在帧级完成，载荷无二次加密）。返回 (cmd, 明文)。
pub(crate) fn data_frame_decode_35(
    frame_bytes: &[u8],
    session_key: &[u8],
) -> Result<(u32, Vec<u8>), String> {
    use crate::lan_frame_6699::parse_frame_6699_exact;

    let frame = parse_frame_6699_exact(frame_bytes, session_key)?;
    let mut plain = frame.payload;
    if plain.len() >= 15 && &plain[..3] == b"3.5" {
        plain.drain(..15);
    }
    Ok((frame.cmd, plain))
}
