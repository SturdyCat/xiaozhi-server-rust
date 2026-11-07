//! pure-base 下的 AIUI 兼容接口。
//!
//! AIUI 是业务级全链路模式，不属于纯底座核心 ASR/VAD/TTS/LLM 数据面。
//! pure-base 保留配置与会话 trait 的最小形状，以便旧配置和共享会话代码仍能解析，
//! 但不编译讯飞 AIUI 的网络客户端，也不执行任何业务行为。

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AiuiConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub appid: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub api_secret: String,
    #[serde(default = "default_scene")]
    pub scene: String,
    #[serde(default = "default_sn_prefix")]
    pub sn_prefix: String,
    #[serde(default = "default_voice")]
    pub voice: String,
    #[serde(default = "default_speed")]
    pub speed: i32,
    #[serde(default = "default_volume")]
    pub volume: i32,
    #[serde(default = "default_pitch")]
    pub pitch: i32,
    #[serde(default)]
    pub prompt: String,
    #[serde(default = "default_pace_ms")]
    pub pace_ms: u64,
}

fn default_scene() -> String {
    "main_box".into()
}
fn default_sn_prefix() -> String {
    "xiaozhi".into()
}
fn default_voice() -> String {
    "x6_dongmanshaonv_pro".into()
}
fn default_speed() -> i32 {
    50
}
fn default_volume() -> i32 {
    50
}
fn default_pitch() -> i32 {
    50
}
fn default_pace_ms() -> u64 {
    10
}

impl Default for AiuiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            appid: String::new(),
            api_key: String::new(),
            api_secret: String::new(),
            scene: default_scene(),
            sn_prefix: default_sn_prefix(),
            voice: default_voice(),
            speed: default_speed(),
            volume: default_volume(),
            pitch: default_pitch(),
            prompt: String::new(),
            pace_ms: default_pace_ms(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct FullChainTurn {
    pub stt: String,
    pub reply: String,
    pub audio: Vec<u8>,
}

pub trait FullChainEngine: Send {
    fn turn(&mut self, _pcm_i16: &[i16]) -> Result<FullChainTurn>;
}

/// 空底座中的惰性兼容引擎：配置可读，但启用后明确失败而不触发网络行为。
pub struct AiuiSession;

impl AiuiSession {
    pub fn new(_cfg: AiuiConfig, _sn: String) -> Self {
        Self
    }
}

impl FullChainEngine for AiuiSession {
    fn turn(&mut self, _pcm_i16: &[i16]) -> Result<FullChainTurn> {
        bail!("AIUI 全链路属于 legacy-business，pure-base 未编译")
    }
}

/// 保持共享下行接口的采样率常量；pure-base 不会产生 AIUI 音频。
pub const TTS_SAMPLE_RATE: u32 = 16_000;
