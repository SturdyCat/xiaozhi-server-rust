//! [`Transport`] 的 EchoMuse 实现：把 `/em/control` + `/em/data` 两条平面翻译成
//! xiaozhi 会话语义（映射表见模块 [`super`] 注释）。
//!
//! 结构：两条 inbound 任务（routes 模块里的 control/data loop）负责"收设备、翻译、
//! 推上行 channel、经出站 channel 下发"；本结构只是**会话视角的薄壳**：
//!
//! - `recv`/`try_recv`：取上行 channel（已翻译好的 `Text`/`Binary`/`Closed`）；
//! - `send_binary`（下行 Opus，v1 裸帧）：解码回 16k PCM → 切 40ms 块 → `0x02`；
//! - `send_text`：只关心 `tts stop`（→ `0x03` EOS），其余 JSON（stt/llm/tts 句起等）
//!   对 Dot 无意义，忽略——设备不消费 xiaozhi 文本协议；
//! - `close`：向 control 任务发 [`ControlOut::Shutdown`]（连接随任务退出关闭）。
//!
//! PCM↔Opus 编解码器为本结构私有（每会话一对），与流水线的编解码器互不共享。

use anyhow::Result;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::app::audio::opus::OpusFrameDecoder;
use crate::app::transport::{IncomingFrame, Transport};

use super::frames;
use super::{ControlOut, DataOut};

pub struct EchomuseTransport {
    /// 上行（已翻译）channel。
    up_rx: UnboundedReceiver<IncomingFrame>,
    /// control 面出站。
    ctl_tx: UnboundedSender<ControlOut>,
    /// data 面出站（已组帧字节）。
    data_tx: UnboundedSender<DataOut>,
    /// 下行 Opus 解码（16k）。
    down_dec: OpusFrameDecoder,
    /// 下行 PCM 暂存（f32 解码 → i16 LE 字节，攒够一块发一条 `0x02`）。
    down_pcm: Vec<u8>,
    /// `close` 后粘滞：与 [`crate::app::transport::WsTransport`] 同语义。
    closed: bool,
}

impl EchomuseTransport {
    pub fn new(
        up_rx: UnboundedReceiver<IncomingFrame>,
        ctl_tx: UnboundedSender<ControlOut>,
        data_tx: UnboundedSender<DataOut>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            up_rx,
            ctl_tx,
            data_tx,
            down_dec: OpusFrameDecoder::new(frames::MIC_SAMPLE_RATE)?,
            down_pcm: Vec::with_capacity(frames::SPEAKER_CHUNK_BYTES * 2),
            closed: false,
        })
    }

    /// 下行：一包 Opus（v1 裸帧）→ 16k PCM → `0x02` 帧若干（攒满一块才发）。
    fn decode_to_speaker(&mut self, opus: &[u8]) -> Result<Vec<DataOut>> {
        let samples = self.down_dec.decode_frame(opus)?;
        self.down_pcm.reserve(samples.len() * 2);
        for s in samples {
            let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
            self.down_pcm.extend_from_slice(&v.to_le_bytes());
        }
        let mut out = Vec::new();
        while self.down_pcm.len() >= frames::SPEAKER_CHUNK_BYTES {
            let chunk: Vec<u8> = self.down_pcm.drain(..frames::SPEAKER_CHUNK_BYTES).collect();
            out.push(frames::frame_speaker(&chunk));
        }
        Ok(out)
    }

    /// 流末尾把不足一块的残段补零成整块发出（保证说话不被截短到 40ms 内）。
    fn flush_speaker_tail(&mut self) -> Option<DataOut> {
        if self.down_pcm.is_empty() {
            return None;
        }
        self.down_pcm.resize(frames::SPEAKER_CHUNK_BYTES, 0);
        Some(frames::frame_speaker(
            &self.down_pcm.drain(..).collect::<Vec<u8>>(),
        ))
    }
}

impl Transport for EchomuseTransport {
    async fn send_text(&mut self, json: String) -> Result<()> {
        // 只翻译 `tts stop` → speaker EOS；其余服务器 JSON 设备不消费（协议未知类型忽略），
        // 不转发——把 xiaozhi 文本漏进 EchoMuse control 面只会被设备丢弃还占带宽。
        if json.contains("\"type\":\"tts_stop\"") {
            if let Some(tail) = self.flush_speaker_tail() {
                self.data_tx
                    .send(tail)
                    .map_err(|e| anyhow::anyhow!("data 面已关闭: {e}"))?;
            }
            self.data_tx
                .send(frames::frame_speaker_eos())
                .map_err(|e| anyhow::anyhow!("data 面已关闭: {e}"))?;
        }
        Ok(())
    }

    async fn send_binary(&mut self, data: Vec<u8>) -> Result<()> {
        for frame in self.decode_to_speaker(&data)? {
            self.data_tx
                .send(frame)
                .map_err(|e| anyhow::anyhow!("data 面已关闭: {e}"))?;
        }
        Ok(())
    }

    async fn recv(&mut self) -> IncomingFrame {
        if self.closed {
            return IncomingFrame::Closed;
        }
        match self.up_rx.recv().await {
            Some(frame) => frame,
            None => {
                self.closed = true;
                IncomingFrame::Closed
            }
        }
    }

    fn try_recv(&mut self) -> Option<IncomingFrame> {
        if self.closed {
            return Some(IncomingFrame::Closed);
        }
        match self.up_rx.try_recv() {
            Ok(frame) => Some(frame),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => None,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                self.closed = true;
                Some(IncomingFrame::Closed)
            }
        }
    }

    async fn keepalive(&mut self) {
        // EchoMuse 的保活是 control 面上的 WebSocket ping（20s 周期），由 routes 里的
        // control 任务负责；会话侧无需重复。
    }

    async fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        // 让 control 任务退出（socket 随之关闭）；data 任务发现对端断开自行收尾。
        let _ = self.ctl_tx.send(ControlOut::Shutdown);
    }
}
