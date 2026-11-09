//! S3n/S3p：Tuya LAN **心跳节拍 + timer 分发**（单全局 interval 心跳 /
//! 每设备一次性重连退避）。
//!
//! 心跳：`lan-connect` 首个连接创建单全局 interval timer（省宿主配额），
//! `host.timer.fired` 时校验 id（防重建后陈旧 timer）→ 遍历 `Established`
//! 设备，距上次心跳 ≥ 间隔才发（多设备共享节拍 / timer 抖动去重）→ 帧形状
//! v3.4 0x55AA / v3.5 0x6699，密钥 = 会话 blob。
//!
//! 重连：非心跳 timer 的 fired 事件按设备状态里的 `reconnect_timer_id` 路由，
//! 到期后交 [`crate::lan_connect::restart`] 从状态 kv 恢复参数重走建链。

use serde_json::{json, Value};
use xiaozhi_plugin_sdk::xiaozhi::plugin::{
    host_core as host, host_net as net, host_timer as timer, types,
};

use crate::lan_codec;
use crate::lan_connect;
use crate::lan_events::required_hex;
use crate::lan_frame_6699;
use crate::lan_reconnect;
use crate::lan_session;
use crate::lan_state::{scan_states, state_write};

/// 心跳节拍全局 timer 的 kv 键。
pub(crate) const HB_TIMER_KEY: &str = "tuya-lan-hbtimer";
/// 心跳间隔下限（防滥用；生产建议 ≥10s，测试可用 200ms）。
pub(crate) const HEARTBEAT_MIN_MS: u64 = 200;

/// 确保单全局心跳 interval timer 存在（首个连接创建；fired 时校验 id 防陈旧）。
pub(crate) fn ensure_timer(heartbeat_ms: u64) -> Result<u64, String> {
    let existing = host::kv_get(HB_TIMER_KEY)
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|v| v.get("timer_id").and_then(Value::as_u64));
    if let Some(id) = existing {
        return Ok(id);
    }
    let id = timer::set_interval(heartbeat_ms).map_err(|e| format!("set-interval 失败：{e}"))?;
    host::kv_put(
        HB_TIMER_KEY,
        json!({"timer_id": id, "interval_ms": heartbeat_ms})
            .to_string()
            .as_bytes(),
    )
    .map_err(|e| format!("timer kv 写入失败：{e}"))?;
    Ok(id)
}

/// `host.timer.fired` 分发：心跳节拍 / 重连退避 timer。
pub(crate) fn on_timer_fired(payload: &str) -> Result<Vec<types::Effect>, String> {
    let fired: Value = serde_json::from_str(payload).map_err(|e| format!("timer 载荷非法：{e}"))?;
    let Some(fired_id) = fired.get("timer_id").and_then(Value::as_u64) else {
        return Ok(vec![]);
    };
    let hb_id = host::kv_get(HB_TIMER_KEY)
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|v| v.get("timer_id").and_then(Value::as_u64));
    if hb_id == Some(fired_id) {
        return heartbeat_round();
    }
    // S3p：重连退避 timer（每设备一个一次性 timeout）。到期后按状态 kv 里的
    // 凭证重新建链；`restart` 内部失败会自行安排下一次退避。
    if let Some((device_id, state)) = take_reconnect_due(fired_id)? {
        if let Err(e) = lan_connect::restart(&device_id, &state) {
            host::log("warn", &format!("设备 {device_id} 重连失败：{e}"));
        }
    }
    Ok(vec![])
}

/// 找到并确认由 `timer_id` 驱动的重连状态（写回清 timer id）。
fn take_reconnect_due(timer_id: u64) -> Result<Option<(String, Value)>, String> {
    for (device_id, mut state) in scan_states() {
        if lan_reconnect::matches(&state, timer_id) {
            lan_reconnect::acknowledge(&mut state);
            state["last_writer"] = json!("reconnect-timer");
            state_write(&device_id, &state)?;
            return Ok(Some((device_id, state)));
        }
    }
    Ok(None)
}

/// 心跳一轮：遍历 `Established` 设备，距上次心跳 ≥ 间隔才发（防 timer 抖动）。
fn heartbeat_round() -> Result<Vec<types::Effect>, String> {
    let now = host::now_ms();
    for (device_id, mut state) in scan_states() {
        if state.get("state").and_then(Value::as_str) != Some("Established") {
            continue;
        }
        let interval = state
            .get("heartbeat_ms")
            .and_then(Value::as_u64)
            .unwrap_or(30_000);
        let last = state.get("last_hb_ms").and_then(Value::as_u64).unwrap_or(0);
        if now.saturating_sub(last) < interval {
            continue; // timer 抖动 / 多设备共享节拍
        }
        let step = (|| -> Result<(), String> {
            let session_key = required_hex(&state, "session_key_hex", 16)?;
            let seqno = state.get("seqno").and_then(Value::as_u64).unwrap_or(3) as u32;
            let conn_id = state.get("conn_id").and_then(Value::as_u64).unwrap_or(0) as u64;
            let version = state
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or("3.4");
            let (_cmd, frame) = if version == "3.5" {
                // v3.5：0x6699/GCM，IV 由 blob+seqno 派生（同连接内唯一）。
                let iv = lan_session::derive_iv(&session_key, seqno, "hb");
                lan_frame_6699::command_frame_35(
                    seqno,
                    &device_id,
                    None,
                    0,
                    "heartbeat",
                    Value::Null,
                    &session_key,
                    &iv,
                )?
            } else {
                lan_codec::command_frame_34(
                    seqno,
                    &device_id,
                    None,
                    0,
                    "heartbeat",
                    Value::Null,
                    &session_key,
                )?
            };
            net::net_send(conn_id, &frame).map_err(|e| format!("心跳发送失败：{e}"))?;
            state["last_hb_ms"] = json!(now);
            state["seqno"] = json!(seqno + 1);
            state["last_writer"] = json!("heartbeat");
            state_write(&device_id, &state)
        })();
        if let Err(e) = step {
            host::log("warn", &format!("设备 {device_id} 心跳失败：{e}"));
        }
    }
    Ok(vec![])
}
