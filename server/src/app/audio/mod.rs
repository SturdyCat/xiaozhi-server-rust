//! 音频处理子模块：Opus 编解码与重采样。
//!
//! 真实（`sherpa` feature）路径使用 `audiopus` / `rubato`；
//! 未启用 `sherpa` feature 的编译提供占位实现（无 libopus），仅供检查/测试编译通过；生产不含。

pub mod opus;
pub mod resample;
