//! `stream()` 的跨 chunk 行缓冲回归。
//!
//! 背景：管道按 4096 字节切块，训练脚本末尾那行 `[train] smoke={...}` 很容易跨边界。
//! 旧实现每块直接 `.lines()`，会把一行劈成两行写进 `job.log`，冒烟 JSON 因此坏掉、
//! 被误判成 `smoke_missing`（白跑一次训练）。这里用"一次 read 只给半行"的假管道锁死它。

use super::*;
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, ReadBuf};

/// 按给定分片逐次返回的假管道（模拟一次 read 只拿到半行）。
struct Chunks {
    chunks: Vec<&'static [u8]>,
    index: usize,
}

impl AsyncRead for Chunks {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let index = self.index;
        let Some(chunk) = self.chunks.get(index).copied() else {
            return Poll::Ready(Ok(())); // EOF
        };
        self.index = index + 1;
        buf.put_slice(chunk);
        Poll::Ready(Ok(()))
    }
}

async fn reset_job() {
    let mut j = job().lock().await;
    *j = super::super::BuildJob::new();
}

#[tokio::test]
async fn a_line_split_across_two_reads_is_not_broken_in_two() {
    let _gate = super::super::tests::JOB_TEST_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_job().await;
    // 第一块在 JSON 中间断开——正是真实管道会发生的位置。
    let head = "[train] smoke={\"cases\":[{\"text\":\"提醒我喝";
    let tail = "水\",\"expected\":\"reminder\"}]}\n";
    stream(Chunks {
        chunks: vec![head.as_bytes(), tail.as_bytes()],
        index: 0,
    })
    .await;

    let log = job().lock().await.log.clone();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 1, "跨 chunk 的一行不能变成两行：{lines:?}");
    assert_eq!(lines[0], format!("{head}{tail}").trim_end());
    reset_job().await;
}

#[tokio::test]
async fn multiple_lines_in_one_chunk_and_a_partial_tail_are_flushed_at_eof() {
    let _gate = super::super::tests::JOB_TEST_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    reset_job().await;
    // 一块里两整行 + 半行；剩余半行在 EOF 时必须 flush（子进程最后一行常没有换行）。
    stream(Chunks {
        chunks: vec![b"line1\nline2\n[train] smoke", b"={\"cases\":[]}"],
        index: 0,
    })
    .await;
    let log = job().lock().await.log.clone();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(
        lines,
        vec!["line1", "line2", "[train] smoke={\"cases\":[]}"]
    );
    reset_job().await;
}
