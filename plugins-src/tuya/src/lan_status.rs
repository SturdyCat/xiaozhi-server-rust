//! S3q：Tuya LAN **设备主动上报（DP 状态）接线**——DP 缓存 / 在线语义 / 事件面。
//!
//! 设备侧语义（对齐 native `server/src/device/tuya/lan.rs::LanSink::on_message`）：
//! 收到带 DP 的报文 → 更新设备 DP 快照 + 置在线 → 管理面/业务可读。DP 载荷
//! 解析与 native **同源**（`#[path]` 共引 `server/src/device/tuya/dps.rs`）。
//!
//! 与 native 的两处**语义修正**（native 实现缺陷，见设计文档 S3q）：
//! 1. **命令过滤**：native 把「任意 JSON 载荷」都套 `parse_dps`，心跳回显
//!    `{"devId","gwId"}` 会被当成 DP 写进缓存；插件只接受设备上报类命令
//!    （`DP_REPORT_CMDS`：Status 0x08 / DpQuery 0x0a / DpQueryNew 0x10 /
//!    UpdateDps 0x12）。
//! 2. **增量合并**：native 用整表替换（后到的部分上报会丢先前 DP）；插件按
//!    DP id 合并（设备可能只报变化的 DP），上限 [`MAX_DPS`] 防放大。
//!
//! 事件面：`tuya.official.lan-status`（插件命名空间）发布当前 DP 快照，宿主侧
//! 订阅者（会话/管理面/未来 native 接管方）按需消费；发布失败只告警不断链。

use serde_json::{json, Value};
use xiaozhi_plugin_sdk::xiaozhi::plugin::host_core as host;

use crate::dps::parse_dps;

/// 设备上报类命令（只有这些命令的 JSON 载荷按 DP 状态处理）。
const DP_REPORT_CMDS: [u32; 4] = [0x08, 0x0a, 0x10, 0x12];
/// DP 缓存键上限（防异常载荷放大；正常设备数十个 DP）。
const MAX_DPS: usize = 128;
/// 单条上报明文上限（解码后；正常 DP 上报远小于此）。
const MAX_REPORT_BYTES: usize = 8 * 1024;
/// 状态事件 topic（必须落在插件自身命名空间）。
const STATUS_TOPIC: &str = "tuya.official.lan-status";

/// 尝试把一帧已解密明文按「设备上报」接入状态与事件面。
///
/// 返回本次生效的 DP 数：`None` = 非上报命令 / 载荷不可解析 / 空 DP（不算
/// 状态）；`Some(n)` = 已合并 n 个 DP 并已置在线。
pub(crate) fn apply(device_id: &str, state: &mut Value, cmd: u32, plain: &[u8]) -> Option<usize> {
    if !DP_REPORT_CMDS.contains(&cmd) {
        return None;
    }
    if plain.len() > MAX_REPORT_BYTES {
        host::log(
            "warn",
            &format!(
                "设备 {device_id} 上报载荷过大（{}B > {MAX_REPORT_BYTES}），已忽略",
                plain.len()
            ),
        );
        return None;
    }
    let text = std::str::from_utf8(plain).ok()?;
    let incoming = parse_dps(text);
    if incoming.is_empty() {
        return None;
    }
    let count = incoming.len();
    let mut merged = state
        .get("dps")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (dp, value) in incoming {
        if merged.len() >= MAX_DPS && !merged.contains_key(&dp) {
            continue; // 已满：只允许覆盖已知 DP，防新增键放大
        }
        merged.insert(dp, value);
    }
    let now = host::now_ms();
    state["dps"] = Value::Object(merged);
    state["online"] = json!(true);
    state["last_seen_ms"] = json!(now);
    state["last_status_ms"] = json!(now);
    let seen = state
        .get("status_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    state["status_count"] = json!(seen + 1);
    state["last_status_dp_count"] = json!(count);
    state["last_writer"] = json!("status");
    publish_status(device_id, state, cmd, count, now);
    Some(count)
}

/// 事件面：发布当前 DP 快照（失败不断链——权限/背压只告警）。
fn publish_status(device_id: &str, state: &Value, cmd: u32, count: usize, now: u64) {
    let payload = json!({
        "device_id": device_id,
        "cmd": cmd,
        "count": count,
        "dps": state.get("dps").cloned().unwrap_or(Value::Null),
        "online": true,
        "at_ms": now,
    });
    let bytes = payload.to_string().into_bytes();
    if bytes.len() > MAX_REPORT_BYTES {
        host::log(
            "warn",
            &format!(
                "设备 {device_id} 状态事件过大（{}B），未发布（DP 已落状态 kv）",
                bytes.len()
            ),
        );
        return;
    }
    match host::publish(STATUS_TOPIC, &bytes) {
        Ok(true) => {}
        Ok(false) => host::log(
            "debug",
            &format!("设备 {device_id} 状态事件有订阅者队列满（背压），已跳过"),
        ),
        Err(e) => host::log("warn", &format!("设备 {device_id} 状态事件发布失败：{e}")),
    }
}
