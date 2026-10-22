//! 流式重采样（TTS 输出采样率 → 下行采样率）。
//!
//! [`StreamingResampler`]：**逐分片**推送、跨分片保持相位连续
//! （TTS 流式下发的分片边界若各自独立重采样，会产生样本级跳变/爆音）。
//! 线性插值实现——对 16k→24k 这类升采样语音足够。

/// 流式线性重采样器：`push` 逐分片推送输入样本，返回该分片转换出的输出样本。
///
/// 维护跨分片状态：下一个输出样本在输入样本坐标系中的位置（f64）+ 尾部 1 个样本缓存，
/// 因此 `push(a); push(b)` 与一次性输入 `a+b` 的输出一致（边界零跳变）。
/// 采样率相同则透传（零开销，最常见 24k→24k 路径）。
pub struct StreamingResampler {
    /// 输出/输入采样率比（`to/from`）。
    ratio: f64,
    /// 下一个输出样本在「输入样本坐标系」中的位置（以流首样本为 0）。
    pos: f64,
    /// 已接收的输入样本总数（= 缓冲区末尾的绝对坐标；缓冲区只保留 pos 附近的尾部）。
    received: f64,
    /// 尚未消费的输入尾部（至少保留 `pos` 所在位置及右侧 1 个样本）。
    tail: Vec<f32>,
    /// 输入坐标系中 `tail[0]` 的绝对位置。
    tail_base: f64,
    /// 采样率相同：直接透传。
    passthrough: bool,
}

impl StreamingResampler {
    pub fn new(from_sr: u32, to_sr: u32) -> Self {
        Self {
            ratio: to_sr as f64 / from_sr.max(1) as f64,
            pos: 0.0,
            received: 0.0,
            tail: Vec::new(),
            tail_base: 0.0,
            passthrough: from_sr == to_sr,
        }
    }

    pub fn push(&mut self, input: &[f32]) -> Vec<f32> {
        if self.passthrough {
            return input.to_vec();
        }
        self.tail.extend_from_slice(input);
        self.received += input.len() as f64;

        let mut out = Vec::with_capacity((input.len() as f64 * self.ratio) as usize + 4);
        loop {
            // 取 pos 左侧样本 idx 与右侧 idx+1 做线性插值
            let rel = self.pos - self.tail_base;
            if rel < 0.0 {
                break; // 防御：不应发生
            }
            let idx = rel as usize;
            if idx + 1 >= self.tail.len() {
                break; // 右邻样本未到，留待下一分片
            }
            let frac = (rel - idx as f64) as f32;
            let a = self.tail[idx];
            let b = self.tail[idx + 1];
            out.push(a + (b - a) * frac);
            self.pos += 1.0 / self.ratio;
        }
        // 压缩：丢弃 pos 左侧不再需要的样本（保留 floor(pos) 那个作为左样本）
        let keep_from = self.pos.floor().max(0.0);
        let drop_n = (keep_from - self.tail_base).max(0.0) as usize;
        if drop_n > 0 && drop_n <= self.tail.len() {
            self.tail.drain(..drop_n);
            self.tail_base += drop_n as f64;
        }
        out
    }
}

#[cfg(test)]
mod tests;
