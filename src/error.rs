//! 统一的错误类型与转换。
//!
//! 全项目使用 `anyhow::Error` 表达可恢复错误；在需要转成 HTTP/WebSocket
//! 响应时，通过 `AppError` 包裹并转换为可读文本。

use std::fmt;

/// 包裹 `anyhow::Error` 以便实现 `IntoResponse`（WebSocket 握手失败时使用）。
#[derive(Debug)]
pub struct AppError(pub anyhow::Error);

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AppError {}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError(e)
    }
}

/// 协议层错误：握手失败、消息解析失败、二进制帧损坏等。
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("缺少握手消息 (hello)")]
    MissingHello,
    #[error("JSON 解析失败: {0}")]
    Json(#[from] serde_json::Error),
    #[error("WebSocket 文本帧非 UTF-8")]
    InvalidUtf8,
    #[error("二进制帧长度不足，无法按 v{0} 解析")]
    TruncatedBinary(u8),
    #[error("鉴权失败")]
    Unauthorized,
    #[error("会话状态错误: {0}")]
    State(String),
}

pub type Result<T> = anyhow::Result<T>;
