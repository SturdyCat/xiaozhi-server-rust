//! 重采样（TTS 输出采样率 → 下行采样率）。
//!
//! 真实路径使用 `rubato`；当 Kokoro 输出采样率与下行采样率相同（常见 24k→24k）
//! 时直接跳过，避免无谓计算。

use std::borrow::Cow;

#[cfg(feature = "sherpa")]
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
pub fn resample(samples: &[f32], _from_sr: u32, _to_sr: u32) -> Cow<'_, [f32]> {
    Cow::Borrowed(samples)
}
