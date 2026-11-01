//! Strict delivery audio: optional melody + speech share one paced TTS round.
//! Unlike ordinary conversational best-effort sentences, every delivery source
//! must complete encoding and synthesis or the outbox remains unsubmitted.
use super::{alarm, Session, SessionParams};
use crate::media::hot_audio::{f32_to_pcm16_bytes, MAX_ENTRY_BYTES, MAX_FRAMES};
use crate::observability::events::TurnTrace;
use crate::workflow::downlink::framer::StreamingFramer;
use crate::workflow::downlink::reservoir::{DownlinkItem, ReservoirSink};
use crate::workflow::downlink::{drain_reservoir, new_reservoir, send_tts_state};
use crate::workflow::pipeline::prefetch::{PreparedAudio, PreparedSentence};
use crate::workflow::pipeline::tts_cache::TtsCache;
use crate::workflow::protocol::Transport;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;
mod transport;
use transport::DeliveryTransport;

#[derive(Default)]
struct Capture {
    frames: Vec<Vec<u8>>,
    pcm: Vec<u8>,
    frame_bytes: usize,
}
impl Capture {
    fn pcm(capture: &mut Option<Self>, chunk: &[f32]) {
        if let Some(c) = capture {
            if c.pcm.len().saturating_add(chunk.len().saturating_mul(2)) > MAX_ENTRY_BYTES {
                *capture = None;
            } else {
                c.pcm.extend_from_slice(&f32_to_pcm16_bytes(chunk));
            }
        }
    }
    fn frame(capture: &mut Option<Self>, frame: &[u8]) {
        if let Some(c) = capture {
            if c.frames.len() >= MAX_FRAMES
                || c.frame_bytes.saturating_add(frame.len()) > MAX_ENTRY_BYTES
            {
                *capture = None;
            } else {
                c.frame_bytes += frame.len();
                c.frames.push(frame.to_vec());
            }
        }
    }
}

async fn push_frame(
    sink: &ReservoirSink,
    abort: &AtomicBool,
    frame: Vec<u8>,
) -> anyhow::Result<()> {
    anyhow::ensure!(!abort.load(Ordering::Relaxed), "delivery audio cancelled");
    anyhow::ensure!(
        sink.push(DownlinkItem::Frame(frame)).await,
        "delivery consumer closed"
    );
    Ok(())
}

async fn push_checked(
    sink: &ReservoirSink,
    cache: &TtsCache,
    params: SessionParams,
    abort: &AtomicBool,
    sentence: PreparedSentence,
) -> anyhow::Result<()> {
    anyhow::ensure!(!abort.load(Ordering::Relaxed), "delivery source cancelled");
    anyhow::ensure!(
        sink.push(DownlinkItem::SentenceStart(sentence.text)).await,
        "delivery consumer closed"
    );
    let mut count = 0;
    match sentence.audio {
        PreparedAudio::Cached(frames) => {
            for frame in frames.iter() {
                push_frame(sink, abort, frame.clone()).await?;
                count += 1;
            }
        }
        PreparedAudio::Stream {
            sr,
            mut chunks,
            done,
        } => {
            let mut framer = StreamingFramer::new_format(sr, params.output_format())?;
            let mut capture = (sentence.cache_key != 0).then(Capture::default);
            while let Some(chunk) = chunks.recv().await {
                anyhow::ensure!(!abort.load(Ordering::Relaxed), "delivery source cancelled");
                Capture::pcm(&mut capture, &chunk.pcm);
                let frames = framer.push(&chunk.pcm)?;
                drop(chunk); // Return download bytes/duration before reservoir backpressure.
                for frame in frames {
                    Capture::frame(&mut capture, &frame);
                    push_frame(sink, abort, frame).await?;
                    count += 1;
                }
            }
            anyhow::ensure!(
                matches!(done.await, Ok(true)),
                "delivery synthesis did not complete"
            );
            if let Some(tail) = framer.finish()? {
                Capture::frame(&mut capture, &tail);
                push_frame(sink, abort, tail).await?;
                count += 1;
            }
            anyhow::ensure!(!abort.load(Ordering::Relaxed), "delivery source cancelled");
            if let Some(c) = capture {
                if !c.frames.is_empty() && !c.pcm.is_empty() {
                    cache
                        .insert_async(sentence.cache_key, c.frames, Some(c.pcm))
                        .await;
                }
            }
        }
    }
    anyhow::ensure!(count > 0, "delivery source produced no frames");
    Ok(())
}

impl<T: Transport> Session<'_, T> {
    pub(super) async fn speak_delivery(
        &mut self,
        text: &str,
        with_alarm: bool,
        delivery_abort: Arc<AtomicBool>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!self.skill_cancelled(), "delivery audio cancelled");
        anyhow::ensure!(
            !delivery_abort.load(Ordering::Relaxed),
            "delivery claim cancelled"
        );
        self.pacer.reset();
        let trace = TurnTrace::new(crate::observability::events::ANCHOR_CANNED);
        let (text_tx, text_rx) = tokio::sync::mpsc::unbounded_channel();
        let _ = text_tx.send(text.to_owned());
        drop(text_tx);
        let (mut prepared, download) =
            self.spawn_prefetch_with_abort(text_rx, &trace, delivery_abort.clone());
        let (sink, source) = new_reservoir(
            self.params.downlink_buffer_ms,
            self.params.downlink_frame_ms,
        );
        let abort = delivery_abort.clone();
        let cache = self.engines.tts_cache.clone();
        let params = self.params;
        let filler = tokio::spawn(async move {
            if with_alarm {
                push_checked(&sink, &cache, params, &abort, alarm::sentence()).await?;
            }
            let sentence = prepared
                .recv()
                .await
                .ok_or_else(|| anyhow::anyhow!("delivery speech unavailable"))?;
            push_checked(&sink, &cache, params, &abort, sentence).await
        });
        let mut first_frame_at = None;
        let t0 = Instant::now();
        let mut transport = DeliveryTransport {
            inner: &mut *self.transport,
            user_abort: self.abort.clone(),
            delivery_abort: delivery_abort.clone(),
            started: false,
        };
        let drain = drain_reservoir(
            &mut transport,
            &self.params,
            &self.session_id,
            &delivery_abort,
            &mut self.downlink_ts,
            &mut self.pacer,
            source,
            t0,
            &mut first_frame_at,
        )
        .await;
        let interrupted = drain.as_ref().map_or(true, |drain| drain.interrupted);
        if interrupted {
            delivery_abort.store(true, Ordering::Relaxed);
            filler.abort();
        }
        let completed = filler
            .await
            .map_err(|e| anyhow::anyhow!("delivery audio worker failed: {e}"));
        // Err does not carry ReservoirDrain.tts_started. Track successful starts
        // locally and also attempt cleanup on ambiguous transport errors.
        let stop = if transport.started || drain.is_err() {
            send_tts_state(&mut transport, &self.session_id, "stop").await
        } else {
            Ok(())
        };
        drop(transport);
        trace.mark_first_audio_at(first_frame_at);
        self.emit_turn_latency(&trace, interrupted);
        self.log_turn_end(&trace, interrupted);
        let drain = drain?;
        self.log_downlink(
            &self.session_id,
            trace.turn_id(),
            &drain,
            &download,
            first_frame_at,
        );
        anyhow::ensure!(!interrupted, "delivery audio interrupted");
        completed??;
        stop?; // Cleanup failure prevents submission, but never masks the original error.
        anyhow::ensure!(drain.frames > 0, "delivery audio produced no frames");
        Ok(())
    }
}
