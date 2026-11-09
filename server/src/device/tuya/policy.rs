//! `connector.tuya` 的 WASM 控制策略（P5a）。
//!
//! 目录选择过 wasm：插件只返回候选索引与动作；local_key、IP、云凭证、原始 DP、
//! TCP/UDP 报文和 socket 句柄永不跨越 WASM 边界，LAN 数据面仍由 native
//! [`super::lan`] 执行。（阶段二收口：策略桥从 wasm 宿主 crate 归位到本模块——
//! `connector.tuya` 是设备业务扩展点名，宿主不持有。）

use serde::Deserialize;
use serde_json::{json, Value};

use crate::plugins::wasm::{Resolution, WasmHost};

const MAX_TUYA_NAME_BYTES: usize = 256;
const MAX_TUYA_DEVICES: usize = 128;

/// WASM Tuya 策略返回的候选索引；local_key、IP、原始 DP 和 socket 句柄永不跨边界。
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct TuyaPrepareResult {
    pub candidate_index: usize,
    pub action: String,
    #[serde(default)]
    pub route: Option<String>,
}

/// 向 `connector.tuya` 派发有限设备目录元数据，读取 native 控制决策。
/// wrapper 随后校验索引和 action，再由 `TuyaEngine` 执行 rustuya LAN 操作。
pub fn tuya_prepare(
    host: &WasmHost,
    name: &str,
    action: &str,
    candidates: &[Value],
) -> std::result::Result<TuyaPrepareResult, String> {
    if name.trim().is_empty() || name.len() > MAX_TUYA_NAME_BYTES {
        return Err("Tuya 请求设备名为空或超长".to_string());
    }
    if !matches!(action, "on" | "off") {
        return Err("Tuya 请求 action 只允许 on/off".to_string());
    }
    if candidates.len() > MAX_TUYA_DEVICES {
        return Err(format!(
            "Tuya 候选目录超限（{} > {MAX_TUYA_DEVICES}）",
            candidates.len()
        ));
    }
    let plugin_id = match host.resolve("connector.tuya") {
        Resolution::Resolved(id) => id,
        other => return Err(format!("Tuya 策略扩展点无可用实现：{other:?}")),
    };
    let req_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({
        "req_id": req_id,
        "name": name.trim(),
        "action": action,
        "candidates": candidates,
    })
    .to_string();
    let errors = host
        .handle_event(&plugin_id, "connector.tuya.prepare", &payload)
        .map_err(|e| format!("Tuya 策略插件事件失败：{e:#}"))?;
    if !errors.is_empty() {
        return Err(format!("Tuya 策略效果执行失败：{errors:?}"));
    }
    let bytes = host
        .kv()
        .get(&plugin_id, &format!("tuya-result:{req_id}"))
        .ok_or_else(|| "Tuya 策略插件未返回结果（需要 kv 键 tuya-result:<req_id>）".to_string())?;
    let result: TuyaPrepareResult = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Tuya 策略结果损坏（需要 candidate_index/action）：{e}"))?;
    if result.candidate_index >= candidates.len() {
        return Err(format!(
            "Tuya 策略 candidate_index 越界：{}（候选数 {}）",
            result.candidate_index,
            candidates.len()
        ));
    }
    if result.action != action || !matches!(result.action.as_str(), "on" | "off") {
        return Err("Tuya 策略返回的 action 非本轮请求的 on/off".to_string());
    }
    if let Some(route) = result.route.as_deref() {
        if route != "native" {
            return Err(format!("Tuya 策略返回不支持的 route={route:?}"));
        }
    }
    Ok(result)
}
