//! `asr.provider` 扩展点的 **wasm 远端引擎适配器**（P4b，feature 门控）。
//!
//! 实现 [`AsrEngine`] trait：utterance 级事件桥（`asr.transcribe` → kv 结果通道），
//! 让用户用自己的远端引擎（Whisper 服务器等，经插件 http-get 效果）替换本地
//! SenseVoice。**推理留原生铁律不变**：本地 SenseVoice 路径零改动，本适配器只是
//! `[asr].engine = "wasm-remote"` 时装配的等价引擎实现。
//!
//! ## 晚绑宿主槽
//!
//! 构建发生在 `PluginHost::boot`（此时 wasm 宿主尚未创建——宿主在 `assemble`
//! 里建），因此适配器**只持有扩展点名**；`recognize` 调用期经
//! [`crate::plugins::wasm::host_global`] 解析宿主，缺失 → 显式 Err（不静默回退本地）。
//! 调用纪律：`recognize` 为同步 trait，管线在 spawn_blocking 内调用——与原生
//! 引擎推理同一纪律。

use anyhow::{Context, Result};

use crate::plugins::asr::AsrEngine;
use crate::plugins::wasm::WasmHost;

/// utterance 转写：`asr.transcribe` 事件（payload JSON 含 f32-le PCM 的 base64），
/// 插件经 kv 结果通道 `asr-result:<req_id>` 返回 `{"text":"..."}`。
/// 调用纪律：`recognize` 本就是同步 trait 且管线在 spawn_blocking 内调用——与原生
/// reqwest/引擎推理同一纪律。错误 = 显式 Err（会话侧按 on_error 语义处理）。
fn asr_transcribe(
    host: &WasmHost,
    plugin_id: &str,
    session_id: &str,
    samples: &[f32],
    sample_rate: u32,
) -> std::result::Result<String, String> {
    use base64::Engine as _;

    let req_id = uuid::Uuid::new_v4().to_string();
    let mut pcm = Vec::with_capacity(samples.len() * 4);
    for s in samples {
        pcm.extend_from_slice(&s.to_le_bytes());
    }
    let payload = serde_json::json!({
        "req_id": req_id,
        "session_id": session_id,
        "sample_rate": sample_rate,
        "samples": samples.len(),
        "pcm_base64": base64::engine::general_purpose::STANDARD.encode(&pcm),
    })
    .to_string();
    host.handle_event(plugin_id, "asr.transcribe", &payload)
        .map_err(|e| format!("ASR 插件事件失败：{e:#}"))?;
    let bytes = host
        .kv()
        .get(plugin_id, &format!("asr-result:{req_id}"))
        .ok_or_else(|| "ASR 插件未返回结果（流程中断）".to_string())?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("ASR 结果损坏：{e}"))?;
    Ok(value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string())
}

/// 经 `asr.provider` 扩展点插件的远端 ASR 引擎（utterance 粒度，晚绑宿主槽）。
pub struct WasmAsrEngine {
    /// 扩展点名（构造期钉死；实现 id 在调用期按扩展点解析——替换语义对管线透明）。
    extension_point: &'static str,
    /// 测试/嵌入方可注入宿主，避免进程级晚绑槽的并行测试相互污染；生产构建不注入。
    host_override: Option<std::sync::Arc<crate::plugins::wasm::WasmHost>>,
}

impl WasmAsrEngine {
    /// 绑定扩展点（如 `"asr.provider"`）。
    pub fn for_extension(extension_point: &'static str) -> Self {
        Self {
            extension_point,
            host_override: None,
        }
    }

    /// 测试/嵌入构造：固定宿主实例，保持与生产晚绑语义相同但不共享全局槽。
    pub fn with_host(
        host: std::sync::Arc<crate::plugins::wasm::WasmHost>,
        extension_point: &'static str,
    ) -> Self {
        Self {
            extension_point,
            host_override: Some(host),
        }
    }

    /// 扩展点当前实现 id（宿主缺失或无可用实现 → None，由调用方决定语义）。
    fn resolved_plugin_id(&self) -> Option<String> {
        let host = self
            .host_override
            .clone()
            .or_else(crate::plugins::wasm::host_global)?;
        match host.resolve(self.extension_point) {
            crate::plugins::wasm::Resolution::Resolved(id) => Some(id),
            _ => None,
        }
    }
}

impl AsrEngine for WasmAsrEngine {
    fn recognize(&self, samples: &[f32], sample_rate: u32) -> Result<String> {
        let host = self
            .host_override
            .clone()
            .or_else(crate::plugins::wasm::host_global)
            .context("WASM 插件平台未初始化（宿主缺失；见服务启动日志）")?;
        let plugin_id = self
            .resolved_plugin_id()
            .context("ASR 插件未装载（扩展点无可用实现）")?;
        asr_transcribe(&host, &plugin_id, "", samples, sample_rate)
            .map_err(|e| anyhow::anyhow!(e))
            .context("ASR 插件转写失败")
    }
}
