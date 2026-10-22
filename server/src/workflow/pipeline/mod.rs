//! 流水线环节（原 app 根：推理闸门 / 分句 / 注入 / TTS 缓存 / 预取）。
//!
//! 每个环节是独立的编排组件，由会话状态机按阶段调用，串起
//! 完整 LLM→TTS→下行管线。

pub mod inference;
pub mod inject;
pub mod prefetch;
pub mod splitter;
pub mod tts_cache;
