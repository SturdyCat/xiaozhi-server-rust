//! A stalled output socket must not starve mic sentinels/control/data EOF.
use super::{frames, media::MediaState, DataOut};
use axum::extract::ws::Message;
use futures_util::{Sink, SinkExt, Stream, StreamExt};

pub(super) async fn write_frame<S, R, E>(
    sink: &mut S,
    stream: &mut R,
    frame: DataOut,
    media: &MediaState,
    mut incoming: impl FnMut(&[u8]),
) -> bool
where
    S: Sink<Message> + Unpin,
    R: Stream<Item = Result<Message, E>> + Unpin,
{
    let ticket = if frames::is_voice(&frame) {
        match media.take_write() {
            Some(ticket) if ticket.is_current() => Some(ticket),
            _ => return true, // Old/unticketed PCM and EOS are never replayed.
        }
    } else {
        None
    };
    let changed = media.changed.notified();
    tokio::pin!(changed);
    changed.as_mut().enable(); // Register BEFORE checking state: no lost cancellation wake.
    if !media.ready() || ticket.as_ref().is_some_and(|t| !t.is_current()) {
        return false;
    }
    let write = sink.send(Message::Binary(frame.into()));
    tokio::pin!(write);
    let written = loop {
        tokio::select! {
            result = &mut write => break result.is_ok(),
            _ = &mut changed => break false,
            inbound = stream.next() => match inbound {
                Some(Ok(Message::Binary(raw))) => incoming(&raw),
                Some(Ok(Message::Text(_))) => {},
                _ => break false,
            }
        }
    };
    if let Some(ticket) = ticket {
        let current = ticket.is_current();
        ticket.complete(written && current);
    }
    written
}

#[cfg(test)]
mod tests;
