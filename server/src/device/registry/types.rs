//! `app/registry` 的**数据模型**：对外暴露的记录类型与会话登记请求。
//!
//! 拆出本文件的唯一原因见 `AGENTS.md` §5.10（逻辑文件 ≤ 600 行）——`mod.rs` 里装着门面与
//! 全部读写方法，再放数据模型就会越线。与 `plugins/registry/fields.rs` 同一手法。
//!
//! ⚠️ 所有 `seq` 字段都是 `pub(super)`（只有父模块 `registry` 能读写）：它是**排序键**，
//! 属于模块内部实现，不是对外契约。字段保持私有还能顺手拦住"外部代码凭 seq 反推顺序"这类耦合。

/// 会话处于流水线的哪个阶段（界面上就是"这台设备在干什么"）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// 已连接、静候唤醒/说话。
    Idle,
    /// 设备已进入聆听（固件上报 `listen state=start`）。
    Listening,
    /// 识别完成、等大模型与合成（"想"）。
    Thinking,
    /// 正在下行音频（"说"）。
    Speaking,
}

impl Phase {
    /// 对外 JSON 契约值（前端据此上色/文案，**改动即破坏接口**）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Listening => "listening",
            Phase::Thinking => "thinking",
            Phase::Speaking => "speaking",
        }
    }

    /// 界面直接可用的一句话（不要求前端再维护一份中文映射表）。
    pub const fn display(self) -> &'static str {
        match self {
            Phase::Idle => "空闲",
            Phase::Listening => "聆听中",
            Phase::Thinking => "思考中",
            Phase::Speaking => "说话中",
        }
    }
}

/// 一台设备在**本进程生命周期内**的累计画像（按 `device_id` 索引）。
///
/// `first_seen_ms` / `session_count` 让"这台设备连了几次、上次什么时候来的"可回答；
/// 协商参数取自**最后一次**握手（设备改配置重连后随之更新）。
#[derive(Debug, Clone)]
pub struct DeviceRecord {
    pub device_id: String,
    /// 握手头 `Client-Id`（设备侧固件版本标识；可能与 `device_id` 相同）。
    pub client_id: String,
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
    /// 是否有活着的会话（由 [`Registry::begin_session`] / [`Registry::close_session`] 翻转）。
    pub online: bool,
    /// 累计建立的会话数（设备每次唤醒通常是一条新连接）。
    pub session_count: u64,
    /// 累计完成的**用户轮次**数（识别成功且有文本）。
    pub turn_count: u64,
    /// 最近一条会话 id。
    pub last_session_id: String,
    /// **最近一次会话结束**时的异常；干净结束则为 `None`（语义见模块注释）。
    pub last_error: Option<String>,
    // ---- 最近一次握手协商到的参数（排障第一现场：设备到底用什么参数在连）----
    pub uplink_sr: u32,
    pub downlink_sr: u32,
    pub downlink_frame_ms: u32,
    pub bin_version: u8,
    /// 设备 hello 是否声明 `features.mcp`。
    pub mcp: bool,
    /// 是否测试台连接（`hello.test=true`，macApp 联调）。
    pub is_test: bool,
    /// **排序键**（内部用，故非 `pub`）：每次"碰到"这条记录就更新，列表按它倒序 =
    /// 最近活跃在前。用序号而非时间戳的理由见模块注释（同毫秒内时间戳无法定序）。
    pub(super) seq: u64,
}

/// 一条**活着的**会话（连接）。会话结束即从表里摘除（设备画像留在 [`DeviceRecord`]）。
#[derive(Debug, Clone)]
pub struct LiveSession {
    pub session_id: String,
    pub device_id: String,
    pub client_id: String,
    pub started_ms: u64,
    pub last_activity_ms: u64,
    pub phase: Phase,
    /// 本次会话内完成的用户轮次数。
    pub turn_count: u64,
    pub uplink_sr: u32,
    pub downlink_sr: u32,
    pub downlink_frame_ms: u32,
    pub bin_version: u8,
    pub mcp: bool,
    pub is_test: bool,
    /// 最近一次识别到的用户原话（界面上"它刚听到了什么"）。
    pub last_user_text: Option<String>,
    /// 本次会话最近一次异常（`None` = 至今正常）。
    pub last_error: Option<String>,
    /// 排序键（内部用；语义同 [`DeviceRecord`]）。
    pub(super) seq: u64,
}

/// 对话记录里的一条（user 或 assistant）。
#[derive(Debug, Clone)]
pub struct TurnRecord {
    /// `"user"` / `"assistant"`（与 LLM 历史里的 role 同词，前端不必再映射）。
    pub role: &'static str,
    pub text: String,
    pub at_ms: u64,
    /// assistant 条目：整轮耗时（含合成与下行）；user 条目为 `None`。
    pub elapsed_ms: Option<u64>,
}

/// 一条会话的**最近**对话（环形，超过 [`MAX_TURNS_PER_SESSION`] 丢最旧）。
#[derive(Debug, Clone)]
pub struct ConversationRecord {
    pub session_id: String,
    pub device_id: String,
    pub started_ms: u64,
    pub last_ms: u64,
    pub turns: Vec<TurnRecord>,
    /// 被截断丢掉的条目数（>0 说明"这里不是完整对话"，界面需明示）。
    pub dropped: u64,
    /// 排序键（内部用；语义同 [`DeviceRecord`]）。
    pub(super) seq: u64,
}

impl ConversationRecord {
    /// 是否被截断（前端据此显示"仅显示最近 N 条"）。
    pub fn truncated(&self) -> bool {
        self.dropped > 0
    }
}

/// 注册表的计数快照（`GET /api/stats` 用；避免读三次锁）。
#[derive(Debug, Clone, Copy)]
pub struct Counts {
    /// 本进程见过的设备总数。
    pub devices: usize,
    /// 其中当前在线（有活会话）的**设备**台数（同设备多条连接只算一台）。
    pub online: usize,
    /// 测试台连接数（`hello.test=true`）。
    pub test_sessions: usize,
    /// 活着会话总数（同一设备可能有多条连接，故未必等于 `online`）。
    pub sessions: usize,
    /// 保留着对话记录的会话数。
    pub conversations: usize,
}

/// 会话开始时要登记的信息（由 `run_session` 从握手参数填好）。
#[derive(Debug, Clone)]
pub struct SessionStart {
    pub session_id: String,
    pub device_id: String,
    pub client_id: String,
    pub uplink_sr: u32,
    pub downlink_sr: u32,
    pub downlink_frame_ms: u32,
    pub bin_version: u8,
    pub mcp: bool,
    pub is_test: bool,
}
