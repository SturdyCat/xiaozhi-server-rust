//! S3p：Tuya LAN **自动重连与指数退避**（guest 侧恢复策略）。
//!
//! 策略来源 = `lan-connect` 载荷（随连接上下文落状态 kv，重连时复用）：
//! `reconnect_attempts`（缺省 0 = 不自动重连，上限 [`MAX_ATTEMPTS`]）、
//! `reconnect_base_ms`（缺省 1s，下限 [`MIN_BASE_MS`]）、`reconnect_max_ms`
//! （缺省 30s，退避封顶）。
//!
//! 状态转移：`Established|WaitResp` 因关闭/握手失败 → `Reconnecting`
//! （attempt+1，延迟 = `base·2^(attempt-1)` 封顶 max，起一次性 timeout）→
//! timer 到期重走建链；attempt 超上限或未配置策略 → `Failed`。握手成功 →
//! [`mark_established`]：attempt 归零、`reconnects` 计数 +1。
//!
//! 退避 timer 经宿主 `host-timer`（每设备至多一个 timeout，远低于单插件 ≤64
//! 配额）；凭证/策略只留在插件隔离 kv。

use serde_json::{json, Value};
use xiaozhi_plugin_sdk::xiaozhi::plugin::{host_core as host, host_timer as timer};

pub(crate) const DEFAULT_BASE_MS: u64 = 1_000;
pub(crate) const DEFAULT_MAX_MS: u64 = 30_000;
/// 退避基数下限（防载荷给 0 造成忙等）。
pub(crate) const MIN_BASE_MS: u64 = 50;
/// 重连次数上限（防载荷给出无界重试）。
pub(crate) const MAX_ATTEMPTS: u64 = 10;
/// 退避指数封顶（2^6 = 64 倍，足够覆盖 30s 默认上限）。
const MAX_SHIFT: u64 = 6;

/// 从 `lan-connect` 载荷读取策略三元组 `(attempts, base_ms, max_ms)`，边界已收敛。
pub(crate) fn policy_from_payload(v: &Value) -> (u64, u64, u64) {
    let attempts = v
        .get("reconnect_attempts")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(MAX_ATTEMPTS);
    let base = v
        .get("reconnect_base_ms")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_BASE_MS)
        .max(MIN_BASE_MS);
    let max = v
        .get("reconnect_max_ms")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_MS)
        .max(base);
    (attempts, base, max)
}

/// 从连接上下文读回策略三元组（重连 timer 到期后复用）。
pub(crate) fn policy_of(state: &Value) -> (u64, u64, u64) {
    let attempts = state
        .get("reconnect_attempts")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(MAX_ATTEMPTS);
    let base = state
        .get("reconnect_base_ms")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_BASE_MS)
        .max(MIN_BASE_MS);
    let max = state
        .get("reconnect_max_ms")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_MS)
        .max(base);
    (attempts, base, max)
}

/// 退避延迟 = `base·2^(attempt-1)`，封顶 max（attempt 从 1 起）。
pub(crate) fn delay_for(base: u64, max: u64, attempt: u64) -> u64 {
    let shift = attempt.saturating_sub(1).min(MAX_SHIFT) as u32;
    base.saturating_mul(1u64 << shift).min(max)
}

fn clear_pending(state: &mut Value) {
    if let Some(id) = state.get("reconnect_timer_id").and_then(Value::as_u64) {
        let _ = timer::clear_timer(id);
    }
    if let Some(obj) = state.as_object_mut() {
        obj.remove("reconnect_timer_id");
    }
}

/// 失败收敛：可重连 → 置 `Reconnecting` 并起退避 timeout（返回 true）；无策略
/// 或已到上限 → 返回 false（调用方自行置 `Failed`/`Closed`）。
pub(crate) fn schedule(state: &mut Value, reason: &str) -> Result<bool, String> {
    let (max_attempts, base, max) = policy_of(state);
    if max_attempts == 0 {
        return Ok(false);
    }
    let attempt = state.get("attempt").and_then(Value::as_u64).unwrap_or(0);
    if attempt >= max_attempts {
        return Ok(false);
    }
    let next = attempt + 1;
    let delay = delay_for(base, max, next);
    clear_pending(state);
    let timer_id =
        timer::set_timeout(delay).map_err(|e| format!("重连退避 timer 创建失败：{e}"))?;
    state["state"] = json!("Reconnecting");
    state["attempt"] = json!(next);
    state["reconnect_timer_id"] = json!(timer_id);
    state["last_error"] = json!(reason);
    state["next_retry_ms"] = json!(delay);
    state["next_retry_at_ms"] = json!(host::now_ms() + delay);
    Ok(true)
}

/// 该状态是否正由 `timer_id` 驱动重连（timer 到期分发用）。
pub(crate) fn matches(state: &Value, timer_id: u64) -> bool {
    state.get("state").and_then(Value::as_str) == Some("Reconnecting")
        && state.get("reconnect_timer_id").and_then(Value::as_u64) == Some(timer_id)
}

/// 确认到期 timer：fired 后宿主已回收，仅清本地 id；attempt 计数保留（下次
/// 失败继续递增）。
pub(crate) fn acknowledge(state: &mut Value) {
    if let Some(obj) = state.as_object_mut() {
        obj.remove("reconnect_timer_id");
    }
    state["last_retry_ms"] = json!(host::now_ms());
}

/// 握手成功：清挂起 timer、attempt 归零；若本次是重连成功则 `reconnects` +1
/// （首连成功计数保持不变）。
pub(crate) fn mark_established(state: &mut Value) {
    let attempt = state.get("attempt").and_then(Value::as_u64).unwrap_or(0);
    clear_pending(state);
    if attempt > 0 {
        let n = state.get("reconnects").and_then(Value::as_u64).unwrap_or(0);
        state["reconnects"] = json!(n + 1);
    }
    if let Some(obj) = state.as_object_mut() {
        obj.remove("next_retry_ms");
        obj.remove("next_retry_at_ms");
    }
    state["attempt"] = json!(0);
}
