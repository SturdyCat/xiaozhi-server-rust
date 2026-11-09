//! `tts.provider` 的 WASM 控制策略桥（P4）：WASM 只改整句控制，PCM 仍由原生引擎产生。
//!
//! 策略事件：`tts.prepare`，payload 为受限 JSON
//! `{req_id,text,speed,speaker}`；插件经 kv 返回 `tts-result:<req_id>`，内容至少含
//! `{text}`，可选 `speed` / `speaker` / `route`。返回的文本与参数再交给包装的
//! 原生 `TtsEngine` 流式合成，避免把模型推理、f32 PCM 或音频 base64 越过 WASM 边界。
//!
//! 构建期不能绑定 `WasmHost`（宿主在 `Engines::assemble` 才创建），因此生产实例只持有
//! 扩展点名，在调用期解析全局宿主；测试可注入宿主避免全局槽污染。

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use crate::plugins::tts::{TtsChunkCallback, TtsEngine};
use crate::plugins::wasm::WasmHost;

const MAX_TTS_TEXT_BYTES: usize = 16 * 1024;
const MAX_TTS_ROUTE_BYTES: usize = 64;

/// WASM 策略返回的控制结果；它不携带任何 PCM 或音频 base64。
#[derive(Debug, Clone, Deserialize, PartialEq)]
struct TtsPrepareResult {
    text: String,
    #[serde(default)]
    speed: Option<f32>,
    #[serde(default)]
    speaker: Option<i32>,
    #[serde(default)]
    route: Option<String>,
}

/// 向 `tts.provider` 派发一次整句控制事件并读取策略结果。
///
/// 请求 payload 只包含文本与合成参数；结果至少包含 `text`，可选覆盖 `speed`、
/// `speaker`、`route`。宿主/插件缺失、事件错误、结果缺失或 JSON 非法均显式返回 Err，
/// 不回退到未经过策略的 native 文本。
fn tts_prepare(
    host: &WasmHost,
    plugin_id: &str,
    text: &str,
    speed: f32,
    speaker: i32,
) -> std::result::Result<TtsPrepareResult, String> {
    if text.len() > MAX_TTS_TEXT_BYTES {
        return Err(format!(
            "TTS 文本超长（{} > {MAX_TTS_TEXT_BYTES} bytes）",
            text.len()
        ));
    }
    if !speed.is_finite() || speed <= 0.0 {
        return Err(format!("TTS 请求 speed 非法：{speed}"));
    }
    let req_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({
        "req_id": req_id,
        "text": text,
        "speed": speed,
        "speaker": speaker,
    })
    .to_string();
    let errors = host
        .handle_event(plugin_id, "tts.prepare", &payload)
        .map_err(|e| format!("TTS 策略插件事件失败：{e:#}"))?;
    if !errors.is_empty() {
        return Err(format!("TTS 策略效果执行失败：{errors:?}"));
    }
    let bytes = host
        .kv()
        .get(plugin_id, &format!("tts-result:{req_id}"))
        .ok_or_else(|| "TTS 策略插件未返回结果（需要 kv 键 tts-result:<req_id>）".to_string())?;
    let result: TtsPrepareResult = serde_json::from_slice(&bytes)
        .map_err(|e| format!("TTS 策略结果损坏（需要 JSON {{text,...}}）：{e}"))?;
    if result.text.len() > MAX_TTS_TEXT_BYTES {
        return Err(format!(
            "TTS 策略文本超长（{} > {MAX_TTS_TEXT_BYTES} bytes）",
            result.text.len()
        ));
    }
    if let Some(route) = result.route.as_deref() {
        if route.len() > MAX_TTS_ROUTE_BYTES {
            return Err(format!("TTS 策略 route 超长（{} bytes）", route.len()));
        }
    }
    if let Some(result_speed) = result.speed {
        if !result_speed.is_finite() || result_speed <= 0.0 {
            return Err(format!("TTS 策略 speed 非法：{result_speed}"));
        }
    }
    Ok(result)
}

/// 由 WASM 策略决定的、仍交给原生 TTS 的控制参数。
#[derive(Debug, Clone, PartialEq)]
pub struct TtsPolicyDecision {
    pub text: String,
    pub speed: f32,
    pub speaker: i32,
    pub route: Option<String>,
}

/// 经 `tts.provider` 扩展点包装原生 TTS 的控制策略。
pub struct WasmTtsPolicy {
    native: Arc<dyn TtsEngine>,
    extension_point: &'static str,
    host_override: Option<Arc<crate::plugins::wasm::WasmHost>>,
}

impl WasmTtsPolicy {
    /// 生产构造：晚绑 `tts.provider` 宿主槽。
    pub fn new(native: Arc<dyn TtsEngine>) -> Self {
        Self {
            native,
            extension_point: "tts.provider",
            host_override: None,
        }
    }

    /// 测试/嵌入构造：固定宿主实例，不改动进程级全局槽。
    pub fn with_host(
        native: Arc<dyn TtsEngine>,
        host: Arc<crate::plugins::wasm::WasmHost>,
        extension_point: &'static str,
    ) -> Self {
        Self {
            native,
            extension_point,
            host_override: Some(host),
        }
    }

    fn host(&self) -> Result<Arc<crate::plugins::wasm::WasmHost>> {
        self.host_override
            .clone()
            .or_else(crate::plugins::wasm::host_global)
            .context("WASM 插件平台未初始化（宿主缺失；见服务启动日志）")
    }

    fn plugin_id(&self, host: &crate::plugins::wasm::WasmHost) -> Result<String> {
        match host.resolve(self.extension_point) {
            crate::plugins::wasm::Resolution::Resolved(id) => Ok(id),
            resolution => Err(anyhow::anyhow!(
                "TTS 策略插件未装载（扩展点 {} 无可用实现，resolution={resolution:?}）",
                self.extension_point
            )),
        }
    }
}

impl WasmTtsPolicy {
    pub(super) fn synthesize_decision(
        &self,
        decision: TtsPolicyDecision,
        chunk_cb: TtsChunkCallback,
    ) -> Result<()> {
        if let Some(route) = decision.route.as_deref() {
            if route != "native" {
                anyhow::bail!(
                    "TTS 策略返回不支持的 route={route:?}（当前仅允许 native 原生 PCM 路径）"
                );
            }
        }
        if !decision.speed.is_finite() || decision.speed <= 0.0 {
            anyhow::bail!("TTS 策略返回非法 speed={}", decision.speed);
        }
        self.native
            .synthesize_stream(&decision.text, decision.speed, decision.speaker, chunk_cb)
    }
}

impl TtsEngine for WasmTtsPolicy {
    fn synthesize_stream(
        &self,
        text: &str,
        speed: f32,
        speaker: i32,
        chunk_cb: TtsChunkCallback,
    ) -> Result<()> {
        let host = self.host()?;
        let plugin_id = self.plugin_id(&host)?;
        let raw = tts_prepare(&host, &plugin_id, text, speed, speaker)
            .map_err(|e| anyhow::anyhow!(e))
            .context("TTS 策略插件执行失败")?;
        let decision = TtsPolicyDecision {
            text: raw.text,
            speed: raw.speed.unwrap_or(speed),
            speaker: raw.speaker.unwrap_or(speaker),
            route: raw.route,
        };

        self.synthesize_decision(decision, chunk_cb)
    }

    fn output_sample_rate(&self) -> u32 {
        self.native.output_sample_rate()
    }

    fn name(&self) -> &'static str {
        "wasm-policy"
    }
}
