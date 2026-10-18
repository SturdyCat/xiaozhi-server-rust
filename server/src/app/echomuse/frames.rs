//! EchoMuse 设备 ↔ 服务器 二进制帧协议（`/em/data` 面）。
//!
//! 权威来源：EchoMuse 仓库 `docs/device-controller-interface.md`（以代码为准）。
//! 首字节是帧型，其余是负载；**帧型按方向分命名空间**——`0x04`/`0x05` 在两个方向
//! 含义不同（下行是音乐帧，上行是 VAD 哨兵），这是协议刻意的设计。
//!
//! 当前实现范围（MVP）：
//! - 上行：`0x01` mic PCM（mono S16_LE 16kHz）、`0x07` 私听会话音频
//!   （`[session u32 BE][seq u16 BE][PCM]` 头）、`0x04`/`0x05` bounded-turn 哨兵。
//! - 下行：`0x02` speaker PCM、`0x03` speaker EOS。
//! - 其余帧型（音乐面 `0x04/0x05` 下行、`0x06/0x08` BLE）**不解析、静默忽略**——
//!   协议规定"未知帧型双向忽略"，即正确的前向兼容降级。

/// 上行：mic PCM 块（mono S16_LE, 16 kHz）。
pub const FRAME_MIC: u8 = 0x01;
/// 上行：bounded turn 内检测到语音后结束（正常收口）。
pub const FRAME_VAD_END: u8 = 0x04;
/// 上行：bounded turn 内超时无语音（静默放弃本轮）。
pub const FRAME_NO_SPEECH: u8 = 0x05;
/// 上行：私听会话音频，头 `session u32 BE + seq u16 BE`，其后是 PCM。
pub const FRAME_SESSION_AUDIO: u8 = 0x07;
/// 下行：voice-response PCM 块（设备立即播放）。
pub const FRAME_SPEAKER: u8 = 0x02;
/// 下行：voice 流结束。
pub const FRAME_SPEAKER_EOS: u8 = 0x03;

/// 设备 mic 音频固定 16 kHz 单声道（协议常量，不随配置变）。
pub const MIC_SAMPLE_RATE: u32 = 16_000;
/// 上行编码帧长（样本数）：60 ms @16kHz，与 xiaozhi 会话流水线的 Opus 帧长一致。
pub const MIC_FRAME_SAMPLES: usize = 960;

/// `0x07` 帧头长度（session u32 + seq u16）。
pub const SESSION_HEADER_LEN: usize = 6;

/// 上行帧解析结果。
#[derive(Debug, PartialEq, Eq)]
pub enum UplinkFrame<'a> {
    /// mic PCM（已剥掉可能的会话头）。
    MicPcm(&'a [u8]),
    /// 语音段正常结束。
    VadEnd,
    /// 超时无语音。
    NoSpeech,
    /// 未知帧型（按协议静默忽略）。
    Unknown(u8),
}

/// 解析一条设备 → 服务器 的 `/em/data` 二进制消息。
pub fn parse_uplink(data: &[u8]) -> UplinkFrame<'_> {
    let Some(&kind) = data.first() else {
        return UplinkFrame::Unknown(0);
    };
    match kind {
        FRAME_MIC => UplinkFrame::MicPcm(&data[1..]),
        FRAME_SESSION_AUDIO => {
            // 0x07 头：session u32 BE + seq u16 BE；截断的帧按未知忽略（不该发生）。
            if data.len() > SESSION_HEADER_LEN {
                UplinkFrame::MicPcm(&data[1 + SESSION_HEADER_LEN..])
            } else {
                UplinkFrame::Unknown(kind)
            }
        }
        FRAME_VAD_END => UplinkFrame::VadEnd,
        FRAME_NO_SPEECH => UplinkFrame::NoSpeech,
        other => UplinkFrame::Unknown(other),
    }
}

/// 打一条下行 speaker PCM 帧（`0x02` + PCM）。
pub fn frame_speaker(pcm: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pcm.len() + 1);
    out.push(FRAME_SPEAKER);
    out.extend_from_slice(pcm);
    out
}

/// 打一条下行 speaker EOS 帧（`0x03`）。
pub fn frame_speaker_eos() -> Vec<u8> {
    vec![FRAME_SPEAKER_EOS]
}

/// 下行 PCM 块大小（字节）：40ms @16kHz mono S16LE = 1280 字节。
/// 设备侧立即播放，块小有利时延；TCP 上 1280B 仍是单包，不碎。
pub const SPEAKER_CHUNK_BYTES: usize = 1280;
