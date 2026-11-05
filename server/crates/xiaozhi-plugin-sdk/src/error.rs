//! SDK 错误类型：guest 侧构建期/运行期的显式失败载体。

use std::fmt;

/// SDK 操作错误（不含宿主错误——那是 wasm 边界外的事）。
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// 内存分配失败（`fallible` 模块的显式形态；guest OOM panic 会成为陷阱，
    /// 走这条路则保持业务性失败）。`0` = 上下文（哪一步分配）。
    Alloc(String),
    /// 重复注册（菜单 entry / 动作 id / 字段 key / 权限项）。
    Duplicate(String),
    /// 形状非法（id / 槽位 / kind / 枚举值等，规则与宿主 `contract::validate_*` 同源）。
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Alloc(ctx) => write!(f, "分配失败：{ctx}"),
            Error::Duplicate(what) => write!(f, "重复注册：{what}"),
            Error::Invalid(why) => write!(f, "形状非法：{why}"),
        }
    }
}

impl std::error::Error for Error {}

/// SDK 专用 Result。
pub type Result<T> = std::result::Result<T, Error>;
