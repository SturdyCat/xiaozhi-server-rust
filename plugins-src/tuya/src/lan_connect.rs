//! S3m/S3o/S3p：Tuya LAN **建链与握手首帧**（首连与自动重连共用一条路径）。
//!
//! `lan-connect` 事件 → 解析载荷（device / ip / port / local_key / version /
//! heartbeat / 重连策略）→ [`start_session`]：宿主 `tcp-connect` 出站 → 发
//! SessKeyNegStart（v3.4 0x55AA / v3.5 0x6699+GCM）→ 状态 kv 置 `WaitResp`
//! （握手后续帧经 `io.tcp.*` 回灌，见 `lan_orchestrate`）。
//!
//! 连接失败**不直接判死**（S3p）：有重连策略 → 转 `Reconnecting` 并起退避
//! timeout（`lan_reconnect`），无策略 → `Failed`；两种情况都返回 Err 供结果
//! 通道（ok:false）观测。重连 timer 到期由 `lan_heartbeat` 分发到 [`restart`]，
//! 从状态 kv 恢复参数后走同一 `start_session`。

use serde_json::{json, Value};
use xiaozhi_plugin_sdk::xiaozhi::plugin::{host_core as host, host_net as net, types};

use crate::lan_events::{hex_encode, lan_result, payload_of, required_hex};
use crate::lan_heartbeat::HEARTBEAT_MIN_MS;
use crate::lan_reconnect;
use crate::lan_session;
use crate::lan_state::{state_read, state_write};

/// 会话编排入口（`connector.tuya.lan-connect`）：连接设备 TCP → 发
/// SessKeyNegStart → 状态置 WaitResp（回包经 io.tcp.data 步进）。
pub(crate) fn lan_connect(rid: &str, payload: &str) -> Result<Vec<types::Effect>, String> {
    // 编排失败（重复会话 / 连接失败）也落结果通道（ok:false）——泵路径对 guest
    // 业务性 Err 静默（call_guest_event Ok(None)），结果通道是唯一可观测面。
    lan_result(rid, lan_connect_inner(payload))
}

fn lan_connect_inner(payload: &str) -> Result<Value, String> {
    let v = payload_of(payload)?;
    let device_id = v
        .get("device_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("载荷缺少 device_id")?
        .to_string();
    let ip = v
        .get("ip")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("载荷缺少 ip")?;
    let port = v
        .get("port")
        .and_then(Value::as_u64)
        .ok_or("载荷缺少 port")? as u16;
    let local_key = required_hex(&v, "local_key_hex", 16)?;
    let version = v.get("version").and_then(Value::as_str).unwrap_or("3.4");
    if version != "3.4" && version != "3.5" {
        return Err(format!("编排暂只支持 v3.4 / v3.5（收到 {version}）"));
    }
    if let Some(state) = state_read(&device_id)? {
        let phase = state.get("state").and_then(Value::as_str).unwrap_or("");
        if phase != "Closed" {
            return Err(format!(
                "设备 {device_id} 已有会话（state={phase}），先等关闭或显式清理"
            ));
        }
    }

    let heartbeat_ms = v
        .get("heartbeat_ms")
        .and_then(Value::as_u64)
        .unwrap_or(30_000)
        .max(HEARTBEAT_MIN_MS);
    let policy = lan_reconnect::policy_from_payload(&v);
    start_session(
        &device_id,
        ip,
        port,
        &local_key,
        version,
        heartbeat_ms,
        policy,
    )
}

/// 建链：`tcp-connect` → 发 SessKeyNegStart → 状态 `WaitResp`。
/// 连接/首帧失败走 [`fail_with_retry`]（可重连则转 `Reconnecting`）。
pub(crate) fn start_session(
    device_id: &str,
    ip: &str,
    port: u16,
    local_key: &[u8],
    version: &str,
    heartbeat_ms: u64,
    policy: (u64, u64, u64),
) -> Result<Value, String> {
    let old = state_read(device_id)?;
    let attempt = old
        .as_ref()
        .and_then(|s| s.get("attempt").and_then(Value::as_u64))
        .unwrap_or(0);
    let reconnects = old
        .as_ref()
        .and_then(|s| s.get("reconnects").and_then(Value::as_u64))
        .unwrap_or(0);
    let mut state = policy_state(
        device_id,
        ip,
        port,
        local_key,
        version,
        heartbeat_ms,
        policy,
        attempt,
        reconnects,
    );
    // 保留上一轮的退避诊断（失败耗尽上限时仍能看到最后一次退避间隔）。
    for key in ["next_retry_ms", "next_retry_at_ms"] {
        if let Some(v) = old.as_ref().and_then(|s| s.get(key)) {
            state[key] = v.clone();
        }
    }
    let nonce = lan_session::random_nonce(local_key, device_id, host::now_ms());
    let conn_id = match net::tcp_connect(ip, port) {
        Ok(id) => id,
        Err(e) => {
            return fail_with_retry(device_id, state, format!("tcp-connect 失败：{e}"));
        }
    };
    state["conn_id"] = json!(conn_id);
    state["nonce_hex"] = json!(hex_encode(&nonce));
    // 第 1 帧：v3.5 的 IV 由 nonce（每连接唯一，含宿主时钟）派生，保证同一
    // local_key 下跨连接不重复；v3.4 无 IV。
    let start_frame = if version == "3.5" {
        let iv = lan_session::derive_iv(&nonce, 1, "start");
        lan_session::session_start_frame_35(1, local_key, &nonce, &iv)?
    } else {
        lan_session::session_start_frame(1, local_key, &nonce)?
    };
    if let Err(e) = net::net_send(conn_id, &start_frame) {
        return fail_with_retry(device_id, state, format!("SessKeyNegStart 发送失败：{e}"));
    }
    let hb_timer = crate::lan_heartbeat::ensure_timer(heartbeat_ms)?;
    state["state"] = json!("WaitResp");
    state["seqno"] = json!(2);
    state["last_hb_ms"] = json!(0);
    state["last_pong_ms"] = json!(0);
    state["data_frames"] = json!(0);
    state["hb_timer_id"] = json!(hb_timer);
    state_write(device_id, &state)?;
    Ok(json!({
        "ok": true,
        "device_id": device_id,
        "connection_id": conn_id,
        "state": "WaitResp",
        "attempt": attempt,
        "route": "plugin",
    }))
}

/// 连接上下文骨架（含凭证材料与重连策略，只落插件隔离 kv）。
#[allow(clippy::too_many_arguments)]
fn policy_state(
    device_id: &str,
    ip: &str,
    port: u16,
    local_key: &[u8],
    version: &str,
    heartbeat_ms: u64,
    policy: (u64, u64, u64),
    attempt: u64,
    reconnects: u64,
) -> Value {
    json!({
        "device_id": device_id,
        "ip": ip,
        "port": port,
        "local_key_hex": hex_encode(local_key),
        "version": version,
        "heartbeat_ms": heartbeat_ms,
        "rx_buf_hex": "",
        "state": "Failed",
        "reconnect_attempts": policy.0,
        "reconnect_base_ms": policy.1,
        "reconnect_max_ms": policy.2,
        "attempt": attempt,
        "reconnects": reconnects,
    })
}

/// 失败收敛：策略允许 → `Reconnecting` + 退避 timer；否则 `Failed`。
/// 无论哪种都先落 kv 再返回 Err（结果通道与状态 kv 双可观测面）。
fn fail_with_retry(device_id: &str, mut state: Value, msg: String) -> Result<Value, String> {
    state["error"] = json!(msg);
    let scheduled = lan_reconnect::schedule(&mut state, &msg)?;
    if scheduled {
        host::log(
            "info",
            &format!("设备 {device_id} 建链失败，进入退避重连：{msg}"),
        );
    }
    state_write(device_id, &state)?;
    Err(msg)
}

/// 重连 timer 到期：从状态 kv 恢复参数（凭证/策略/心跳间隔）重走建链。
pub(crate) fn restart(device_id: &str, state: &Value) -> Result<Value, String> {
    let ip = state
        .get("ip")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("重连状态缺少 ip")?;
    let port = state
        .get("port")
        .and_then(Value::as_u64)
        .ok_or("重连状态缺少 port")? as u16;
    let local_key = required_hex(state, "local_key_hex", 16)?;
    let version = state
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("3.4");
    let heartbeat_ms = state
        .get("heartbeat_ms")
        .and_then(Value::as_u64)
        .unwrap_or(30_000)
        .max(HEARTBEAT_MIN_MS);
    let policy = lan_reconnect::policy_of(state);
    start_session(
        device_id,
        ip,
        port,
        &local_key,
        version,
        heartbeat_ms,
        policy,
    )
}
