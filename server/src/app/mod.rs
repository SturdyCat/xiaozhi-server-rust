//! 应用层：与具体引擎无关的运行时组件——会话编排、网关、协议、传输、下行、
//! 平台功能（OTA 托管 / 发音人目录）、令牌用量计量。引擎实现见 [`crate::plugins`]。

pub mod audio;
pub mod downlink;
pub mod firmware;
pub mod protocol;
pub mod session;
pub mod sse;
pub mod splitter;
pub mod transport;
pub mod usage;
pub mod tts_cache;
pub mod voices;
pub mod ws;
