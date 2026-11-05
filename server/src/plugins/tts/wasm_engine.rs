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
use std::sync::Arc;

use crate::plugins::tts::{TtsChunkCallback, TtsEngine};
use crate::plugins::wasm::bridge;

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
        let raw = bridge::tts_prepare(&host, &plugin_id, text, speed, speaker)
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
