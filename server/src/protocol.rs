//! xiaozhi-esp32 WebSocket 协议层（严格对齐本地 `xiaozhi-esp32` 固件）。
//!
//! 协议权威来源：
//! - `../xiaozhi-esp32/docs/websocket_zh.md`
//! - `../xiaozhi-esp32/main/protocols/websocket_protocol.cc`
//!
//! 关键约定：
//! - 设备 hello 的 `version` 字段 = **二进制协议版本（1/2/3）**。
//! - 服务器下行二进制帧必须使用与设备相同的版本。
//! - 服务器 hello 的 `audio_params` 被设备用作**下行（TTS）解码参数**。

use serde::{Deserialize, Serialize};

/// 音频参数：上行（设备侧）与下行（服务器侧）各一份。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioParams {
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default = "default_sr")]
    pub sample_rate: u32,
    #[serde(default = "default_channels")]
    pub channels: u16,
    #[serde(default = "default_frame")]
    pub frame_duration: u32,
}

fn default_format() -> String {
    "opus".into()
}
fn default_sr() -> u32 {
    16_000
}
fn default_channels() -> u16 {
    1
}
fn default_frame() -> u32 {
    60
}

impl Default for AudioParams {
    fn default() -> Self {
        AudioParams {
            format: default_format(),
            sample_rate: default_sr(),
            channels: default_channels(),
            frame_duration: default_frame(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientFeatures {
    #[serde(default)]
    pub mcp: bool,
    #[serde(default)]
    pub aec: bool,
}

/// 设备 → 服务器 的 hello 负载（不含外层 `type`）。
// `features`/`transport` 为固件协议字段，当前流水线未消费，保留以完整对齐协议。
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClientHello {
    /// 二进制协议版本 1/2/3。
    #[serde(default)]
    pub version: u8,
    #[serde(default)]
    pub features: Option<ClientFeatures>,
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default)]
    pub audio_params: Option<AudioParams>,
}

/// 设备 → 服务器 的文本消息。
///
/// 注意：`xiaozhi-esp32` 固件发送小写 `type` 标签（`hello`/`listen`/`abort`/`mcp`），
/// 故用 `rename_all = "snake_case"` 对齐（单词变体仍为小写，多单词如 `AsrTest` → `asr_test`），
/// 否则反序列化会报 unknown variant。
// 部分变体字段（如 session_id/mode/text/reason）为固件协议字段，当前未消费，保留以完整对齐协议。
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello(ClientHello),
    Listen {
        #[serde(default)]
        session_id: Option<String>,
        /// start | stop | detect
        state: String,
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        text: Option<String>,
    },
    Abort {
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        reason: Option<String>,
    },
    Mcp {
        #[serde(default)]
        session_id: Option<String>,
        payload: serde_json::Value,
    },
    /// 网页测试台专用：录音开始/结束后对整段缓冲做一次性 ASR（跳过 VAD 自动切段）。
    AsrTest {
        #[serde(default)]
        session_id: Option<String>,
        /// start | stop
        action: String,
    },
    /// 网页测试台专用：文本直接合成语音下发（跳过 ASR/LLM 流水线）。
    TtsTest {
        #[serde(default)]
        session_id: Option<String>,
        text: String,
        /// 语音角色（Kokoro sid），None 时用服务器配置默认值。
        #[serde(default)]
        speaker: Option<i32>,
        /// 语言（如 "zh"/"en"），None 时用服务器配置默认值。
        #[serde(default)]
        lang: Option<String>,
        /// 语速，None 时用服务器配置默认值。
        #[serde(default)]
        speed: Option<f32>,
    },
}

/// 服务器 → 设备 的文本消息。
///
/// 与设备侧一致使用小写 `type` 标签（`hello`/`stt`/`llm`/`tts`/`system`/`custom`/`mcp`）。
// `System`/`Custom` 为固件协议预留变体（如重启/自定义指令），当前未下发，保留以完整对齐协议。
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerMessage {
    Hello {
        transport: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        audio_params: Option<AudioParams>,
    },
    Stt {
        session_id: String,
        text: String,
    },
    Llm {
        session_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        emotion: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    Tts {
        session_id: String,
        /// start | stop | sentence_start
        state: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    System {
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        command: String,
    },
    Custom {
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        payload: serde_json::Value,
    },
    Mcp {
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        payload: serde_json::Value,
    },
}

impl ServerMessage {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("ServerMessage 序列化不会失败")
    }
}

// ----------------------- 二进制协议版本封装 -----------------------

/// 二进制协议版本，对应设备 hello 的 `version` 字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinVersion {
    V1,
    V2,
    V3,
}

impl BinVersion {
    pub fn from_u8(v: u8) -> BinVersion {
        match v {
            2 => BinVersion::V2,
            3 => BinVersion::V3,
            _ => BinVersion::V1,
        }
    }
}

/// 下行封装：将 Opus 帧按版本打包。
/// - v1：裸 Opus 字节。
/// - v2：u16 version | u16 type(0=OPUS) | u32 reserved | u32 ts_ms | u32 size | payload
/// - v3：u8 type(0=OPUS) | u8 reserved | u16 size | payload
pub fn wrap_downlink(v: BinVersion, opus: &[u8], ts_ms: u32) -> Vec<u8> {
    match v {
        BinVersion::V1 => opus.to_vec(),
        BinVersion::V2 => {
            let mut b = Vec::with_capacity(16 + opus.len());
            b.extend_from_slice(&1u16.to_le_bytes()); // version
            b.extend_from_slice(&0u16.to_le_bytes()); // type = OPUS
            b.extend_from_slice(&0u32.to_le_bytes()); // reserved
            b.extend_from_slice(&ts_ms.to_le_bytes());
            b.extend_from_slice(&(opus.len() as u32).to_le_bytes());
            b.extend_from_slice(opus);
            b
        }
        BinVersion::V3 => {
            let mut b = Vec::with_capacity(4 + opus.len());
            b.push(0); // type = OPUS
            b.push(0); // reserved
            b.extend_from_slice(&(opus.len() as u16).to_le_bytes());
            b.extend_from_slice(opus);
            b
        }
    }
}

/// 上行剥离：从设备发来的二进制帧中取出 Opus 载荷。
pub fn unwrap_uplink(v: BinVersion, data: &[u8]) -> &[u8] {
    match v {
        BinVersion::V1 => data,
        BinVersion::V2 => {
            if data.len() < 16 {
                return &[];
            }
            let size = u32::from_le_bytes([data[12], data[13], data[14], data[15]]) as usize;
            if data.len() < 16 + size {
                return &[];
            }
            &data[16..16 + size]
        }
        BinVersion::V3 => {
            if data.len() < 4 {
                return &[];
            }
            let size = u16::from_le_bytes([data[2], data[3]]) as usize;
            if data.len() < 4 + size {
                return &[];
            }
            &data[4..4 + size]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bin_v1_roundtrip() {
        let opus = [1u8, 2, 3, 4, 5];
        let wrapped = wrap_downlink(BinVersion::V1, &opus, 0);
        assert_eq!(wrapped, opus);
        assert_eq!(unwrap_uplink(BinVersion::V1, &wrapped), &opus);
    }

    #[test]
    fn bin_v2_roundtrip() {
        let opus = [9u8, 8, 7, 6];
        let wrapped = wrap_downlink(BinVersion::V2, &opus, 1234);
        assert_eq!(wrapped.len(), 16 + opus.len());
        assert_eq!(unwrap_uplink(BinVersion::V2, &wrapped), &opus);
    }

    #[test]
    fn bin_v3_roundtrip() {
        let opus = [4u8, 5, 6];
        let wrapped = wrap_downlink(BinVersion::V3, &opus, 0);
        assert_eq!(wrapped.len(), 4 + opus.len());
        assert_eq!(unwrap_uplink(BinVersion::V3, &wrapped), &opus);
    }
}
