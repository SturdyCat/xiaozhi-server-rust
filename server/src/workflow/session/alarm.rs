//! A short local melody, not speech synthesis or downloaded media.
use crate::workflow::pipeline::prefetch::{PcmChunk, PreparedAudio, PreparedSentence};

const SAMPLE_RATE: u32 = 24_000;
pub(super) const LABEL: &str = "提醒铃声";

fn melody() -> Vec<f32> {
    let mut pcm = Vec::with_capacity(SAMPLE_RATE as usize * 720 / 1000);
    for frequency in [880.0, 1108.73, 1318.51, 880.0, 1108.73, 1318.51] {
        for sample in 0..SAMPLE_RATE * 120 / 1000 {
            let time = sample as f32 / SAMPLE_RATE as f32;
            let envelope = if time >= 0.1 {
                0.0
            } else {
                (time / 0.008).min((0.1 - time) / 0.008).min(1.0).max(0.0)
            };
            let phase = 2.0 * std::f32::consts::PI * frequency * time;
            pcm.push(0.18 * envelope * (phase.sin() + 0.2 * (2.0 * phase).sin()));
        }
    }
    pcm
}

pub(super) fn sentence() -> PreparedSentence {
    let (tx, chunks) = tokio::sync::mpsc::channel(1);
    let _ = tx.try_send(PcmChunk::unbudgeted(melody()));
    drop(tx); // <= 70 KiB of fixed local audio, not an unbounded provider stream.
    let (done_tx, done) = tokio::sync::oneshot::channel();
    let _ = done_tx.send(true);
    PreparedSentence {
        text: LABEL.into(),
        cache_key: 0,
        audio: PreparedAudio::Stream {
            sr: SAMPLE_RATE,
            chunks,
            done,
        },
    }
}

#[cfg(test)]
mod tests;
