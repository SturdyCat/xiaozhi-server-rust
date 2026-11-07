//! 各实现的选择、签名、校验、构建与描述符常量。
//!
//! 按职责拆分为四个子模块；公开描述符仍通过本模块保持原有路径。

mod build;
mod descriptors;
mod selection;
mod signatures;

// 子模块实现函数保持模块私有；父模块将其导入后供 descriptor 子模块使用。
use build::*;
use selection::*;
use signatures::*;

pub use descriptors::*;
