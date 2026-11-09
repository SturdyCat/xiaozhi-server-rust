//! `vad.provider` 的会话级 WASM 控制策略。
//!
//! WASM 只参与 native VAD 构造参数的策略准备；逐帧 `accept`、`flush` 和所有
//! PCM 数据仍由原生 `VadEngine` 执行，不跨 WASM 边界。

use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;

use crate::plugins::vad::VadConfig;
use crate::plugins::wasm::{Resolution, WasmHost};

const MAX_VAD_ROUTE_BYTES: usize = 64;
const MAX_VAD_DURATION_SECONDS: f32 = 10.0;

/// WASM VAD 策略只返回 native 构造参数，不接收或返回 PCM/音频帧。
#[derive(Debug, Clone, Deserialize, PartialEq)]
struct VadPrepareResult {
    #[serde(default)]
    threshold: Option<f32>,
    #[serde(default)]
    min_silence_duration: Option<f32>,
    #[serde(default)]
    min_speech_duration: Option<f32>,
    #[serde(default)]
    route: Option<String>,
}

/// 向 `vad.provider` 派发一次会话级策略事件。
///
/// 事件只携带采样率和当前 VAD 控制参数；插件返回的覆盖值用于构造 native
/// `VadEngine`。逐帧 `accept`/`flush` 调用不会进入 WASM。
fn vad_prepare(
    host: &WasmHost,
    plugin_id: &str,
    sample_rate: u32,
    threshold: f32,
    min_silence_duration: f32,
    min_speech_duration: f32,
) -> std::result::Result<VadPrepareResult, String> {
    if sample_rate == 0 {
        return Err("VAD 请求 sample_rate 非法".to_string());
    }
    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        return Err(format!("VAD 请求 threshold 非法：{threshold}"));
    }
    if !min_silence_duration.is_finite()
        || !(0.0..=MAX_VAD_DURATION_SECONDS).contains(&min_silence_duration)
        || !min_speech_duration.is_finite()
        || !(0.0..=MAX_VAD_DURATION_SECONDS).contains(&min_speech_duration)
    {
        return Err("VAD 请求 duration 非法".to_string());
    }
    let req_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({
        "req_id": req_id,
        "sample_rate": sample_rate,
        "threshold": threshold,
        "min_silence_duration": min_silence_duration,
        "min_speech_duration": min_speech_duration,
    })
    .to_string();
    let errors = host
        .handle_event(plugin_id, "vad.prepare", &payload)
        .map_err(|e| format!("VAD 策略插件事件失败：{e:#}"))?;
    if !errors.is_empty() {
        return Err(format!("VAD 策略效果执行失败：{errors:?}"));
    }
    let bytes = host
        .kv()
        .get(plugin_id, &format!("vad-result:{req_id}"))
        .ok_or_else(|| "VAD 策略插件未返回结果（需要 kv 键 vad-result:<req_id>）".to_string())?;
    let result: VadPrepareResult = serde_json::from_slice(&bytes)
        .map_err(|e| format!("VAD 策略结果损坏（需要 JSON 控制字段）：{e}"))?;
    if let Some(value) = result.threshold {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(format!("VAD 策略 threshold 非法：{value}"));
        }
    }
    for (name, value) in [
        ("min_silence_duration", result.min_silence_duration),
        ("min_speech_duration", result.min_speech_duration),
    ] {
        if let Some(value) = value {
            if !value.is_finite() || !(0.0..=MAX_VAD_DURATION_SECONDS).contains(&value) {
                return Err(format!("VAD 策略 {name} 非法：{value}"));
            }
        }
    }
    if let Some(route) = result.route.as_deref() {
        if route.len() > MAX_VAD_ROUTE_BYTES || route != "native" {
            return Err(format!("VAD 策略返回不支持的 route={route:?}"));
        }
    }
    Ok(result)
}

/// VAD 策略适配器：只在每个 native VAD 会话构造前执行一次 `vad.prepare`。
pub struct WasmVadPolicy {
    host_override: Option<Arc<WasmHost>>,
    extension_point: &'static str,
}

impl WasmVadPolicy {
    pub fn for_extension(extension_point: &'static str) -> Self {
        Self {
            host_override: None,
            extension_point,
        }
    }

    /// 测试/嵌入构造，避免并行测试共享进程级宿主槽。
    pub fn with_host(host: Arc<WasmHost>, extension_point: &'static str) -> Self {
        Self {
            host_override: Some(host),
            extension_point,
        }
    }

    fn host(&self) -> Result<Arc<WasmHost>> {
        self.host_override
            .clone()
            .or_else(crate::plugins::wasm::host_global)
            .context("WASM 插件平台未初始化（宿主缺失；见服务启动日志）")
    }

    fn plugin_id(&self, host: &WasmHost) -> Result<String> {
        match host.resolve(self.extension_point) {
            Resolution::Resolved(id) => Ok(id),
            other => anyhow::bail!(
                "VAD 策略插件未装载（扩展点 {} 无可用实现：{other:?}）",
                self.extension_point
            ),
        }
    }

    pub fn prepare(&self, cfg: &VadConfig) -> Result<VadConfig> {
        self.prepare_with_identity(cfg)
            .map(|(effective, _)| effective)
    }

    pub fn prepare_with_identity(&self, cfg: &VadConfig) -> Result<(VadConfig, String)> {
        let host = self.host()?;
        let plugin_id = self.plugin_id(&host)?;
        let result = vad_prepare(
            &host,
            &plugin_id,
            16_000,
            cfg.threshold,
            cfg.min_silence_duration,
            cfg.min_speech_duration,
        )
        .map_err(|e| anyhow::anyhow!(e))
        .context("VAD 策略插件执行失败")?;

        let mut effective = cfg.clone();
        if let Some(value) = result.threshold {
            effective.threshold = value;
        }
        if let Some(value) = result.min_silence_duration {
            effective.min_silence_duration = value;
        }
        if let Some(value) = result.min_speech_duration {
            effective.min_speech_duration = value;
        }
        Ok((effective, plugin_id))
    }
}
