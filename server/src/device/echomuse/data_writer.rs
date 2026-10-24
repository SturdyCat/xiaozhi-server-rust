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
                // 心跳/控制帧不是断链信号：Ping 由 axum 自动回 Pong，Pong 是设备回我们
                // 的心跳。此前 `_ => break false` 会把一次 Pong 当作"发送失败"返回 false，
                // 上层 data_loop 随即 `if !written { break }` 断开整条链（设备侧 close
                // 1006）——等于服务端自己的保活把自己踢下线。
                Some(Ok(Message::Text(_)))
                | Some(Ok(Message::Ping(_)))
                | Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) => break false,
                // 读错误 / 流结束：真断链，放弃本次写。
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
