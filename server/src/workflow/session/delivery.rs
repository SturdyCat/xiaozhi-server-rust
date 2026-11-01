//! Durable outbox -> normal paced canned audio, never a synthetic user turn.
use super::{skill_plan::now_ms, Session};
use crate::workflow::protocol::Transport;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

impl<T: Transport> Session<'_, T> {
    /// Called only from the idle main loop. AIUI owns its cloud audio and cannot
    /// use this local TTS path; test clients are not delivery devices.
    pub(super) async fn deliver_pending_message(&mut self) {
        if !self.delivery_idle
            || self.is_test
            || self.aiui.is_some()
            || !self.pending_segments.is_empty()
            || self.close_requested
            || self.abort.load(Ordering::Relaxed)
        {
            return;
        }
        let system = self.engines.skill_system.clone();
        let device = self.device_id.clone();
        let session = self.session_id.clone();
        let claim =
            tokio::task::spawn_blocking(move || system.claim_delivery(&device, &session, now_ms()))
                .await;
        let claim = match claim {
            Ok(Ok(Some(claim))) => claim,
            Ok(Ok(None)) => return,
            error => {
                self.note_error(format!("outbox claim failed: {error:?}"));
                return;
            }
        };
        let system = self.engines.skill_system.clone();
        let check = claim.clone();
        let current =
            tokio::task::spawn_blocking(move || system.delivery_is_current(&check, now_ms()))
                .await
                .is_ok_and(|result| result.unwrap_or(false));
        let mut submitted = false;
        if current && !self.skill_cancelled() {
            // Re-check task cancellation while TTS may be blocked on network or
            // pacing. Failure is fail-closed, never proof that audio was played.
            let system = self.engines.skill_system.clone();
            let check = claim.clone();
            let delivery_abort = Arc::new(AtomicBool::new(false));
            let abort = delivery_abort.clone();
            let watcher = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    let system = system.clone();
                    let check = check.clone();
                    let current = tokio::task::spawn_blocking(move || {
                        system.delivery_is_current(&check, now_ms())
                    })
                    .await
                    .is_ok_and(|result| result.unwrap_or(false));
                    if !current {
                        abort.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            });
            let result = self
                .speak_delivery(&claim.text, claim.task_id.is_some(), delivery_abort.clone())
                .await;
            watcher.abort();
            let _ = watcher.await;
            submitted = result.is_ok()
                && !delivery_abort.load(Ordering::Relaxed)
                && !self.abort.load(Ordering::Relaxed);
            if let Err(error) = result {
                self.note_error(format!("outbox audio submission failed: {error:#}"));
            }
        }
        let system = self.engines.skill_system.clone();
        let id = claim.id.clone();
        let finished = tokio::task::spawn_blocking(move || {
            system.finish_delivery(&claim, submitted, now_ms())
        })
        .await;
        match finished {
            Ok(Ok(true)) => tracing::info!(
                "session {} outbox {id} submitted={submitted}; not a played acknowledgement",
                self.session_id
            ),
            Ok(Ok(false)) => tracing::warn!(
                "session {} outbox {id} lease/cancellation changed before finish",
                self.session_id
            ),
            error => self.note_error(format!("outbox finish failed: {error:?}")),
        }
    }
}
