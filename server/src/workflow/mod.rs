//! 对话工作流核心：会话状态机 + 流水线环节（推理/分句/注入/TTS 缓存/预取）
//! + 会话协议与传输抽象 + 下行音频流水线 + MCP 客户端框架。

pub mod downlink;
pub mod mcp;
pub mod pipeline;
pub mod protocol;
pub mod session;
