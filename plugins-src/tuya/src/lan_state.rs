//! S3p：Tuya LAN 会话状态 kv 访问（编排 / 建链 / 心跳 / 重连共用）。
//!
//! 每设备一条 `tuya-lan-state:<device_id>`：连接上下文（conn_id / version /
//! 相位 / seqno / nonce / local_key / 会话 blob / 心跳与重连计数）。凭证材料
//! 只留插件隔离 kv，不下发宿主。事件间无内存态：全部经 kv 读写（纯函数模式）。

use serde_json::Value;
use xiaozhi_plugin_sdk::xiaozhi::plugin::host_core as host;

/// 会话编排状态键前缀（每设备一条连接上下文，含凭证材料，只留隔离 kv）。
pub(crate) const LAN_STATE_PREFIX: &str = "tuya-lan-state";

pub(crate) fn state_key(device_id: &str) -> String {
    format!("{LAN_STATE_PREFIX}:{device_id}")
}

pub(crate) fn state_read(device_id: &str) -> Result<Option<Value>, String> {
    match host::kv_get(&state_key(device_id)) {
        Some(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).map_err(|e| format!("状态 kv 非法：{e}"))?,
        )),
        None => Ok(None),
    }
}

pub(crate) fn state_write(device_id: &str, value: &Value) -> Result<(), String> {
    host::kv_put(&state_key(device_id), value.to_string().as_bytes())
        .map_err(|e| format!("状态 kv 写入失败：{e}"))
}

/// 全量状态扫描（kv-keys 前缀剥离；设备量为目录级，可接受）。
/// 返回 (device_id, state)；坏条目跳过（防单条脏数据阻断整体巡检）。
pub(crate) fn scan_states() -> Vec<(String, Value)> {
    let mut out = Vec::new();
    // 前缀带冒号：kv_keys 返回剥掉该前缀后的余量（即 device_id）。
    for key in host::kv_keys(&format!("{LAN_STATE_PREFIX}:")) {
        let Some(bytes) = host::kv_get(&format!("{LAN_STATE_PREFIX}:{key}")) else {
            continue;
        };
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        out.push((key, v));
    }
    out
}

/// 按 connection-id 反查设备状态。
pub(crate) fn find_state_by_conn(conn_id: u64) -> Result<Option<(String, Value)>, String> {
    for (key, v) in scan_states() {
        if v.get("conn_id").and_then(Value::as_u64) == Some(conn_id) {
            return Ok(Some((key, v)));
        }
    }
    Ok(None)
}
