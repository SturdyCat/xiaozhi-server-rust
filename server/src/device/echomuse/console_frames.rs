//! 控制台帧协议（服务端唯一出处）：常量、帧定义、入站解析、限流与 UTF-8 对齐纯函数。
//!
//! **前后端字面量必须逐字一致**（设计 §5.2 / §8.1）：`type` 取值、`error.code`、
//! `closed.reason`、`notice.code`、`ready.mode` 全部由本文件的 serde 派生
//! （`#[serde(rename_all = "kebab-case")]`）与 [`CloseReason::code`] 决定；客户端
//! `EchoMuseConsoleState` 按同名常量解析。
//!
//! 本文件只放**纯函数**（无 IO、无全局状态），供 `console_tests.rs` 直驱断言；网络、
//! 槽与收尾顺序在 `console.rs`。`tokio::time::Instant` 而非 `std::time::Instant`：
//! 会话定时器与令牌桶共用同一时钟类型，避免两次 `from_std` 转换（真实时钟下等价）。

use std::time::Duration;

use serde::Serialize;
use tokio::time::Instant;

// ---- 常量（§8.2：两侧同值） ----

/// 等设备回拨 `/shell/{id}` 的期限。与一次性操作同值，但语义**只是回拨期限**：
/// 会话时长由 [`IDLE_TIMEOUT`]/[`SESSION_MAX`] 管（设计 §5.1）。
pub(super) const DIAL_TIMEOUT: Duration = Duration::from_secs(15);
/// 空闲上限：600 s 无**输入**（输出流动不重置计时）。
pub(super) const IDLE_TIMEOUT: Duration = Duration::from_secs(600);
/// 单会话硬上限：7200 s。
pub(super) const SESSION_MAX: Duration = Duration::from_secs(7200);
/// 单帧上限：设备本来 4 KiB 一块（`device_control.go:1170`），这里按 8 KiB 再切一次。
pub(super) const FRAME_MAX: usize = 8 * 1024;
/// 会话累计转发上限：达上限后发一次 `notice{output-capped}` 并停止转发，**不断会话**。
pub(super) const SESSION_BYTES_CAP: u64 = 16 * 1024 * 1024;
/// 单行输入上限（字节）：超过即 `error{input-too-long}`，一个字节都不写设备。
pub(super) const LINE_MAX: usize = 4096;
/// 输入速率：令牌桶 10 帧/秒（容量 = 速率，允许一次性连发 10 条）。
pub(super) const INPUT_PER_SEC: u32 = 10;
/// 浏览器入站帧上限：`LINE_MAX` 的 4 倍足够容纳 JSON 包装；更大的帧不是"一行命令"，
/// 按协议违规直接断开（正常输入框发不出来），比无限缓冲安全。
pub(super) const INBOUND_MAX: usize = LINE_MAX * 4;
/// `ready.mode` 取值之一：老固件忽略 `pty` 字段时设备仍是管道，无提示符、无回显。
/// 保留这个值而不是删掉，是为了让前端能按设备**实际**给出的模式调整提示符与回显口径。
#[allow(dead_code)]
pub(super) const MODE_PIPE: &str = "pipe";
/// `ready.mode` 取值之一：真终端（mksh 挂在 pty 上，有提示符、回显、行编辑）。
/// 控制台默认按这个模式开会话，与官方 dashboard 的 `shell_open{pty:true}` 一致。
pub(super) const MODE_PTY: &str = "pty";

// ---- 文案（服务端唯一出处，与 §8.3 前端 `ConsoleCopy` 逐字对齐） ----

pub(super) const COPY_BUSY: &str = "设备已被另一个操作占用（系统信息/日志/资产下发），请稍后重试";
pub(super) const COPY_INPUT_TOO_LONG: &str = "单行命令过长（上限 4096 字节），未发送";
pub(super) const COPY_RATE_LIMITED: &str = "输入过快，已丢弃（每次最多 10 条/秒）";
pub(super) const COPY_DEVICE_OFFLINE: &str = "设备离线——控制台不可用";
pub(super) const COPY_INTERNAL: &str = "控制台内部错误：";
pub(super) const COPY_CAPPED: &str = "会话输出已达 16 MiB 上限，后续输出不再显示（命令仍可执行）";
pub(super) const COPY_READY_NOTICE: &str =
    "终端：设备 root shell（pty），cd/环境变量在同一会话里保持。审计会记录你输入的每一行命令";

// ---- 帧定义 ----

/// `error.code`（§8.1 五个，缺一不可、多一不可）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum ErrorCode {
    Busy,
    InputTooLong,
    RateLimited,
    DeviceOffline,
    Internal,
}

impl ErrorCode {
    /// code 对应的中文说明；`detail` 只对 [`ErrorCode::Internal`] 生效（其余 code 的
    /// 文案是定值，不受调用方影响，避免同 code 多套文案漂移）。
    pub(super) fn message(self, detail: &str) -> String {
        match self {
            Self::Internal => format!("{COPY_INTERNAL}{detail}"),
            Self::Busy => COPY_BUSY.to_string(),
            Self::InputTooLong => COPY_INPUT_TOO_LONG.to_string(),
            Self::RateLimited => COPY_RATE_LIMITED.to_string(),
            Self::DeviceOffline => COPY_DEVICE_OFFLINE.to_string(),
        }
    }
}

/// `notice.code`（§8.1 只有 `output-capped`：输入从不"暂停"，会话封顶后仍可写）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum NoticeCode {
    OutputCapped,
}

/// `closed.reason`（§8.1 八个 + 仅审计用的 `client-closed`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum CloseReason {
    IdleTimeout,
    SessionLimit,
    DeviceOffline,
    Superseded,
    DeviceShellLost,
    /// 防御性：槽被非预期接管（正常不发生）。
    #[allow(dead_code)]
    Replaced,
    /// 保留码：服务端无信号处理（§0.3），一期**不发**本帧，仅供类型完整与将来接线。
    #[allow(dead_code)]
    ServerShutdown,
    Internal,
    /// 浏览器先关：不发 `closed` 帧（对端已走），只进审计行。
    ClientClosed,
}

impl CloseReason {
    /// 审计行里用的 kebab-case 字面量。与 serde 输出必须一致——`console_tests.rs`
    /// 逐个变体断言两者相等，防止字面量漂移。
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::IdleTimeout => "idle-timeout",
            Self::SessionLimit => "session-limit",
            Self::DeviceOffline => "device-offline",
            Self::Superseded => "superseded",
            Self::DeviceShellLost => "device-shell-lost",
            Self::Replaced => "replaced",
            Self::ServerShutdown => "server-shutdown",
            Self::Internal => "internal",
            Self::ClientClosed => "client-closed",
        }
    }
}

/// 服务端 → 浏览器帧。内部标签 `type`，字段名与 §5.2 表逐字一致。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(super) enum ConsoleFrame {
    Ready {
        device_id: String,
        mode: &'static str,
        insecure: bool,
        notice: String,
    },
    Output {
        text: String,
    },
    Notice {
        code: NoticeCode,
        text: String,
    },
    Error {
        code: ErrorCode,
        error: String,
    },
    Closed {
        reason: CloseReason,
        detail: String,
        inputs: u32,
        bytes_out: u64,
        secs: u64,
    },
    Pong,
}

/// 编码成一段 JSON 文本帧；序列化失败（结构上不可能）也要给浏览器一个可解析的帧，
/// 绝不发空帧让前端卡在 `connecting`。
pub(super) fn encode(frame: &ConsoleFrame) -> String {
    serde_json::to_string(frame).unwrap_or_else(|_| {
        r#"{"type":"error","code":"internal","error":"控制台内部错误：帧编码失败"}"#.to_string()
    })
}

/// 浏览器 → 服务端（§5.2 方向 2）。`clear` 不存在：清屏是纯 UI 动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Incoming {
    Input(String),
    CtrlC,
    Ping,
    Unknown,
}

/// 解析入站文本帧。未知 type / 非法 JSON / `data` 不是字符串一律 [`Incoming::Unknown`]
/// 并静默忽略（与项目"未知帧型双向忽略"的口径一致），绝不当成空命令写设备。
pub(super) fn parse_incoming(text: &str) -> Incoming {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Incoming::Unknown;
    };
    match value.get("type").and_then(|t| t.as_str()) {
        Some("input") => match value.get("data").and_then(|d| d.as_str()) {
            // 前端的输入框可能带尾部换行；协议规定 data **不含** `\n`，
            // 这里容忍并剥掉尾部 CR/LF，避免发出一行空命令。
            Some(data) => Incoming::Input(data.trim_end_matches(['\r', '\n']).to_string()),
            None => Incoming::Unknown,
        },
        Some("ctrl-c") => Incoming::CtrlC,
        Some("ping") => Incoming::Ping,
        _ => Incoming::Unknown,
    }
}

/// 设备回拨后可能先发一帧 `shell_meta{pty}`（官方 `handle_shell` 在转发任何终端字节
/// **之前**发出，dashboard 正是靠它决定走 pty 还是 legacy 行模式）。
///
/// 这一帧不是终端输出：写进画面会在提示符前多出一行 JSON；丢掉又会让前端失去
/// "设备到底支持不支持 pty"的唯一信号。所以单独识别、原样转给浏览器，由前端决定
/// 提示符风格与回显口径。
///
/// 用 JSON 解析而不是 `contains`：任何命令输出都可能包含 `shell_meta` 这几个字符
/// （`grep meta /proc/...`），误判会把真实输出当控制帧吞掉。
pub(super) fn is_shell_meta(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| {
            value
                .get("type")
                .and_then(|t| t.as_str())
                .map(|t| t == "shell_meta")
        })
        .unwrap_or(false)
}

// ---- 限流 / 长度 / 输出封顶（纯判定） ----
/// 单行长度判定：>4096 **字节**（不是字符）即拒。
pub(super) fn check_line(data: &str) -> Result<(), ErrorCode> {
    if data.len() > LINE_MAX {
        return Err(ErrorCode::InputTooLong);
    }
    Ok(())
}

/// 一条入站输入的处理决策：要么给出一段"要写设备的字节"，要么给出拒绝码。
///
/// 刻意把"写什么字节"与"能不能写"合在一个纯函数里——这样"被拒时设备侧零字节"
/// 是**类型上**成立（[`InputAction::Reject`] 不携带字节），不依赖调用方自觉。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum InputAction {
    Write(Vec<u8>),
    Reject(ErrorCode),
}

/// `input` 帧：令牌桶 → 长度 → `utf8(data) + "\n"`（pipe 模式的 `sh` 只在读到换行时
/// 执行一条完整命令行，缺换行会一直攥在行缓冲里，见 `shell.rs::push` 同源注释）。
pub(super) fn plan_line(bucket: &mut TokenBucket, now: Instant, data: &str) -> InputAction {
    if !bucket.allow(now) {
        return InputAction::Reject(ErrorCode::RateLimited);
    }
    if let Err(code) = check_line(data) {
        return InputAction::Reject(code);
    }
    let mut bytes = data.as_bytes().to_vec();
    bytes.push(b'\n');
    InputAction::Write(bytes)
}

/// `ctrl-c` 帧：单字节 `0x03`（设备侧 pipe stdin 直写 raw 字节）。
pub(super) fn plan_ctrl_c(bucket: &mut TokenBucket, now: Instant) -> InputAction {
    if !bucket.allow(now) {
        return InputAction::Reject(ErrorCode::RateLimited);
    }
    InputAction::Write(vec![0x03])
}

/// 令牌桶：容量 = 速率（允许一次性连发 `INPUT_PER_SEC` 条），按经过时间补充。
#[derive(Debug)]
pub(super) struct TokenBucket {
    per_sec: f32,
    capacity: f32,
    tokens: f32,
    last: Option<Instant>,
}

impl TokenBucket {
    pub(super) fn new(per_sec: u32) -> Self {
        let capacity = per_sec.max(1) as f32;
        Self {
            per_sec: capacity,
            capacity,
            tokens: capacity,
            last: None,
        }
    }

    /// 取一个令牌；失败不扣（拒绝本身不消耗额度）。
    pub(super) fn allow(&mut self, now: Instant) -> bool {
        if let Some(last) = self.last {
            let elapsed = now.saturating_duration_since(last).as_secs_f32();
            self.tokens = (self.tokens + elapsed * self.per_sec).min(self.capacity);
        }
        self.last = Some(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// 输出转发决策。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Admit {
    /// 正常转发。
    Forward,
    /// 本次越界：发一次 `notice{output-capped}`，本块丢弃。
    Cap,
    /// 已封顶：静默丢弃（不重复发 notice）。
    Drop,
}

/// 会话累计输出预算。刻意不复用一次性操作的 `MAX_OUTPUT`（64 KiB 硬失败）：
/// `ls -R /` 一条就超，对长驻会话是误杀（设计 §5.4）。
#[derive(Debug)]
pub(super) struct Budget {
    cap: u64,
    used: u64,
    capped: bool,
}

impl Budget {
    pub(super) fn new(cap: u64) -> Self {
        Self {
            cap,
            used: 0,
            capped: false,
        }
    }

    /// `Forward` 累计计数，越界那一块整体丢弃并置封顶（会话保留、内存不增长）。
    pub(super) fn admit(&mut self, n: usize) -> Admit {
        if self.capped {
            return Admit::Drop;
        }
        let n = n as u64;
        if self.used.saturating_add(n) > self.cap {
            self.capped = true;
            return Admit::Cap;
        }
        self.used += n;
        Admit::Forward
    }

    /// 已转发的累计字节数（`closed.bytes_out` 用它）。
    pub(super) fn used(&self) -> u64 {
        self.used
    }
}

/// 增量 UTF-8 对齐器：设备的 4 KiB 块会切断多字节字符，不处理则每块边界都是 `�`。
///
/// 规则：保留尾部最多 3 个不完整字节与下一块拼接；**真非法字节**按
/// `String::from_utf8_lossy` 语义替换成 `U+FFFD` 后继续——不清缓冲、不断会话。
#[derive(Debug)]
pub(super) struct Utf8Carry {
    pending: [u8; 4],
    len: usize,
}

impl Utf8Carry {
    pub(super) fn new() -> Self {
        Self {
            pending: [0; 4],
            len: 0,
        }
    }

    /// 吃一块字节，返回其中**已是完整字符**的部分（可能为空串）。
    pub(super) fn push(&mut self, bytes: &[u8]) -> String {
        if bytes.is_empty() && self.len == 0 {
            return String::new();
        }
        let mut buf = Vec::with_capacity(self.len + bytes.len());
        buf.extend_from_slice(&self.pending[..self.len]);
        buf.extend_from_slice(bytes);
        self.len = 0;
        match std::str::from_utf8(&buf) {
            Ok(text) => text.to_string(),
            Err(err) => {
                let valid = err.valid_up_to();
                let mut head = String::from_utf8_lossy(&buf[..valid]).into_owned();
                match err.error_len() {
                    // 真非法字节：这一段按 lossy 替换，余下部分同语义继续。
                    Some(bad) => {
                        head.push_str(&String::from_utf8_lossy(&buf[valid..valid + bad]));
                        head.push_str(&String::from_utf8_lossy(&buf[valid + bad..]));
                        head
                    }
                    // 截断的多字节字符：整段留到下一块（最长 3 字节，4 字节序列已完整）。
                    None => {
                        let tail = &buf[valid..];
                        let keep = tail.len().min(4);
                        self.pending[..keep].copy_from_slice(&tail[..keep]);
                        self.len = keep;
                        head
                    }
                }
            }
        }
    }

    /// 收尾：残余不足一个完整字符，按 lossy 输出（会话已结束，不留在缓冲里）。
    pub(super) fn flush(&mut self) -> String {
        let text = String::from_utf8_lossy(&self.pending[..self.len]).into_owned();
        self.len = 0;
        text
    }
}
