//! 设备 / 会话 / 最近对话的**进程内注册表**：管理页「概览 / 设备 / 会话 / 对话记录」的数据源。
//!
//! ## 为什么需要它
//!
//! 本模块落地前，服务端对"谁连过、现在谁在线、刚才聊了什么"**一无所知**：
//!
//! - 设备号只在握手时打一行日志就丢了（`app/ws/mod.rs` 的 `device_id`）；
//! - 全局唯一的会话结构是 [`crate::app::usage::UsageMeter`] 的 `sessions`——它按会话只记
//!   token，不含设备号、连接状态、协商参数（二进制协议版本 / 采样率 / 帧长）；
//! - 对话原文虽然会进 `gm_messages`，但**只在 `[memory].enabled = true` 时**才写，
//!   且没有任何查询端点。
//!
//! 参考实现 `xinnan-tech/xiaozhi-esp32-server` 把「设备」当一等公民（设备列表 / 在线状态 /
//! 最后活跃 / 会话与对话历史），这是本项目管理面最大的空洞。本模块补上其中**只读**的一半。
//!
//! ## 设计取舍
//!
//! - **全部内存态、不落盘**：与 [`crate::app::mcp::registry::McpRegistry`] 同口径。重启即清空
//!   （设备下次连上自动回填），换来零新配置项、零文件格式维护、零写盘风险。真要跨重启保留，
//!   把存储换成 SQLite 表即可——**对外 JSON 契约不用动**。
//! - **三个上界，全部有界**：设备 [`MAX_DEVICES`]、活跃会话 [`MAX_SESSIONS`]、保留对话的
//!   会话数 [`MAX_CONVERSATION_SESSIONS`] × 每会话轮次 [`MAX_TURNS_PER_SESSION`]。
//!   长时间运行不会无界增长——这是"进程内"方案唯一必须写对的事。
//! - **排序键是单调序号 `seq`，不是时间戳**：同一毫秒内连着来两条记录时，用时间戳排序的结果
//!   取决于 `HashMap` 的随机迭代顺序（实测：一轮快循环后"最近的在最前"会随机乱序）。
//!   `seq` 每次"碰到"记录就自增，因此列表顺序**确定**且与"最近活跃"一致；时间戳只用于展示。
//! - **`last_error` 的语义是"最近一次会话结束时的异常"**：会话**干净结束**时清空、
//!   异常结束时写入。因此"上次为什么断"在断电后仍可回答，而修好之后错误提示不会永远挂着。
//! - **`online` 由会话生命周期派生，不保证实时**：进程被 `kill -9` 时不会翻转，所以界面同时
//!   显示"最后活跃距今多久"，不把 `online` 当唯一真相（与 `McpRegistry` 同一妥协）。
//! - **只读投影**：本模块不参与任何语音流水线决策，只记录与上报。写路径全是"记一笔"，
//!   失败（例如锁中毒）也绝不向上传播错误。
//! - **[`SessionGuard`] 用 RAII 兜底**：会话开始返回守卫，`Drop` 时自动摘除。这样
//!   `run_session` 中途任何 `?` 提前返回、乃至 panic 展开，都不会把设备永久留在"在线"。
//! - **`unknown` 设备**：固件不带 `Device-Id` 头时落到字面量 `"unknown"`，多台此类设备会挤进
//!   同一条记录（与 `McpRegistry` 完全相同的行为，界面上看到 `unknown` 就是这个情况）。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// 重新导出时间基准：与 `McpRegistry` 共用同一个（跨模块时间戳可直接比较）。
pub use crate::app::mcp::registry::now_ms;

/// 设备表上界（超出后淘汰最旧的一条；按插入顺序，不追求精确 LRU）。
pub const MAX_DEVICES: usize = 64;
/// 活跃会话上界（正常远达不到；防的是"连接泄漏"这类异常）。
pub const MAX_SESSIONS: usize = 128;
/// 保留对话记录的**会话**数上界（最近 N 条会话）。
pub const MAX_CONVERSATION_SESSIONS: usize = 32;
/// 每个会话保留的**轮次条目**上界（user + assistant 各算一条）。
pub const MAX_TURNS_PER_SESSION: usize = 40;

mod types;
pub use types::*;

struct Inner {
    /// 注册表创建时刻（= 服务启动）。所有累计值的口径都是"自这次启动以来"。
    started_ms: u64,
    /// 单调序号：每次"碰到"一条记录就自增并写进该记录，列表按它倒序 = 最近活跃在前。
    /// 用序号而不是时间戳的理由见模块注释（同毫秒内时间戳无法定序）。
    next_seq: u64,
    devices: HashMap<String, DeviceRecord>,
    /// 设备插入顺序（用于淘汰最旧）。
    device_order: Vec<String>,
    sessions: HashMap<String, LiveSession>,
    /// 会话插入顺序（用于淘汰最旧）。
    session_order: Vec<String>,
    conversations: HashMap<String, ConversationRecord>,
    /// 对话记录键的插入顺序（用于淘汰最旧会话）。
    conversation_order: Vec<String>,
}

impl Inner {
    /// 取下一个序号（每次调用 +1）。
    fn bump(&mut self) -> u64 {
        self.next_seq += 1;
        self.next_seq
    }
}

/// 进程级注册表（挂 [`crate::engine::Engines`]，跨会话共享）。
///
/// `Clone` 是**廉价**的（内部 `Arc<Mutex<..>>`），因此可自由塞进守卫、发给 handler。
#[derive(Clone)]
pub struct Registry {
    inner: Arc<Mutex<Inner>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                started_ms: now_ms(),
                next_seq: 0,
                devices: HashMap::new(),
                device_order: Vec::new(),
                sessions: HashMap::new(),
                session_order: Vec::new(),
                conversations: HashMap::new(),
                conversation_order: Vec::new(),
            })),
        }
    }

    /// 锁中毒时取回数据而不是 panic：注册表是**只读观测面**，绝不该因为一次 panic
    /// 把整条语音链路带崩（与 `UsageMeter`/`McpRegistry` 同一口径）。
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 会话开始：登记设备（首次建档 / 更新最后活跃）与活会话，返回 RAII 守卫。
    ///
    /// 守卫 `Drop` 时自动 [`Registry::close_session`]，调用方不必写清理路径。
    pub fn begin_session(&self, start: SessionStart) -> SessionGuard {
        let now = now_ms();
        let mut guard = self.lock();
        // 先重借用成 `&mut Inner`：`MutexGuard` 的字段访问走 `DerefMut`（整租），
        // 直接写 `&mut inner.a, &mut inner.b` 会被判成"两次可变借用"。重借用后是普通
        // 结构体字段借用，两个**不同**字段可以同时可变借出（`evict` 需要）。
        let inner = &mut *guard;
        let seq = inner.bump();

        // ---- 设备：存在则更新，不存在则建档 ----
        let (first_seen_ms, session_count, turn_count, last_error) =
            match inner.devices.get(&start.device_id) {
                // `last_error` 语义 = "最近一次会话**结束**时的异常"，故这里**保留**它：
                // 新会话刚起来时，上一条会话为什么断仍然是有效信息；它会在本会话干净结束时清掉。
                Some(d) => (
                    d.first_seen_ms,
                    d.session_count + 1,
                    d.turn_count,
                    d.last_error.clone(),
                ),
                None => (now, 1, 0, None),
            };
        let device_seq = inner.bump();
        let device = DeviceRecord {
            device_id: start.device_id.clone(),
            client_id: start.client_id.clone(),
            first_seen_ms,
            last_seen_ms: now,
            online: true,
            session_count,
            turn_count,
            last_session_id: start.session_id.clone(),
            last_error,
            uplink_sr: start.uplink_sr,
            downlink_sr: start.downlink_sr,
            downlink_frame_ms: start.downlink_frame_ms,
            bin_version: start.bin_version,
            mcp: start.mcp,
            is_test: start.is_test,
            seq: device_seq,
        };
        if !inner.devices.contains_key(&start.device_id) {
            inner.device_order.push(start.device_id.clone());
        }
        inner.devices.insert(start.device_id.clone(), device);
        evict(&mut inner.device_order, &mut inner.devices, MAX_DEVICES);

        // ---- 活会话 ----
        let session = LiveSession {
            session_id: start.session_id.clone(),
            device_id: start.device_id.clone(),
            client_id: start.client_id,
            started_ms: now,
            last_activity_ms: now,
            phase: Phase::Idle,
            turn_count: 0,
            uplink_sr: start.uplink_sr,
            downlink_sr: start.downlink_sr,
            downlink_frame_ms: start.downlink_frame_ms,
            bin_version: start.bin_version,
            mcp: start.mcp,
            is_test: start.is_test,
            last_user_text: None,
            last_error: None,
            seq,
        };
        if inner
            .sessions
            .insert(start.session_id.clone(), session)
            .is_none()
        {
            inner.session_order.push(start.session_id.clone());
        }
        evict(&mut inner.session_order, &mut inner.sessions, MAX_SESSIONS);

        // 会话一开就有对话记录条目（空对话也要能被"最近会话"列出来）
        if !inner.conversations.contains_key(&start.session_id) {
            let conv_seq = inner.bump();
            inner.conversation_order.push(start.session_id.clone());
            inner.conversations.insert(
                start.session_id.clone(),
                ConversationRecord {
                    session_id: start.session_id.clone(),
                    device_id: start.device_id,
                    started_ms: now,
                    last_ms: now,
                    turns: Vec::new(),
                    dropped: 0,
                    seq: conv_seq,
                },
            );
        }
        evict(
            &mut inner.conversation_order,
            &mut inner.conversations,
            MAX_CONVERSATION_SESSIONS,
        );

        SessionGuard {
            registry: self.clone(),
            session_id: start.session_id,
        }
    }

    /// 会话结束：摘除活会话、收尾设备画像。
    ///
    /// 幂等——守卫与显式调用重复触发无副作用。**注意**：设备可能已重连并开始新会话，
    /// 因此只有"设备记录的 `last_session_id` 还是我"时才把 `online` 置否，
    /// 否则新连接的在线状态会被旧会话的收尾抹掉（与 `McpRegistry::mark_offline` 同一判据）。
    ///
    /// `last_error` 的收尾规则：本会话有异常 → 写到设备上；本会话干净 → **清空**。
    /// 两者都只在"我还是设备的最新会话"时生效（原因同上）。
    pub fn close_session(&self, session_id: &str) {
        let mut guard = self.lock();
        let inner = &mut *guard;
        if !inner.sessions.contains_key(session_id) {
            return;
        }
        let seq = inner.bump();
        // contains_key 刚查过，这里不可能为 None
        let Some(session) = inner.sessions.remove(session_id) else {
            return;
        };
        inner.session_order.retain(|id| id != session_id);
        let Some(device) = inner.devices.get_mut(&session.device_id) else {
            return;
        };
        if device.last_session_id == session_id {
            device.online = false;
            device.last_error = session.last_error.clone();
        }
        device.last_seen_ms = device.last_seen_ms.max(session.last_activity_ms);
        device.seq = seq;
    }

    /// 记一条**用户轮次**：识别出文本、即将交给大模型。
    ///
    /// 同时更新会话（阶段=思考中、轮次+1、最近原话）与设备（累计轮次、最后活跃）。
    pub fn record_user_turn(&self, session_id: &str, text: &str) {
        let now = now_ms();
        let mut guard = self.lock();
        let inner = &mut *guard;
        let device_id = match inner.sessions.get_mut(session_id) {
            Some(s) => {
                s.phase = Phase::Thinking;
                s.turn_count += 1;
                s.last_activity_ms = now;
                s.last_user_text = Some(text.to_string());
                s.device_id.clone()
            }
            // 会话未登记（不该发生）：仍写对话记录，避免丢轮次
            None => device_id_of(inner, session_id),
        };
        let seq = inner.bump();
        if let Some(s) = inner.sessions.get_mut(session_id) {
            s.seq = seq;
        }
        if let Some(d) = inner.devices.get_mut(&device_id) {
            d.turn_count += 1;
            d.last_seen_ms = now;
            d.seq = seq;
        }
        push_turn(
            inner,
            session_id,
            &device_id,
            TurnRecord {
                role: "user",
                text: text.to_string(),
                at_ms: now,
                elapsed_ms: None,
            },
        );
    }

    /// 记一条**助手轮次**：整轮回复已下发完成（或被兜底话术替代）。
    ///
    /// `elapsed_ms` = 整轮耗时（LLM + 合成 + 下行），界面上用来判断"这轮慢不慢"。
    pub fn record_assistant_turn(&self, session_id: &str, text: &str, elapsed_ms: u64) {
        let now = now_ms();
        let mut guard = self.lock();
        let inner = &mut *guard;
        let device_id = match inner.sessions.get_mut(session_id) {
            Some(s) => {
                s.phase = Phase::Idle;
                s.last_activity_ms = now;
                s.device_id.clone()
            }
            None => device_id_of(inner, session_id),
        };
        let seq = inner.bump();
        if let Some(s) = inner.sessions.get_mut(session_id) {
            s.seq = seq;
        }
        if let Some(d) = inner.devices.get_mut(&device_id) {
            d.last_seen_ms = now;
            d.seq = seq;
        }
        push_turn(
            inner,
            session_id,
            &device_id,
            TurnRecord {
                role: "assistant",
                text: text.to_string(),
                at_ms: now,
                elapsed_ms: Some(elapsed_ms),
            },
        );
    }

    /// 更新会话阶段（`listen state=start` → 聆听中；下行开始 → 说话中）。
    ///
    /// 会话不存在时静默忽略（不该因为一条迟到的观测把请求打挂）。
    pub fn set_phase(&self, session_id: &str, phase: Phase) {
        let mut inner = self.lock();
        let seq = inner.bump();
        if let Some(s) = inner.sessions.get_mut(session_id) {
            s.phase = phase;
            s.last_activity_ms = now_ms();
            s.seq = seq;
        }
    }

    /// 记一条会话级异常（会话结束时会带到设备画像上）。
    pub fn record_error(&self, session_id: &str, error: impl Into<String>) {
        let error = error.into();
        let mut inner = self.lock();
        let seq = inner.bump();
        if let Some(s) = inner.sessions.get_mut(session_id) {
            s.last_error = Some(error);
            s.last_activity_ms = now_ms();
            s.seq = seq;
        }
    }

    // ---------------------------------------------------------------
    // 读：全部返回**克隆快照**，锁只在函数内持有（handler 侧拼 JSON 时不占锁）
    // ---------------------------------------------------------------

    /// 自**本注册表创建**（= 服务启动）以来的毫秒数。
    ///
    /// 存在理由：注册表是内存态，所有累计值（设备数 / 轮次数 / 保留对话数）的口径都是
    /// "自这次启动以来"。界面上必须把这句话说清楚，否则用户会把它误读成"今日统计"。
    pub fn uptime_ms(&self) -> u64 {
        now_ms().saturating_sub(self.lock().started_ms)
    }

    /// 计数快照（`GET /api/stats`）。
    pub fn counts(&self) -> Counts {
        let inner = self.lock();
        let online: HashSet<&str> = inner
            .sessions
            .values()
            .map(|s| s.device_id.as_str())
            .collect();
        Counts {
            devices: inner.devices.len(),
            online: online.len(),
            test_sessions: inner.sessions.values().filter(|s| s.is_test).count(),
            sessions: inner.sessions.len(),
            conversations: inner.conversations.len(),
        }
    }

    /// 全部设备（**最近活跃在前**）。
    pub fn devices(&self) -> Vec<DeviceRecord> {
        let inner = self.lock();
        let mut out: Vec<DeviceRecord> = inner.devices.values().cloned().collect();
        out.sort_by(|a, b| b.seq.cmp(&a.seq));
        out
    }

    /// 一台设备（测试与按 id 直取用）。
    #[allow(dead_code)]
    pub fn device(&self, device_id: &str) -> Option<DeviceRecord> {
        self.lock().devices.get(device_id).cloned()
    }

    /// 全部活会话（**最近活跃在前**）。
    pub fn sessions(&self) -> Vec<LiveSession> {
        let inner = self.lock();
        let mut out: Vec<LiveSession> = inner.sessions.values().cloned().collect();
        out.sort_by(|a, b| b.seq.cmp(&a.seq));
        out
    }

    /// 保留的对话记录（**最近活跃在前**）。
    pub fn conversations(&self) -> Vec<ConversationRecord> {
        let inner = self.lock();
        let mut out: Vec<ConversationRecord> = inner.conversations.values().cloned().collect();
        out.sort_by(|a, b| b.seq.cmp(&a.seq));
        out
    }

    /// 某设备名下的活会话（`GET /api/devices/{id}` 的"当前连接"段）。
    pub fn sessions_of(&self, device_id: &str) -> Vec<LiveSession> {
        let inner = self.lock();
        let mut out: Vec<LiveSession> = inner
            .sessions
            .values()
            .filter(|s| s.device_id == device_id)
            .cloned()
            .collect();
        out.sort_by(|a, b| b.seq.cmp(&a.seq));
        out
    }

    /// 某设备名下的对话记录（最近的在前，最多 `limit` 条会话）。
    pub fn conversations_of(&self, device_id: &str, limit: usize) -> Vec<ConversationRecord> {
        let inner = self.lock();
        let mut out: Vec<ConversationRecord> = inner
            .conversations
            .values()
            .filter(|c| c.device_id == device_id)
            .cloned()
            .collect();
        out.sort_by(|a, b| b.seq.cmp(&a.seq));
        out.truncate(limit);
        out
    }

    /// 已登记设备数（测试用：验证上界淘汰）。
    #[allow(dead_code)]
    pub fn device_count(&self) -> usize {
        self.lock().devices.len()
    }

    /// 活会话数（测试用）。
    #[allow(dead_code)]
    pub fn session_count(&self) -> usize {
        self.lock().sessions.len()
    }
}

/// 取会话所属设备；会话不在表里时退回对话记录里的设备号（可能为空串）。
fn device_id_of(inner: &Inner, session_id: &str) -> String {
    inner
        .conversations
        .get(session_id)
        .map(|c| c.device_id.clone())
        .unwrap_or_default()
}

/// 会话守卫：`Drop` 即摘除活会话。持有 [`Registry`] 的廉价克隆。
///
/// 存在的理由只有一条——**会话可能在 `run_session` 的任意一行提前返回**（`?` 出错、
/// panic 展开、传输断开）。若把清理写成"末尾一行代码"，那些路径全会泄漏一条"永远在线"的
/// 假会话，而这恰恰是"设备管理"页面最不能忍的错误。
pub struct SessionGuard {
    registry: Registry,
    session_id: String,
}

impl SessionGuard {
    /// 会话 id（调用方需要它注入日志时用；避免再传一份）。
    #[allow(dead_code)]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.registry.close_session(&self.session_id);
    }
}

/// 把一条轮次追加进对话记录；会话条目不存在时**补齐**（测试直接调用 `record_*` 时会走到）。
fn push_turn(inner: &mut Inner, session_id: &str, device_id: &str, turn: TurnRecord) {
    let now = turn.at_ms;
    if !inner.conversations.contains_key(session_id) {
        inner.conversation_order.push(session_id.to_string());
        inner.conversations.insert(
            session_id.to_string(),
            ConversationRecord {
                session_id: session_id.to_string(),
                device_id: device_id.to_string(),
                started_ms: now,
                last_ms: now,
                turns: Vec::new(),
                dropped: 0,
                seq: 0,
            },
        );
        evict(
            &mut inner.conversation_order,
            &mut inner.conversations,
            MAX_CONVERSATION_SESSIONS,
        );
    }
    let seq = inner.bump();
    let Some(conv) = inner.conversations.get_mut(session_id) else {
        return;
    };
    conv.last_ms = now;
    conv.seq = seq;
    conv.turns.push(turn);
    // 环形：只保留**最近**的 N 条（丢最旧，并记下丢了多少条以便界面如实说明）
    while conv.turns.len() > MAX_TURNS_PER_SESSION {
        conv.turns.remove(0);
        conv.dropped += 1;
    }
}

/// 按插入顺序淘汰最旧条目，直到不超过 `max`。
fn evict<V>(order: &mut Vec<String>, map: &mut HashMap<String, V>, max: usize) {
    while order.len() > max {
        let oldest = order.remove(0);
        map.remove(&oldest);
    }
}

#[cfg(test)]
mod tests;
