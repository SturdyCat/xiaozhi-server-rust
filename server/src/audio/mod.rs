//! 音频处理子模块：Opus 编解码与重采样。
//!
//! 真实（`sherpa` feature）路径使用 `audiopus` / `rubato`；
//! 默认（mock）路径提供占位实现，使无模型本地联调可编译运行。

pub mod opus;
pub mod resample;
