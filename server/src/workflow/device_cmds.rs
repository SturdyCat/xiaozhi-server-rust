//! 会话外设备命令通道：到期定时任务的**动作直推**（如「10分钟后关电源」）。
//!
//! ## 为什么需要它
//!
//! 设备 MCP 工具调用（`devices.call`）只能在会话主循环里执行：transport 的读写端都
//! 被 `Session::run` 独占，等 MCP 回复与读上行帧共用同一条连接，谁也不能绕过谁。
//! 而定时任务到期时**没有活动会话上下文**——scheduler（`runtime/skill_tasks`）手里
//! 只有 `device_id`。唯一干净的通道与本仓库的 [`crate::workflow::pipeline::inject`]
//! 同构：会话侧注册一个 `mpsc::Sender`，scheduler 往里投命令，主循环 `select!` 消费。
//!
//! 与 inject 的两点差异：
//! - **键是 `device_id`**（scheduler 只有设备号；同一设备同一时刻只有一条活动会话，
//!   后注册覆盖先注册，RAII 守卫保证断连即摘除）；
//! - **载荷带 oneshot 回执**：命令在会话循环内执行完（成功 / 设备拒绝 / 发送失败）
//!   都要回一个结果，scheduler 据此决定播报「已按时关闭电源」还是失败提示。
//!
//! ## 降级语义（与发件箱提醒的关系）
//!
//! 查无通道（设备离线）/ 通道满 / 调用失败，都**不重试**：到期动作 v1 只推一次，
//! 失败即降级为纯文字提醒（原文仍进 outbox，由设备认领播报）。旧固件本来就把
//! 到期任务当提醒，因此整条链路对旧设备零破坏。
//!
//! 全内存态、进程重启即清空（与 `inject` / `mcp::registry` 同口径）：到期任务本身
//! 落在 SQLite（`skills_tasks`），重启不丢；丢的只是「设备此刻在线」这个易逝事实。

use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};

/// 每设备命令通道容量：主循环空闲时 `try_send` 必成功；到期动作每秒最多几个、
/// 执行很快，容量 4 只是防御性排队上限，超出即报忙（本通道不是可靠队列）。
pub const CMD_CHANNEL_CAPACITY: usize = 4;

/// 会话主循环可执行的会话外命令。**只在会话循环内展开**——transport 独占权在那里。
#[derive(Debug)]
pub enum DeviceCmd {
    /// 对该设备执行一次 MCP `tools/call`（语义与计划内 `devices.call` 步骤一致，
    /// 含 `has_tool` 已声明能力校验）。结果经 `reply` 回给发送方。
    CallDeviceTool {
        tool: String,
        arguments: Value,
        reply: oneshot::Sender<Result<()>>,
    },
}

/// `DeviceCmdHub::send` 的失败原因（scheduler 据此降级为纯文字提醒）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceCmdError {
    /// 该设备没有活动会话（离线 / 测试台 / AIUI 全链路未注册）。
    NoChannel,
    /// 通道存在但接收端已关闭（会话刚结束、守卫尚未摘除的瞬间窗口）。
    Closed,
    /// 通道满（主循环正在处理上一条命令或语音轮次）。
    Busy,
}

/// 设备命令注册表。挂在 [`crate::runtime::Engines`] 上，会话侧与 scheduler 共用。
/// `Clone` 共享同一张表（scheduler 持有的只是句柄）。
#[derive(Default, Clone)]
pub struct DeviceCmdHub {
    inner: Arc<Mutex<HashMap<String, mpsc::Sender<DeviceCmd>>>>,
}

impl DeviceCmdHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// 会话侧注册一条命令通道；返回的 [`DeviceCmdGuard`] Drop 时自动摘除。
    /// 同设备重复注册（重连竞态）以后注册者为准。
    pub fn register(&self, device_id: &str, tx: mpsc::Sender<DeviceCmd>) -> DeviceCmdGuard {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(device_id.to_string(), tx);
        DeviceCmdGuard {
            inner: Arc::clone(&self.inner),
            device_id: device_id.to_string(),
        }
    }

    /// 向指定设备的活动会话投递一条命令。非阻塞：通道满即返回
    /// [`DeviceCmdError::Busy`]，绝不阻塞 scheduler。
    pub fn send(&self, device_id: &str, cmd: DeviceCmd) -> Result<(), DeviceCmdError> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(tx) = guard.get(device_id) else {
            return Err(DeviceCmdError::NoChannel);
        };
        match tx.try_send(cmd) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => Err(DeviceCmdError::Busy),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(DeviceCmdError::Closed),
        }
    }
}

/// [`DeviceCmdHub::register`] 的 RAII 守卫：Drop 即摘除该设备的命令通道。
pub struct DeviceCmdGuard {
    inner: Arc<Mutex<HashMap<String, mpsc::Sender<DeviceCmd>>>>,
    device_id: String,
}

impl Drop for DeviceCmdGuard {
    fn drop(&mut self) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.device_id);
    }
}

#[cfg(test)]
#[path = "device_cmds_tests.rs"]
mod tests;
