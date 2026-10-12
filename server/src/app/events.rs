//! 轻量事件总线（P0-5）：会话在关键节点 `emit` 进程级事件。
//!
//! ## 定位
//!
//! 这是一条**叠加观测层**：与 `observe.rs` 的 `note_*` 注册表埋点**并存**，不是替换——
//! `note_*` 写的是"当前在线/会话状态"这种有状态的观测面（面板直接读），而事件总线是
//! 一条**无状态、可多订阅者**的扇出通道（用于大盘 / 审计 / 未来扩展）。两者字段口径尽量
//! 对齐（session_id / device_id），但互不影响、不共享代码。
//!
//! ## 为什么用 `tokio::sync::broadcast`
//!
//! - 一对多扇出：一个会话 emit，N 个订阅者（大盘 / 日志 / 指标）各自收全量；
//! - 解耦：会话**完全不感知**谁在听，新增订阅者不必改会话代码；
//! - `send` 失败（无订阅者 / 慢消费者 LAGGED）被忽略——观测面绝不允许把语音链路带崩。
//!
//! ## 容量
//!
//! 通道容量固定 1024。这是"慢消费者"容忍上限：订阅者处理不过来会丢最旧事件（LAGGED），
//! 但发布者永不阻塞。P0 仅有调试订阅者，1024 足够；未来大盘若需零丢失，再评估换方案。

use tokio::sync::broadcast;

/// 进程级事件：会话生命周期与一轮对话的关键节点。
///
/// 所有字段均为只读快照，**不含任何密钥 / 业务决策状态**——订阅者拿到的只是"发生了什么"
/// 的事实，不能据此改变流水线行为（决策仍在会话内部）。
#[derive(Clone, Debug)]
pub enum ServerEvent {
    /// 会话建立（握手完成、引擎热刷新后、进入主循环前）。
    SessionStart {
        session_id: String,
        device_id: String,
        client_id: String,
        is_test: bool,
    },
    /// 会话结束（主循环退出、标记离线前）。`reason` 为轻量字符串：
    /// `"closed"` = 承载正常关闭；`"command"` = 指令闸门命中断开本次会话。
    SessionEnd {
        session_id: String,
        device_id: String,
        reason: String,
    },
    /// 一条消息入对话（用户说话 / 助手整轮下发完成）。`elapsed_ms` 仅助手轮次有
    /// （整轮耗时 LLM + 合成 + 下行），用户轮次为 `None`。
    MessageAdded {
        session_id: String,
        device_id: String,
        role: String,
        text: String,
        elapsed_ms: Option<u64>,
    },
    /// 一轮 TTS 下行完成（含兜底话术 / 唤醒问候语这类非 LLM 句）：句数 / 帧数 / 是否打断 / 音频时长。
    TtsRound {
        session_id: String,
        device_id: String,
        sentences: u32,
        frames: u32,
        interrupted: bool,
        audio_ms: u64,
    },
    /// 会话级异常（LLM 调用失败 / 任务异常等）。
    Error {
        session_id: String,
        device_id: String,
        message: String,
    },
}

/// 事件总线：基于 `tokio::sync::broadcast` 的进程级扇出通道。
///
/// 持有 `Sender`，`subscribe()` 产出 `Receiver`。`emit` 忽略一切发送失败——
/// 无订阅者（`SendError`）或慢消费者（`Err(Lagged)`）都不应影响发布者（会话）的运行。
pub struct EventBus {
    tx: broadcast::Sender<ServerEvent>,
}

impl EventBus {
    /// 构造总线。`capacity` 为通道缓冲上限（见模块注释，P0 用 1024）。
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    /// 新增一个订阅者（各自独立收全量事件）。
    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.tx.subscribe()
    }

    /// 发布一条事件；**忽略**发送结果（无订阅者 / LAGGED 都不影响发布者）。
    pub fn emit(&self, e: ServerEvent) {
        let _ = self.tx.send(e);
    }
}
