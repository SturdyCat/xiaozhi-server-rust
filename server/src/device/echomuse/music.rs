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

use tokio::sync::mpsc::UnboundedSender;

use super::media::MediaState;
use super::{DataOut, fx::Fx};

/// 已捕获的音乐平面（灯效/音乐控制 + 媒体预算 + data 出站）。
/// 三个口来自同一次 hub 捕获：设备重连后旧捕获全部失效（epoch 校验兜底）。
#[derive(Clone)]
pub(crate) struct MusicPlane {
    fx: Fx,
    media: Arc<MediaState>,
    data_tx: UnboundedSender<DataOut>,
}

impl MusicPlane {
    pub(crate) fn new(
        fx: Fx,
        media: Arc<MediaState>,
        data_tx: UnboundedSender<DataOut>,
    ) -> Self {
        Self {
            fx,
            media,
            data_tx,
        }
    }

    /// 解码并播放一个 WAV 文件；返回预计时长。旧流立即被顶掉。
    pub(crate) async fn play_file(&self, path: &std::path::Path) -> Result<Duration, String> {
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| format!("读取失败: {e}"))?;
        let pcm = tokio::task::spawn_blocking(move || decode_wav_48k_mono(&bytes))
            .await
            .map_err(|e| format!("解码任务失败: {e}"))??;
        let duration = Duration::from_secs_f64(pcm.len() as f64 / 2.0 / 48_000.0);
        let epoch = self
            .media
            .start_music()
            .ok_or_else(|| "设备 data 链未就绪".to_string())?;
        let media = self.media.clone();
        let data_tx = self.data_tx.clone();
        tokio::spawn(async move {
            play_stream(media, data_tx, pcm, epoch).await;
        });
        Ok(duration)
    }

    /// 用户停止：epoch 作废 + `music_flush` 丢弃设备缓冲（不可恢复，恢复播放
    /// 请重新 play）。
    pub(crate) fn stop(&self) {
        self.media.stop_music();
        self.fx.music_flush();
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

/// 解析 16-bit PCM WAV → 48 kHz mono S16LE。支持单/双声道、任意采样率（线性
/// 重采样）。只认 `RIFF/WAVE` + `fmt `(PCM) + `data`，其余 chunk 跳过。
pub(crate) fn decode_wav_48k_mono(bytes: &[u8]) -> Result<Vec<u8>, String> {
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
                    return Err(format!("不支持的编码 format={audio_format}（仅 16-bit PCM）"));
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
        out.push((a + (b - a) * frac).round().clamp(i16::MIN as f64, i16::MAX as f64) as i16);
    }
    out
}

#[cfg(test)]
#[path = "music_tests.rs"]
mod tests;
