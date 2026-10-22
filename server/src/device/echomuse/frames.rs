//! EchoMuse 设备 ↔ 服务器 二进制帧协议（`/em/data` 面）。
//!
//! 权威来源：EchoMuse 仓库 `docs/device-controller-interface.md`（以代码为准）。
//! 首字节是帧型，其余是负载；**帧型按方向分命名空间**——`0x04`/`0x05` 在两个方向
//! 含义不同（下行是音乐帧，上行是 VAD 哨兵），这是协议刻意的设计。
//!
//! 当前实现范围（MVP + 蓝牙代理）：
//! - 上行：`0x01` mic PCM（mono S16_LE 16kHz）、`0x07` 私听会话音频
//!   （`[session u32 BE][seq u16 BE][PCM]` 头）、`0x04`/`0x05` bounded-turn 哨兵、
//!   `0x06` BLE 广播批次（`ble_proxy` 开启时经 [`super::ble`] 采集）。
//! - 下行：`0x02` speaker PCM（**mono S16_LE 48 kHz**，见 [`SPEAKER_SAMPLE_RATE`]）、
//!   `0x03` speaker EOS、`0x08` BLE 连接桥 JSON（管理面直驱 GATT 请求）。
//! - 其余帧型（音乐面 `0x04/0x05` 下行）**不解析、静默忽略**——协议规定"未知帧型
//!   双向忽略"，即正确的前向兼容降级。
//!
//! `ble_connect`/`ble_adverts_data` 是能力协商的产物：服务器 ack 不声明时设备不发
//! `0x06`/`0x08`（见 [`super::ack_message`]），data 任务也只在 `ble_proxy` 开启时翻译。

/// 上行：mic PCM 块（mono S16_LE, 16 kHz）。
pub const FRAME_MIC: u8 = 0x01;
/// 上行：bounded turn 内检测到语音后结束（正常收口）。
pub const FRAME_VAD_END: u8 = 0x04;
/// 上行：bounded turn 内超时无语音（静默放弃本轮）。
pub const FRAME_NO_SPEECH: u8 = 0x05;
/// 上行：私听会话音频，头 `session u32 BE + seq u16 BE`，其后是 PCM。
pub const FRAME_SESSION_AUDIO: u8 = 0x07;
/// 上行：BLE 广播批次（`{"adverts":[...]}` JSON，UTF-8）。仅当 ack 声明
/// `ble_adverts_data` 后设备才发送；负载由 [`super::ble::BleLink::merge_adverts`] 消费。
pub const FRAME_BLE_ADVERTS: u8 = 0x06;
/// 双向：BLE 连接桥一条 JSON 消息（服务器请求 / 设备 result·notify·disconnected·slots）。
/// 仅当双方都在 ack 声明 `ble_connect` 后使用；负载由 [`super::ble`] 收发。
pub const FRAME_BLE_GATT: u8 = 0x08;
/// 下行：voice-response PCM 块（设备立即播放，mono S16_LE 48 kHz）。
pub const FRAME_SPEAKER: u8 = 0x02;
/// 下行：voice 流结束。
pub const FRAME_SPEAKER_EOS: u8 = 0x03;

/// 设备 mic 音频固定 16 kHz 单声道（协议常量，不随配置变）。
///
/// 生产侧用 [`crate::media::media::AudioFormat::echo_mic`] 表达同一事实；本常量
/// 是**独立锚点**，供上行断言与文档引用——两处不一致即为协议适配 bug。
#[allow(dead_code)]
pub const MIC_SAMPLE_RATE: u32 = 16_000;

/// 设备 speaker 线缆采样率：**48 kHz 单声道 S16_LE**（协议常量，不随配置变）。
///
/// 上游依据（`docs/device-controller-interface.md` + `controller/CLAUDE.md`）：
/// 控制器把 TTS 用 ffmpeg 直接解码成 "48kHz mono" 后发 `0x02` 帧；设备端 DAC 也是
/// 48 kHz（ALSA 写入时把单声道复制成 L=R），**收到 PCM 不做任何重采样**。
/// ⚠️ 早期按 16 kHz 下发 → 设备按 48k 播放 → 3 倍速、音调拔高（"快进尖叫"）。
pub const SPEAKER_SAMPLE_RATE: u32 = 48_000;

/// 下行 PCM 块时长（毫秒）。与上行 mic 块对齐，便于按块计时。
pub const SPEAKER_FRAME_MS: u32 = 40;

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
    /// BLE 广播批次负载（`0x06` 后的 JSON 字节）。
    BleAdverts(&'a [u8]),
    /// BLE 连接桥 JSON 负载（`0x08` 后的字节）。
    BleGatt(&'a [u8]),
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
        FRAME_BLE_ADVERTS => UplinkFrame::BleAdverts(&data[1..]),
        FRAME_BLE_GATT => UplinkFrame::BleGatt(&data[1..]),
        other => UplinkFrame::Unknown(other),
    }
}

/// Sole outbound kind classifier used by the socket writer's PCM ticket adapter.
pub(crate) fn is_voice(data: &[u8]) -> bool {
    matches!(
        data.first(),
        Some(&FRAME_SPEAKER) | Some(&FRAME_SPEAKER_EOS)
    )
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

/// 打一条下行 BLE 连接桥帧（`0x08` + JSON 文本；管理面 GATT 请求用）。
pub fn frame_ble_gatt(json: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(json.len() + 1);
    out.push(FRAME_BLE_GATT);
    out.extend_from_slice(json.as_bytes());
    out
}

/// 下行 PCM 块大小（字节）：40ms @48kHz mono S16LE = 3840 字节。
///
/// 按 [`SPEAKER_SAMPLE_RATE`]/[`SPEAKER_FRAME_MS`] 推导，改采样率时不会与分帧器
/// 的 `sample_rate × frame_ms` 口径漂移（漂移的表现是每块都被 media 层拒收、
/// 静默无声）。块小有利时延；3840B 在 TCP 上会拆成 3 段，可接受。
pub const SPEAKER_CHUNK_BYTES: usize =
    (SPEAKER_SAMPLE_RATE as usize * SPEAKER_FRAME_MS as usize / 1000) * 2;
