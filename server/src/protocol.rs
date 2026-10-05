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
//!
//! ## serde 小写 `type` 标签（AI 易踩陷阱）
//!
//! 固件发送/期望**小写** `type` 标签（`hello`/`listen`/`abort`…），故枚举用
//! `rename_all = "snake_case"`（单词变体仍为小写，多单词如 `AsrTest` → `asr_test`）。
//! 漏写会在运行时报 `unknown variant hello`，且 `cargo check` **无法发现**，
//! 只有真实 WebSocket 客户端能暴露。详情与排查见 `../agents.md` §5.1。
//!
//! ## 二进制帧封装（权威）
//!
//! 上行剥离 [`unwrap_uplink`]、下行打包 [`wrap_downlink`] 是唯一的封装/解封装出口；
//! 浏览器测试台按同样规则嗅探（首字节特征判定 v2/v3，否则视为 v1）。字节布局：
//!
//! | 版本 | 封装格式 |
//! |---|---|
//! | v1 | 裸 Opus 字节（无头） |
//! | v2 | `u16` version \| `u16` type(0=OPUS) \| `u32` reserved \| `u32` ts_ms \| `u32` size \| payload |
//! | v3 | `u8` type(0=OPUS) \| `u8` reserved \| `u16` size \| payload |
//!
//! 上行按版本剥离头部，下行严格使用与设备相同的版本；建议先用 v1 真机验证再切 v2/v3。

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
    /// 测试台标记：macApp 管理端的测试连接带 `test:true`，ESP 设备不带。
    /// 仅测试会话受理 `asr_test`/`tts_test`/`llm_test` 三种独立服务请求；
    /// 设备会话收到这三种消息会被忽略（正式流程不走测试端点）。
    #[serde(default)]
    pub test: bool,
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
    /// 测试台专用：直接调用 LLM 验证连通性（跳过 ASR/TTS，单轮无历史）。
    /// 服务端按磁盘上最新 `[llm]` 配置临时构建客户端，改完配置无需重启即可验证。
    LlmTest {
        #[serde(default)]
        session_id: Option<String>,
        text: String,
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
    /// 测试台专用：LLM 测试结果（结束时一次性下发；`state=ok` 时 `text` 为回复正文，
    /// `state=error` 时 `text` 为可读错误信息；`elapsed_ms` 为服务端直调 LLM 的总耗时）。
    #[serde(rename = "llm_test")]
    LlmTestResult {
        session_id: String,
        /// ok | error
        state: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        elapsed_ms: Option<u64>,
    },
    /// 测试台专用：TTS 合成测试结果（音频下发前/后一次性回报；`state=ok` 时
    /// `text` 为服务端合成耗时 ms，`state=error` 时为可读错误信息——引擎缺失/凭据错误
    /// 等失败不再静默，测试台对话框直接可见）。
    #[serde(rename = "tts_test")]
    TtsTestResult {
        session_id: String,
        /// ok | error
        state: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        engine: Option<String>,
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

    /// 帧内 version 字段值。下行打包必须用与设备协商相同的值：
    /// 固件上行 v2 帧写 `htons(version_)`（=2），下行解码虽不校验该字段，
    /// 但对称填 2 才是正确语义（对齐 xiaozhi-esp32 `websocket_protocol.cc`）。
    pub fn version_code(self) -> u16 {
        match self {
            BinVersion::V1 => 1,
            BinVersion::V2 => 2,
            BinVersion::V3 => 3,
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
            // version 字段必须等于与设备协商的版本（固件上行 v2 也写 2），不可硬编码。
            b.extend_from_slice(&v.version_code().to_le_bytes());
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
        // v2 帧首 2 字节是 version 字段，必须等于 2（对齐固件上行 v2 写 htons(2)）。
        assert_eq!(u16::from_le_bytes([wrapped[0], wrapped[1]]), 2);
        assert_eq!(unwrap_uplink(BinVersion::V2, &wrapped), &opus);
    }

    #[test]
    fn bin_v3_roundtrip() {
        let opus = [4u8, 5, 6];
        let wrapped = wrap_downlink(BinVersion::V3, &opus, 0);
        assert_eq!(wrapped.len(), 4 + opus.len());
        assert_eq!(unwrap_uplink(BinVersion::V3, &wrapped), &opus);
    }

    #[test]
    fn llm_test_client_message_deserializes_snake_case() {
        // ClientMessage 用 rename_all = "snake_case"：多单词变体必须是 llm_test（小写下划线），
        // 写错（如 llmTest）会在运行时报 unknown variant 且 cargo check 无法发现。
        let cm: ClientMessage =
            serde_json::from_str(r#"{"type":"llm_test","text":"你好"}"#).expect("解析失败");
        assert!(
            matches!(cm, ClientMessage::LlmTest { text, .. } if text == "你好"),
            "应反序列化为 LlmTest 变体"
        );
    }

    #[test]
    fn llm_test_result_serializes_with_snake_case_tag() {
        // ServerMessage 用 rename_all = "lowercase"（LlmTestResult 默认会变成 "llmtest"），
        // 显式 rename 后 tag 必须是 "llm_test"，与上行命名对称。
        let sm = ServerMessage::LlmTestResult {
            session_id: "s1".to_string(),
            state: "ok".to_string(),
            text: Some("回复".to_string()),
            elapsed_ms: Some(123),
        };
        let v: serde_json::Value = serde_json::from_str(&sm.to_json()).expect("序列化失败");
        assert_eq!(v["type"], "llm_test");
        assert_eq!(v["session_id"], "s1");
        assert_eq!(v["state"], "ok");
        assert_eq!(v["text"], "回复");
        assert_eq!(v["elapsed_ms"], 123);
    }
}
