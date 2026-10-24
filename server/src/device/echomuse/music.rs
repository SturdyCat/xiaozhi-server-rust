//! 音乐面：管理面播放入口（16-bit PCM WAV → 48 kHz mono S16LE）+ 逐帧下发任务。
//!
//! 下行帧 `0x04`/`0x05`（见 [`super::frames`]）；`duck`/`music_flush` 的下发在
//! [`super::fx`]，接线在 turn 起止（语音轮次只 duck、从不 flush 音乐）。
//! 音源本轮支持 16-bit PCM WAV（零新依赖，线性重采样任意采样率 → 48k）；
//! mp3/flac 等压缩格式等引入解码器后在此扩展，下发面不变。
//!
//! 流控：音乐帧与语音帧共用 output 预算（共享设备播放带宽与 2s 水库）——
//! 语音轮次占满预算时音乐自然让路（enqueue 返回 None → 任务退避重试），
//! 不需要显式优先级机制。

use std::sync::Arc;
use std::time::Duration;

use symphonia::core::audio::{AudioBuffer, AudioBufferRef, Signal};
use symphonia::core::conv::IntoSample;
use symphonia::core::sample::Sample;

use tokio::sync::mpsc::UnboundedSender;

use super::dlna::http_get_bytes;
use super::media::MediaState;
use super::{fx::Fx, DataOut};

/// 已捕获的音乐平面（灯效/音乐控制 + 媒体预算 + data 出站）。
/// 三个口来自同一次 hub 捕获：设备重连后旧捕获全部失效（epoch 校验兜底）。
#[derive(Clone)]
pub(crate) struct MusicPlane {
    fx: Fx,
    media: Arc<MediaState>,
    data_tx: UnboundedSender<DataOut>,
}

impl MusicPlane {
    pub(crate) fn new(fx: Fx, media: Arc<MediaState>, data_tx: UnboundedSender<DataOut>) -> Self {
        Self { fx, media, data_tx }
    }

    /// 用户停止：epoch 作废 + `music_flush` 丢弃设备缓冲（不可恢复，恢复播放
    /// 请重新 play）。
    pub(crate) fn stop(&self) {
        self.media.stop_music();
        self.fx.music_flush();
    }

    /// 从 URL 拉取音频（DLNA 直链），下载后解码为 48k mono S16LE，复用
    /// [`play_stream`] 逐帧推流到设备（独立音乐平面，与语音混音）。
    /// 返回时下载/解码已在后台 spawn，不阻塞调用方；失败仅记日志。
    pub(crate) async fn play_url(&self, url: &str) -> Result<(), String> {
        let bytes = http_get_bytes(url).await?;
        let pcm = tokio::task::spawn_blocking(move || decode_media_48k_mono(&bytes))
            .await
            .map_err(|e| format!("解码任务失败: {e}"))??;
        let epoch = self
            .media
            .start_music()
            .ok_or_else(|| "设备 data 链未就绪".to_string())?;
        let media = self.media.clone();
        let data_tx = self.data_tx.clone();
        tokio::spawn(async move {
            play_stream(media, data_tx, pcm, epoch).await;
        });
        Ok(())
    }
}

/// 逐帧下发：40ms/帧对齐 voice 分帧；预算满退避重试（语音优先，音乐让路）。
async fn play_stream(
    media: Arc<MediaState>,
    data_tx: UnboundedSender<DataOut>,
    pcm: Vec<u8>,
    epoch: crate::media::media::AudioEpoch,
) {
    for chunk in pcm.chunks(super::frames::SPEAKER_CHUNK_BYTES) {
        loop {
            if !epoch.is_current() || !media.ready() {
                return; // 被新播放顶掉 / 断链 / stop
            }
            if let Some(done) = media.enqueue_music(chunk, &data_tx) {
                // socket 写完再取下一帧（与 voice 同一 receipt 语义；超时按断链放弃）。
                match tokio::time::timeout(Duration::from_secs(5), done).await {
                    Ok(Ok(true)) => break,
                    _ => return,
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    media.enqueue_music_eos(&data_tx);
}

/// 线性插值重采样（音乐场景足够；波形保真交给后续解码器升级）。
fn resample_linear(samples: &[i16], from: u32) -> Vec<i16> {
    if samples.len() < 2 {
        return samples.to_vec();
    }
    let from = from as f64;
    let to = 48_000f64;
    let out_len = ((samples.len() as f64) * to / from) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 * from / to;
        let i0 = src.floor() as usize;
        let i1 = (i0 + 1).min(samples.len() - 1);
        let frac = src - i0 as f64;
        let a = samples[i0] as f64;
        let b = samples[i1] as f64;
        out.push(
            (a + (b - a) * frac)
                .round()
                .clamp(i16::MIN as f64, i16::MAX as f64) as i16,
        );
    }
    out
}

/// 解码任意受支持音频（mp3/flac/aac/m4a/ogg/wav…，经 `symphonia`）为 48 kHz
/// mono S16LE（`Vec<u8>`），直接喂 [`play_stream`]。不支持的格式返回错误。
pub(crate) fn decode_media_48k_mono(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;
    use symphonia::default::{get_codecs, get_probe};

    let mss = MediaSourceStream::new(
        Box::new(std::io::Cursor::new(bytes.to_vec())),
        Default::default(),
    );
    let probed = get_probe()
        .format(
            &Hint::default(),
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("无法识别音频格式: {e}"))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| "音频文件中没有音轨".to_string())?;
    // 跳过空编码（如纯视频轨）。
    if track.codec_params.codec == CODEC_TYPE_NULL {
        return Err("音轨编码无效".into());
    }
    let sample_rate = track.codec_params.sample_rate.unwrap_or(48_000);
    let n_ch = track.codec_params.channels.map(|c| c.count()).unwrap_or(1) as usize;
    let mut decoder = get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("无法初始化解码器: {e}"))?;

    let mut mono: Vec<i16> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::ResetRequired) => continue,
            Err(_) => break,
        };
        match decoder.decode(&packet) {
            Ok(buf) => collect_mono(&buf, n_ch, &mut mono),
            Err(symphonia::core::errors::Error::ResetRequired) => {
                let _ = decoder.reset();
                continue;
            }
            Err(_) => continue,
        }
    }
    if mono.is_empty() {
        return Err("解码后无音频样本".into());
    }
    let out = if sample_rate == 48_000 {
        mono
    } else {
        resample_linear(&mono, sample_rate)
    };
    let mut bytes_out = Vec::with_capacity(out.len() * 2);
    for s in out {
        bytes_out.extend_from_slice(&s.to_le_bytes());
    }
    Ok(bytes_out)
}

/// 把一帧解码后的音频（任意声道/样本类型）下混为 mono S16LE 追加到 `out`。
///
/// symphonia 的 `AudioBufferRef` 变体包 `Cow`（deref 到 `AudioBuffer`，planar 存储）。
fn collect_mono(buf: &AudioBufferRef, n_ch: usize, out: &mut Vec<i16>) {
    if n_ch == 0 {
        return;
    }
    match buf {
        AudioBufferRef::U8(b) => collect_plane(&b, n_ch, out),
        AudioBufferRef::U16(b) => collect_plane(&b, n_ch, out),
        AudioBufferRef::U24(b) => collect_plane(&b, n_ch, out),
        AudioBufferRef::U32(b) => collect_plane(&b, n_ch, out),
        AudioBufferRef::S8(b) => collect_plane(&b, n_ch, out),
        AudioBufferRef::S16(b) => collect_plane(&b, n_ch, out),
        AudioBufferRef::S24(b) => collect_plane(&b, n_ch, out),
        AudioBufferRef::S32(b) => collect_plane(&b, n_ch, out),
        AudioBufferRef::F32(b) => collect_plane(&b, n_ch, out),
        AudioBufferRef::F64(b) => collect_plane(&b, n_ch, out),
    }
}

/// 单样本类型版的 mono 下混：每声道样本先归一化为 f64（-1..1），平均后限幅，
/// 避免「先转 i16 再平均」的立体声削波。
fn collect_plane<T: Sample + IntoSample<f64>>(
    buf: &AudioBuffer<T>,
    n_ch: usize,
    out: &mut Vec<i16>,
) {
    for i in 0..buf.frames() {
        let mut sum = 0f64;
        for c in 0..n_ch {
            let v: f64 = buf.chan(c)[i].into_sample();
            sum += v;
        }
        let avg = (sum / n_ch as f64).clamp(-1.0, 1.0);
        out.push((avg * 32767.0).round() as i16);
    }
}

#[cfg(test)]
#[path = "music_tests.rs"]
mod tests;
