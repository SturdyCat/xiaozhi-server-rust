//! Opus 编解码（下行编码 / 上行解码）。
//!
//! 上行：设备发来的 Opus 帧 → 解码为单声道 f32 PCM → VAD/ASR。
//! 下行：TTS 生成的 PCM → 按下行采样率编码为 Opus → 二进制帧下发。

/// 将单声道 f32 PCM（范围 [-1,1]）编码为 Opus 帧。
#[cfg(feature = "sherpa")]
pub fn encode_opus_frame(samples: &[f32], sample_rate: u32) -> anyhow::Result<Vec<u8>> {
    use audiopus::{coder::Encoder, Application, Channels, SampleRate};

    let sr = match sample_rate {
        8000 => SampleRate::Hz8000,
        12000 => SampleRate::Hz12000,
        16000 => SampleRate::Hz16000,
        24000 => SampleRate::Hz24000,
        48000 => SampleRate::Hz48000,
        _ => anyhow::bail!("不支持的 Opus 采样率 {sample_rate}"),
    };

    let enc = Encoder::new(sr, Channels::Mono, Application::Voip)
        .map_err(|e| anyhow::anyhow!("创建 Opus 编码器失败: {e:?}"))?;

    // audiopus 接受 i16 输入；按帧长整帧编码。
    let pcm: Vec<i16> = samples
        .iter()
        .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
        .collect();
    let mut out = vec![0u8; 4000];
    let len = enc
        .encode(&pcm, &mut out)
        .map_err(|e| anyhow::anyhow!("Opus 编码失败: {e:?}"))?;
    out.truncate(len);
    Ok(out)
}

#[cfg(not(feature = "sherpa"))]
pub fn encode_opus_frame(_samples: &[f32], _sample_rate: u32) -> anyhow::Result<Vec<u8>> {
    // mock 模式无 libopus：下行音频以空帧表示（本地联调仅验证协议回包）。
    Ok(Vec::new())
}

/// 将 Opus 帧解码为单声道 f32 PCM。
#[cfg(feature = "sherpa")]
pub fn decode_opus_frame(data: &[u8], sample_rate: u32) -> anyhow::Result<Vec<f32>> {
    use audiopus::{coder::Decoder, Channels, SampleRate};

    let sr = match sample_rate {
        8000 => SampleRate::Hz8000,
        12000 => SampleRate::Hz12000,
        16000 => SampleRate::Hz16000,
        24000 => SampleRate::Hz24000,
        48000 => SampleRate::Hz48000,
        _ => anyhow::bail!("不支持的 Opus 采样率 {sample_rate}"),
    };

    let mut dec = Decoder::new(sr, Channels::Mono)
        .map_err(|e| anyhow::anyhow!("创建 Opus 解码器失败: {e:?}"))?;
    // 120ms 上限缓冲足够各采样率下的单帧。
    let mut pcm = vec![0i16; 5760];
    let n = dec
        .decode(Some(data), &mut pcm, false)
        .map_err(|e| anyhow::anyhow!("Opus 解码失败: {e:?}"))?;
    pcm.truncate(n);
    Ok(pcm.into_iter().map(|s| s as f32 / 32768.0).collect())
}

#[cfg(not(feature = "sherpa"))]
pub fn decode_opus_frame(_data: &[u8], _sample_rate: u32) -> anyhow::Result<Vec<f32>> {
    Ok(Vec::new())
}
