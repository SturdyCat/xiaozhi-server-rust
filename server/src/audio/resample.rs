//! 重采样（TTS 输出采样率 → 下行采样率）。
//!
//! - 整段路径 [`resample`]：真实路径使用 `rubato`；采样率相同直接跳过。
//! - 流式路径 [`StreamingResampler`]：**逐分片**推送、跨分片保持相位连续
//!   （TTS 流式下发的分片边界若各自独立重采样，会产生样本级跳变/爆音）。
//!   线性插值实现——对 16k→24k 这类升采样语音足够，且无 rubato 的固定块长约束。

use std::borrow::Cow;

/// 整段重采样（工具函数）。当前下行主路径为流式（[`StreamingResampler`]）；
/// 保留给将来的非流式场景/测试。
#[cfg(feature = "sherpa")]
#[allow(dead_code)]
pub fn resample(samples: &[f32], from_sr: u32, to_sr: u32) -> Cow<'_, [f32]> {
    if from_sr == to_sr {
        // 最常见路径（Kokoro 默认 24k→24k）：直接借用，避免整段克隆。
        return Cow::Borrowed(samples);
    }
    use rubato::{
        Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType,
        WindowFunction,
    };
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };
    let ratio = to_sr as f64 / from_sr as f64;
    let mut resampler = match SincFixedIn::<f32>::new(ratio, 2.0, params, samples.len().max(1), 1) {
        Ok(r) => r,
        Err(_) => return Cow::Borrowed(samples),
    };
    match resampler.process(&[samples.to_vec()], None) {
        Ok(out) if !out.is_empty() => Cow::Owned(out[0].clone()),
        _ => Cow::Borrowed(samples),
    }
}

#[cfg(not(feature = "sherpa"))]
#[allow(dead_code)]
pub fn resample(samples: &[f32], _from_sr: u32, _to_sr: u32) -> Cow<'_, [f32]> {
    Cow::Borrowed(samples)
}

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
mod tests {
    use super::StreamingResampler;

    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| i as f32 / n as f32).collect()
    }

    /// 分片推送与整段推送结果一致（相位连续，无边界跳变）。
    #[test]
    fn streamed_chunks_match_single_push() {
        let input = ramp(1000);
        let mut whole = StreamingResampler::new(16000, 24000);
        let expect = whole.push(&input);

        let mut streamed = StreamingResampler::new(16000, 24000);
        let mut got = Vec::new();
        for chunk in input.chunks(160) {
            got.extend(streamed.push(chunk));
        }
        assert_eq!(got.len(), expect.len());
        for (a, b) in got.iter().zip(expect.iter()) {
            assert!((a - b).abs() < 1e-6, "分片结果与整段不一致: {a} vs {b}");
        }
    }

    /// 相同采样率透传。
    #[test]
    fn passthrough_when_same_rate() {
        let input = ramp(100);
        let mut r = StreamingResampler::new(24000, 24000);
        assert_eq!(r.push(&input), input);
    }

    /// 输出样本数 ≈ 输入 × ratio（允许 ±2）。
    #[test]
    fn output_length_tracks_ratio() {
        let input = ramp(1600); // 100ms @16k
        let mut r = StreamingResampler::new(16000, 24000);
        let out = r.push(&input);
        assert!((out.len() as i64 - 2400).abs() <= 2, "got {}", out.len());
    }

    /// 压缩逻辑不丢数据：长流分片推进后 tail 有界且结果连续。
    #[test]
    fn tail_bounded_and_continuous() {
        let mut r = StreamingResampler::new(16000, 24000);
        let mut total = 0usize;
        for _ in 0..200 {
            total += r.push(&ramp(160)).len();
        }
        // 200×160=32000 输入样本 → 48000 输出（±2）
        assert!((total as i64 - 48000).abs() <= 2, "got {total}");
        assert!(r.tail.len() < 8, "tail 未收敛: {}", r.tail.len());
    }
}
