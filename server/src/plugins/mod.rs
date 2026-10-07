//! 引擎插件层：ASR / TTS / LLM / VAD / AIUI 全链路，**按能力分目录**。
//!
//! 每个能力目录 = trait 定义 + 配置段 + 各厂商实现文件 + 工厂（`build_*`）。
//! 新增厂商：在对应能力目录加实现文件、实现 trait、工厂加一行注册即可，
//! 会话编排（[`crate::app::session`]）与装配器（[`crate::engine::Engines`]）不动。
//!
//! 厂商实现文件中的 `#[cfg(feature = "sherpa")]` 仅圈定本地引擎（SenseVoice /
//! Kokoro / Silero）；远程/云厂商实现无 feature 门槛。

pub mod aiui;
pub mod asr;
pub mod llm;
pub mod tts;
pub mod vad;
