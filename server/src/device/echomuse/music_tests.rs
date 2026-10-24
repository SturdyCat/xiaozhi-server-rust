use super::decode_wav_48k_mono;

/// 构造一个 16-bit PCM WAV（单/双声道、指定采样率）。
fn wav(samples: &[i16], channels: u16, rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    let byte_rate = rate * channels as u32 * 2;
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[test]
fn wav_48k_mono_passes_through_byte_exact() {
    let samples: Vec<i16> = vec![0, 1000, -1000, 32767, -32768];
    let bytes = wav(&samples, 1, 48_000);
    let out = decode_wav_48k_mono(&bytes).unwrap();
    let got: Vec<i16> = out
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes(c.try_into().unwrap()))
        .collect();
    assert_eq!(got, samples);
}

#[test]
fn wav_stereo_averages_to_mono() {
    // L=200, R=100 → 平均 150。
    let bytes = wav(&[200, 100], 2, 48_000);
    let out = decode_wav_48k_mono(&bytes).unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(i16::from_le_bytes(out[0..2].try_into().unwrap()), 150);
}

#[test]
fn wav_resamples_other_rates_to_48k() {
    // 24k → 48k：样本数翻倍（末尾插值截断容差 ±1）。
    let samples: Vec<i16> = (0..480).map(|i| (i * 100 % 30000) as i16).collect();
    let bytes = wav(&samples, 1, 24_000);
    let out = decode_wav_48k_mono(&bytes).unwrap();
    assert!((out.len() as i64 - (960 * 2) as i64).abs() <= 4, "got {}", out.len());
    // 首个样本与源一致（i0=0, frac=0）。
    assert_eq!(i16::from_le_bytes(out[0..2].try_into().unwrap()), samples[0]);
}

#[test]
fn wav_rejects_non_pcm_and_bad_bits() {
    let mut bytes = wav(&[0, 1], 1, 48_000);
    // 改编码为 IEEE float (3)。
    bytes[20..22].copy_from_slice(&3u16.to_le_bytes());
    assert!(decode_wav_48k_mono(&bytes).is_err());
    let mut bytes = wav(&[0, 1], 1, 48_000);
    bytes[34..36].copy_from_slice(&8u16.to_le_bytes()); // 8-bit
    assert!(decode_wav_48k_mono(&bytes).is_err());
    assert!(decode_wav_48k_mono(b"not a wav").is_err());
}

#[test]
fn wav_skips_unknown_chunks_and_odd_padding() {
    // data 前插一个 3 字节 LIST chunk（带对齐 pad）。
    let bytes = wav(&[10, -10], 1, 48_000);
    // 手工插入：在 fmt 之后、data 之前塞 "LIST" + size=3 + 3 字节 + 1 pad。
    let mut patched = Vec::new();
    patched.extend_from_slice(&bytes[..36]); // RIFF 头 + fmt（fmt 恰好结束于 36）
    // 注意：上面切片假设 fmt chunk 占 8+16=24，起于 12，即 36 处是 "data" id。
    let data_len = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
    patched.extend_from_slice(b"LIST");
    patched.extend_from_slice(&3u32.to_le_bytes());
    patched.extend_from_slice(b"abc"); // 3 字节 + 1 pad
    patched.push(0);
    patched.extend_from_slice(b"data");
    patched.extend_from_slice(&data_len.to_le_bytes());
    patched.extend_from_slice(&bytes[44..]);
    let out = decode_wav_48k_mono(&patched).unwrap();
    assert_eq!(out, vec![10i16.to_le_bytes(), (-10i16).to_le_bytes()].concat());
}
