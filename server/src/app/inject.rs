//! 设备消息注入通道（P1-2）：管理台 HTTP 端点 → **活跃会话主循环**的单向命令通道。
//!
//! ## 为什么是"注册表 + mpsc"而不是直接调用
//!
//! 会话主循环 `run()` 独占 `&mut Session`（transport/流水线状态都不是 Send 共享体），
//! HTTP 端点无法直接触达某条连接——唯一通道是"会话侧注册一个 `mpsc::Sender`，
//! 主循环 `select!` 消费"。注册表以 `session_id` 为键：HTTP 侧先用 registry 找到设备的
//! 活跃会话，再往对应会话的通道里投递文本。
//!
//! ## 注册口径（谁能被注入）
//!
//! 仅 **设备 + 级联流水线** 会话注册（见 `session/start.rs`）：
//! - 测试台（`hello.test=true`）走独立请求-响应（asr_test 等），注入语义不通；
//! - AIUI 全链路的输入是 PCM（识别在云端闭环），没有"文本进 LLM"的入口。
//! 这两类**不注册**，HTTP 侧查无通道时报"该会话不支持注入"。
//!
//! ## 生命周期
//!
//! [`InjectGuard`] RAII：`run_session` 返回（正常结束 / `?` 提前返回 / panic 展开）
//! 即从注册表摘除——与 registry 的 `SessionGuard` 同一守卫模式，无需成对清理代码。
//!
//! 全内存态；`send` 用 `try_send`（有界 4）——会话空闲时注入立即被消费，
//! 忙时（流水线正在处理上一轮）返回 [`InjectError::Busy`]，HTTP 端点不阻塞等待。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

/// 每会话注入通道容量：主循环空闲时 `try_send` 必成功；容量 4 只是防御
/// "连点发送按钮"的排队上限，超出即报忙（注入不是可靠队列，无需积压）。
/// `pub`：`session/start.rs` 建通道时用同一常量（容量口径单点）。
pub const CHANNEL_CAPACITY: usize = 4;

/// `Injector::send` 的失败原因（HTTP 端点据此映射状态码与文案）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectError {
    /// 注册表里没有该会话（会话不存在 / 测试台 / AIUI 全链路未注册）。
    NoChannel,
    /// 通道存在但接收端已关闭（会话刚结束，注册表尚未摘除的瞬间窗口）。
    Closed,
    /// 会话忙：通道已满（流水线正在处理上一轮，注入队列达到上限）。
    Busy,
}

/// 会话注入注册表。挂在 [`crate::engine::Engines`] 上，HTTP 端点与会话侧共用。
#[derive(Default)]
pub struct Injector {
    inner: Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>,
}

impl Injector {
    pub fn new() -> Self {
        Self::default()
    }

    /// 会话侧注册一条注入通道；返回的 [`InjectGuard`] Drop 时自动摘除。
    pub fn register(&self, session_id: &str, tx: mpsc::Sender<String>) -> InjectGuard {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(session_id.to_string(), tx);
        InjectGuard {
            inner: Arc::clone(&self.inner),
            session_id: session_id.to_string(),
        }
    }

    /// 向指定会话注入一段文本（作为用户话语走完整管线）。
    /// 非阻塞：通道满即返回 [`InjectError::Busy`]，绝不阻塞 HTTP worker。
    pub fn send(&self, session_id: &str, text: String) -> Result<(), InjectError> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(tx) = guard.get(session_id) else {
            return Err(InjectError::NoChannel);
        };
        match tx.try_send(text) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err(InjectError::Busy),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(InjectError::Closed),
        }
    }
}

/// [`Injector::register`] 的 RAII 守卫：Drop 即摘除该会话的注入通道。
pub struct InjectGuard {
    inner: Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>,
    session_id: String,
}

impl Drop for InjectGuard {
    fn drop(&mut self) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.session_id);
    }
}

#[cfg(test)]
mod tests;
