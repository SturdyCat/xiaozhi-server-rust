//! 令牌用量计量：让"成本"与"上下文膨胀"变得可见（`docs/plugin-architecture-unification.md` §4.3）。
//!
//! 两个问题此前无法回答：
//! 1. **成本**：跑了多少 token？（全局累计）
//! 2. **上下文膨胀**：记忆/灵魂/系统提示词注入之后，prompt 到底长了多少？
//!    （[`TokenUsage::pressure`] = 输入侧总量，**不含输出**——它才是"推给模型的上下文"）
//!
//! 设计取舍：
//! - **只记 token，不算钱**：单价随模型/渠道/时段变化，内置价目表必然过期
//!   （DSH 全包同样没有单价字段）。
//! - **按会话保留最近 N 条**：会话结束后仍可查（刚跑完就想看），但有上界，不会无界增长。
//! - 计量全程只加不解锁业务：锁内只做整数累加，不涉 IO。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::plugins::llm::TokenUsage;

/// 保留的会话数上限（超出后淘汰最旧的一条；按插入顺序，不追求精确 LRU）。
const MAX_SESSIONS: usize = 64;

#[derive(Debug, Clone, Copy, Default)]
pub struct UsageSnapshot {
    /// 已计量的轮次数（有 usage 的轮次）。
    pub turns: u64,
    pub usage: TokenUsage,
}

impl UsageSnapshot {
    pub fn to_json(&self, session_id: Option<&str>) -> serde_json::Value {
        let mut v = serde_json::json!({
            "turns": self.turns,
            "usage": self.usage.to_json(),
        });
        if let Some(id) = session_id {
            v["session_id"] = serde_json::json!(id);
        }
        v
    }
}

struct Inner {
    global: UsageSnapshot,
    sessions: HashMap<String, UsageSnapshot>,
    /// 插入顺序（用于淘汰最旧会话）。
    order: Vec<String>,
}

/// 进程级用量计量（挂 [`crate::engine::Engines`]，跨会话共享）。
#[derive(Clone)]
pub struct UsageMeter {
    inner: Arc<Mutex<Inner>>,
}

impl Default for UsageMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl UsageMeter {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                global: UsageSnapshot::default(),
                sessions: HashMap::new(),
                order: Vec::new(),
            })),
        }
    }

    /// 记一轮用量（`None` = 上游没给 usage，如老网关；此时只计轮次、不累加 token）。
    pub fn record(&self, session_id: &str, usage: Option<TokenUsage>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(u) = usage {
            inner.global.turns += 1;
            inner.global.usage.add(&u);
        }
        // 先登记顺序（不能放进 or_insert_with 闭包：闭包会与 sessions 的可变借用冲突）
        if !inner.sessions.contains_key(session_id) {
            inner.order.push(session_id.to_string());
        }
        let entry = inner.sessions.entry(session_id.to_string()).or_default();
        if let Some(u) = usage {
            entry.turns += 1;
            entry.usage.add(&u);
        }
        // 上界：淘汰最旧会话，避免长时间运行后 map 无界增长
        while inner.order.len() > MAX_SESSIONS {
            let oldest = inner.order.remove(0);
            inner.sessions.remove(&oldest);
        }
    }

    /// 全局累计快照（端点走 [`Self::to_json`]；此访问器供测试与后续 Prometheus 类导出用）。
    #[allow(dead_code)]
    pub fn global(&self) -> UsageSnapshot {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .global
    }

    pub fn session(&self, session_id: &str) -> Option<UsageSnapshot> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sessions
            .get(session_id)
            .copied()
    }

    /// `GET /api/usage` 的响应体（全局 + 各会话）。
    pub fn to_json(&self) -> serde_json::Value {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut sessions: Vec<serde_json::Value> = inner
            .order
            .iter()
            .filter_map(|id| inner.sessions.get(id).map(|s| s.to_json(Some(id))))
            .collect();
        // 最近的在前面（排查刚跑完的会话更顺手）
        sessions.reverse();
        serde_json::json!({
            "global": inner.global.to_json(None),
            "sessions": sessions,
            "note": "只统计令牌，不做金额换算（单价随模型/渠道变化，内置价目表必然过期）",
        })
    }
}

#[cfg(test)]
mod tests;
