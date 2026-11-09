//! `device.echomuse` 的会话入口策略（P6a）。
//!
//! WASM 只决定 EchoMuse 会话欢迎文案；设备 WS、心跳、hub、PTY 和音频仍由 native
//! 承载。（阶段二收口：策略桥从 wasm 宿主 crate 归位到本模块——
//! `device.echomuse` 是设备业务扩展点名，宿主不持有。）

use serde::Deserialize;
use serde_json::json;

use crate::plugins::wasm::{Resolution, WasmHost};

const MAX_ECHOMUSE_WELCOME_BYTES: usize = 1024;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct EchomusePrepareResult {
    #[serde(default)]
    pub welcome: Option<String>,
    #[serde(default)]
    pub route: Option<String>,
}

/// 让 WASM 只决定 EchoMuse 会话欢迎文案；设备 WS、心跳、hub、PTY 和音频仍由 native 承载。
pub fn echomuse_prepare(
    host: &WasmHost,
    device_id: &str,
    base_os: &str,
    welcome: &str,
) -> std::result::Result<Option<String>, String> {
    if device_id.trim().is_empty() || device_id.len() > 128 {
        return Err("EchoMuse device_id 为空或超长".to_string());
    }
    if base_os.len() > 64 || welcome.len() > MAX_ECHOMUSE_WELCOME_BYTES {
        return Err("EchoMuse 会话元数据或欢迎语超长".to_string());
    }
    let plugin_id = match host.resolve("device.echomuse") {
        Resolution::Resolved(id) => id,
        other => return Err(format!("EchoMuse 策略扩展点无可用实现：{other:?}")),
    };
    let req_id = uuid::Uuid::new_v4().to_string();
    let payload =
        json!({"req_id": req_id, "device_id": device_id, "base_os": base_os, "welcome": welcome})
            .to_string();
    let errors = host
        .handle_event(&plugin_id, "device.echomuse.prepare", &payload)
        .map_err(|e| format!("EchoMuse 策略插件事件失败：{e:#}"))?;
    if !errors.is_empty() {
        return Err(format!("EchoMuse 策略效果执行失败：{errors:?}"));
    }
    let bytes = host
        .kv()
        .get(&plugin_id, &format!("echomuse-result:{req_id}"))
        .ok_or_else(|| "EchoMuse 策略插件未返回结果".to_string())?;
    let result: EchomusePrepareResult =
        serde_json::from_slice(&bytes).map_err(|e| format!("EchoMuse 策略结果损坏：{e}"))?;
    if let Some(route) = result.route.as_deref() {
        if route != "native" {
            return Err(format!("EchoMuse 策略返回不支持的 route={route:?}"));
        }
    }
    if let Some(text) = result.welcome.as_deref() {
        if text.len() > MAX_ECHOMUSE_WELCOME_BYTES {
            return Err("EchoMuse 策略返回欢迎语超长".to_string());
        }
        return Ok((!text.trim().is_empty()).then(|| text.trim().to_string()));
    }
    Ok(None)
}
