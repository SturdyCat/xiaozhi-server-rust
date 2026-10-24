use super::resample_linear;

/// 解析 16-bit PCM WAV → 48 kHz mono S16LE（生产链路已统一走
/// `decode_media_48k_mono`；保留一份极简 WAV 解析作为重采样/下混的**测试参照实现**）。
fn decode_wav_48k_mono(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("不是 RIFF/WAVE 文件".into());
    }
    let mut fmt: Option<(u16, u32, u16)> = None; // (channels, sample_rate, bits)
    let mut data: Option<&[u8]> = None;
    let mut pos = 12usize;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body_start = pos + 8;
        let body_end = body_start.saturating_add(size).min(bytes.len());
        let body = &bytes[body_start..body_end];
        match id {
            b"fmt " if body.len() >= 16 => {
                let audio_format = u16::from_le_bytes(body[0..2].try_into().unwrap());
                let channels = u16::from_le_bytes(body[2..4].try_into().unwrap());
                let sample_rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                let bits = u16::from_le_bytes(body[14..16].try_into().unwrap());
                if audio_format != 1 {
                    return Err(format!(
                        "不支持的编码 format={audio_format}（仅 16-bit PCM）"
                    ));
                }
                if bits != 16 {
                    return Err(format!("不支持的位深 {bits}（仅 16-bit）"));
                }
                if !matches!(channels, 1 | 2) {
                    return Err(format!("不支持的声道数 {channels}"));
                }
                fmt = Some((channels, sample_rate, bits));
            }
            b"data" => data = Some(body),
            _ => {}
        }
        pos = body_start + size + (size & 1); // chunk 按字对齐
    }
    let Some((channels, rate, _)) = fmt else {
        return Err("缺 fmt chunk".into());
    };
    let Some(data) = data else {
        return Err("缺 data chunk".into());
    };
    if !(8_000..=192_000).contains(&rate) {
        return Err(format!("采样率 {rate} 超出支持范围"));
    }
    // 交错 → mono（双声道平均）。
    let step = channels as usize;
    let mono: Vec<i16> = data
        .chunks_exact(step * 2)
        .map(|frame| {
            let mut sum = 0i32;
            for ch in 0..step {
                let off = ch * 2;
                sum += i16::from_le_bytes(frame[off..off + 2].try_into().unwrap()) as i32;
            }
            (sum / step as i32) as i16
        })
        .collect();
    // 线性重采样 → 48 kHz。
    let out = if rate == 48_000 {
        mono
    } else {
        resample_linear(&mono, rate)
    };
    let mut bytes_out = Vec::with_capacity(out.len() * 2);
    for s in out {
        bytes_out.extend_from_slice(&s.to_le_bytes());
    }
    Ok(bytes_out)
}

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
    assert!(
        (out.len() as i64 - (960 * 2) as i64).abs() <= 4,
        "got {}",
        out.len()
    );
    // 首个样本与源一致（i0=0, frac=0）。
    assert_eq!(
        i16::from_le_bytes(out[0..2].try_into().unwrap()),
        samples[0]
    );
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
    assert_eq!(
        out,
        vec![10i16.to_le_bytes(), (-10i16).to_le_bytes()].concat()
    );
}
