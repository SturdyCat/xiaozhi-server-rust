//! Opus 编解码（下行编码 / 上行解码）。
//!
//! 上行：设备发来的 Opus 帧 → 解码为单声道 f32 PCM → VAD/ASR。
//! 下行：TTS 生成的 PCM → 按下行采样率编码为 Opus → 二进制帧下发。
//! 下行编码必须用 [`OpusFrameEncoder`]（流式、有状态），不要每帧新建编码器。

/// 有状态 Opus 帧编码器：一段 TTS 下行的所有帧必须共用同一个实例。
///
/// ⚠️ 不要退回「每帧新建 Encoder」（历史实现即如此）：Opus 是流式编码器，逐帧新建 =
/// 每 60ms 重启一次编码流，每个帧边界都残留编码器 startup 伪影——实听表现为
/// 「声音不连续/毛刺」（用户实测反馈）。本机对照实验（AudioToolbox 代理编解码）：
/// 逐帧新建时帧尾波形偏差约 7% 幅度且每帧固定残留；整段共用一个编码器则收敛。
#[cfg(feature = "sherpa")]
pub struct OpusFrameEncoder {
    enc: audiopus::coder::Encoder,
    out: Vec<u8>,
}

#[cfg(feature = "sherpa")]
impl OpusFrameEncoder {
    /// `sample_rate` 为下行采样率（Opus 仅支持 8k/12k/16k/24k/48k）。
    pub fn new(sample_rate: u32) -> anyhow::Result<Self> {
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
        Ok(Self {
            enc,
            out: vec![0u8; 4000],
        })
    }

    /// 编码一帧（须为整帧长）。同一实例连续调用即构成连续 Opus 流。
    pub fn encode_frame(&mut self, samples: &[f32]) -> anyhow::Result<Vec<u8>> {
        // audiopus 接受 i16 输入；按帧长整帧编码。
        let pcm: Vec<i16> = samples
            .iter()
            .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();
        let len = self
            .enc
            .encode(&pcm, &mut self.out)
            .map_err(|e| anyhow::anyhow!("Opus 编码失败: {e:?}"))?;
        Ok(self.out[..len].to_vec())
    }
}

#[cfg(not(feature = "sherpa"))]
pub struct OpusFrameEncoder;

#[cfg(not(feature = "sherpa"))]
impl OpusFrameEncoder {
    pub fn new(_sample_rate: u32) -> anyhow::Result<Self> {
        Ok(Self)
    }

    pub fn encode_frame(&mut self, _samples: &[f32]) -> anyhow::Result<Vec<u8>> {
        // mock 模式无 libopus：下行音频以空帧表示（本地联调仅验证协议回包）。
        Ok(Vec::new())
    }
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
