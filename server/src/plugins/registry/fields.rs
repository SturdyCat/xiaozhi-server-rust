//! 注册表字段元数据的兼容导出层。
//!
//! 公共能力字段与实现私有字段按职责分别维护；本模块保留原有
//! `super::fields::*` 导入路径，避免描述符和 schema 投影改变。

pub(super) use super::fields_common::*;
pub(super) use super::fields_impl::*;
