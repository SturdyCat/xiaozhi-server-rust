//! 设备 MCP 工具注册表：管理界面「MCP 工具」页的数据源。
//!
//! ## 为什么需要一个注册表
//!
//! 工具清单只存在于固件里，且**只有设备连着的时候才问得到**（设备在 NAT 后、空闲时不建连）。
//! 所以"这台设备支持什么"这个信息必须**落下来**：会话开始时探测一次，结果按 `device_id`
//! 存住，管理界面随时可看（设备离线时显示上次结果 + 距今多久）。
//!
//! ## 设计取舍
//!
//! - **按 `device_id` 而不是 `session_id` 索引**：设备每次唤醒都是一条新连接（新 session），
//!   但"这台设备有什么工具"是设备属性。`device_id` 来自握手头 `Device-Id`（固件写 MAC，
//!   `websocket_protocol.cc:105`），因此是稳定键。⚠️ 客户端不带该头时落到 `"unknown"`，
//!   多台这样的设备会挤进同一条记录（界面上看到 `unknown` 就是这个情况）。
//! - **有上界**：只保留最近 [`MAX_DEVICES`] 台设备（与用量计量的 64 条同口径），
//!   长时间运行不会无界增长。
//! - **失败不丢上次的好数据**：探测失败时**保留**上次的工具清单与板型/版本，只更新
//!   `error` 与时间戳——"这台设备昨天还能用音量工具，今天探测失败了"比"什么都没有"更有用。
//! - **`online` 是会话生命周期派生的**：探测成功/失败都算在线（那一刻设备确实连着），
//!   会话结束由 [`McpRegistry::mark_offline`] 置否。**它不保证实时**（进程被强杀时不会翻转），
//!   故界面同时显示 `age_ms`，不把"在线"当作唯一真相。
//! - **只读投影**：本模块不改变任何运行期行为（与 `GET /api/plugins` 同契约）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use super::discovery::McpToolInfo;

/// 保留的设备数上限（超出后淘汰最旧的一条；按插入顺序，不追求精确 LRU）。
pub const MAX_DEVICES: usize = 64;

/// 当前 Unix 毫秒（注册表与界面共用同一时间基准，客户端无需自带时钟）。
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 一次探测的结果（会话侧填好交给注册表）。
#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub device_id: String,
    /// 发起探测的会话 id（[`McpRegistry::mark_offline`] 用它避免"旧会话把新会话标成离线"）。
    pub session_id: String,
    pub probed_at_ms: u64,
    /// 固件 `serverInfo.name`（板型名）。
    pub board: String,
    /// 固件 `serverInfo.version`（固件版本）。
    pub firmware_version: String,
    /// 设备声明的 MCP 协议版本。
    pub protocol_version: String,
    pub tools: Vec<McpToolInfo>,
    /// 探测未完成的原因（`None` = 完整成功）。部分成功（第 2 页超时）也会带原因。
    pub error: Option<String>,
}

/// 一台设备的工具快照。
#[derive(Debug, Clone)]
pub struct DeviceTools {
    pub device_id: String,
    pub session_id: String,
    pub probed_at_ms: u64,
    pub online: bool,
    pub board: String,
    pub firmware_version: String,
    pub protocol_version: String,
    pub tools: Vec<McpToolInfo>,
    /// 最近一次探测的问题（`None` = 成功）。
    pub error: Option<String>,
}

impl DeviceTools {
    /// 工具数 / 其中不暴露给 AI 的数量（`user_only`）。
    fn counts(&self) -> (usize, usize) {
        let user = self.tools.iter().filter(|t| t.user_only).count();
        (self.tools.len(), user)
    }

    /// 工具清单是否含某工具（纯内存遍历，零 I/O）。
    fn has_tool(&self, name: &str) -> bool {
        self.tools.iter().any(|t| t.name == name)
    }

    /// 指令闸门兼容性三态（`GET /api/mcp/tools` 的 `command_gate` 字段）：
    /// 清单在手且含 stop 工具 = 兼容；清单在手但不含 = 说「退下」将降级断开；
    /// 探测失败且零工具（清单未知）= 未知。
    fn command_gate(&self) -> &'static str {
        if self.error.is_some() && self.tools.is_empty() {
            "unknown"
        } else if self.has_tool(super::STOP_LISTENING_TOOL) {
            "compatible"
        } else {
            "will_degrade"
        }
    }

    /// `GET /api/mcp/tools` 的一行。
    ///
    /// `age_ms` 在**序列化时**算（而不是存起来）：注册表里的时间戳是事实，距今多久随请求变化。
    pub fn to_json(&self, now: u64) -> Value {
        let (total, user) = self.counts();
        json!({
            "device_id": self.device_id,
            "session_id": self.session_id,
            "online": self.online,
            "probed_at_ms": self.probed_at_ms,
            "age_ms": now.saturating_sub(self.probed_at_ms),
            "board": self.board,
            "firmware_version": self.firmware_version,
            "protocol_version": self.protocol_version,
            "tool_count": total,
            "user_tool_count": user,
            "ai_tool_count": total - user,
            "command_gate": self.command_gate(),
            "error": self.error,
            "tools": self.tools.iter().map(McpToolInfo::to_json).collect::<Vec<_>>(),
        })
    }
}

struct Inner {
    devices: HashMap<String, DeviceTools>,
    /// 插入顺序（用于淘汰最旧设备）。
    order: Vec<String>,
}

/// 进程级设备工具注册表（挂 [`crate::runtime::Engines`]，跨会话共享）。
#[derive(Clone)]
pub struct McpRegistry {
    inner: Arc<Mutex<Inner>>,
}

impl Default for McpRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl McpRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                devices: HashMap::new(),
                order: Vec::new(),
            })),
        }
    }

    /// 记一次探测结果。
    ///
    /// **失败保留旧数据**的判定在这里（只有一处，便于测试）：当本次**一个工具都没取到**
    /// 且带 `error` 时，视为"这次没问出来"——保留上次的工具清单与板型/版本，
    /// 只更新 `error`/时间戳/会话/在线状态。反之（取到了工具，哪怕第 2 页超时）按新结果整体替换。
    pub fn record(&self, probe: ProbeResult) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let keep_old = probe.tools.is_empty() && probe.error.is_some();
        let old = keep_old
            .then(|| inner.devices.get(&probe.device_id).cloned())
            .flatten();
        let entry = DeviceTools {
            device_id: probe.device_id.clone(),
            session_id: probe.session_id,
            probed_at_ms: probe.probed_at_ms,
            // 探测发生在一条活着的会话上，因此这一刻设备在线
            online: true,
            board: old
                .as_ref()
                .map_or(probe.board.clone(), |o| o.board.clone()),
            firmware_version: old.as_ref().map_or(probe.firmware_version.clone(), |o| {
                o.firmware_version.clone()
            }),
            protocol_version: old.as_ref().map_or(probe.protocol_version.clone(), |o| {
                o.protocol_version.clone()
            }),
            tools: old.map_or(probe.tools, |o| o.tools),
            error: probe.error,
        };
        if !inner.devices.contains_key(&probe.device_id) {
            inner.order.push(probe.device_id.clone());
        }
        inner.devices.insert(probe.device_id, entry);
        while inner.order.len() > MAX_DEVICES {
            let oldest = inner.order.remove(0);
            inner.devices.remove(&oldest);
        }
    }

    /// 指令闸门决策查询（P1-10，三态）：调用前先查该设备**已发现**的工具清单。
    ///
    /// - `None` = 清单**未知**（未登记，或最近一次探测失败且一个工具都没拿到）→
    ///   调用方保持现有"试探调用 + `-32602` 降级"路径，不劣化；
    /// - `Some(false)` = 清单在手且**明确没有**该工具 → 调用方跳过调用直接降级
    ///   （省一次注定失败的 2s 超时 / `-32602` 往返）；
    /// - `Some(true)` = 清单在手且含该工具 → 正常调用。
    ///
    /// 注意区分"清单为空但探测成功"（设备明确没有工具，`Some`）与
    /// "探测失败"（`error` 非空且零工具，`None`）——后者是"没问出来"，不是"没有"。
    pub fn has_tool(&self, device_id: &str, tool: &str) -> Option<bool> {
        let dt = self.get(device_id)?;
        if dt.error.is_some() && dt.tools.is_empty() {
            return None;
        }
        Some(dt.has_tool(tool))
    }

    /// 会话结束：把该设备标为离线。
    ///
    /// 只有 `session_id` 与注册表里一致才翻转——设备重连后新会话已探测过，
    /// 旧会话收尾不该把新会话的"在线"抹掉。
    pub fn mark_offline(&self, device_id: &str, session_id: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = inner.devices.get_mut(device_id) {
            if entry.session_id == session_id {
                entry.online = false;
            }
        }
    }

    /// 单台设备的快照（**测试与现场排障用**：端点只暴露 [`Self::to_json`] 投影，
    /// 但按 `device_id` 直取对写断言与登机排查更顺手——同 `UsageMeter::global` 的口径）。
    #[allow(dead_code)]
    pub fn get(&self, device_id: &str) -> Option<DeviceTools> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .devices
            .get(device_id)
            .cloned()
    }

    /// 已登记的设备数（测试用：验证上界淘汰）。
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .devices
            .len()
    }

    /// `GET /api/mcp/tools` 的响应体（最近探测的在前）。
    pub fn to_json(&self) -> Value {
        let now = now_ms();
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut devices: Vec<Value> = inner
            .order
            .iter()
            .filter_map(|id| inner.devices.get(id).map(|d| d.to_json(now)))
            .collect();
        // 最近的在前面（刚唤醒过的设备最可能被查看）
        devices.reverse();
        json!({
            "devices": devices,
            "count": devices.len(),
            "note": "设备工具清单由服务器在**会话开始时**通过 MCP `initialize` + `tools/list` 拉取：\
                     设备空闲时不建连，故刚部署本功能、或设备长期未唤醒时这里会是空的。\
                     `user_only=true` 的工具默认不出现在 `tools/list`、不会被大模型看到（服务器按名字直接调用）。",
        })
    }
}

#[cfg(test)]
mod tests;
