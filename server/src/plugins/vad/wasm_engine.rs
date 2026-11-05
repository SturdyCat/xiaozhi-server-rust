//! `vad.provider` 的会话级 WASM 控制策略。
//!
//! WASM 只参与 native VAD 构造参数的策略准备；逐帧 `accept`、`flush` 和所有
//! PCM 数据仍由原生 `VadEngine` 执行，不跨 WASM 边界。

use std::sync::Arc;

use anyhow::{Context, Result};

use crate::plugins::vad::VadConfig;
use crate::plugins::wasm::{bridge, Resolution, WasmHost};

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
        let result = bridge::vad_prepare(
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
