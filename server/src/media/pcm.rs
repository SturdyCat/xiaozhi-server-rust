//! 上行 PCM 的采样格式换算（纯函数）。
//!
//! 从 `workflow/session/mod.rs` 下移（缺口②实施时 session.rs 行数逼近 600 上限，
//! AGENTS.md §5.10：行为不变的搬移）。唯一调用方是 AIUI 全链路
//! （`workflow/session/aiui.rs`）：AIUI 只收 16k i16 PCM。

/// 上行 f32 PCM → 16k i16（AIUI 仅收 16k；设备默认上行 16k，非 16k 走线性插值重采样）。
pub(crate) fn f32_to_i16_16k(pcm: &[f32], uplink_sr: u32) -> Vec<i16> {
    if uplink_sr == 16_000 {
        return pcm
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();
    }
    let ratio = uplink_sr as f32 / 16_000.0;
    let mut out = Vec::with_capacity((pcm.len() as f32 / ratio) as usize + 1);
    let mut pos = 0.0f32;
    while (pos as usize) < pcm.len() {
        let i = pos as usize;
        let frac = pos - i as f32;
        let a = pcm[i];
        let b = pcm.get(i + 1).copied().unwrap_or(a);
        out.push(((a + (b - a) * frac).clamp(-1.0, 1.0) * 32767.0) as i16);
        pos += ratio;
    }
    out
}

#[cfg(test)]
mod tests;
