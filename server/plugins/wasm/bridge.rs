//! skills 层 ↔ weather 插件的事件桥（P2c 生产接线）。
//!
//! skills 目录「不联网」铁律不变：网络发生在 wasm 插件效果（宿主 fetcher 执行）。
//! 本桥只做四件事：扩展点解析（替换语义对 skills 透明）→ geo 预填（宿主服务，
//! 事件源侧职责）→ 派发事件（宿主同步跑完整瀑布）→ 从插件 kv 结果通道取回 JSON。
//!
//! 调用纪律：`handle_event` 内含阻塞 HTTP（生产 fetcher），调用方必须在
//! `spawn_blocking`（skills 计划执行本就如此，与原生 reqwest 同纪律）。

use serde::Deserialize;
use serde_json::{json, Value};

use super::registry::WasmHost;

mod nlu;
mod policies;
use super::Resolution;
pub use nlu::nlu_catalog_prepare;
pub use policies::{
    debug_prepare, memory_prepare, skill_prepare, soul_prepare, tuya_prepare, wakeup_prepare,
};

/// 经 `skill.weather` 扩展点查天气（官方默认 `weather.official` 或用户兼容实现）。
/// 与原生 `WeatherService::forecast` 同契约：永远返回成功形态的 `Value`。
pub fn forecast(host: &WasmHost, city: &str, province: &str, range: &str) -> Value {
    // ① 扩展点解析：未装载/全部失效 → 明确报缺（管理台上传或启动引导构建）
    let plugin_id = match host.resolve("skill.weather") {
        Resolution::Resolved(id) => id,
        other => {
            return json!({
                "ok": false,
                "speech": "天气插件未装载（管理台上传 weather.official 或检查启动引导日志）",
                "resolution": format!("{other:?}"),
            })
        }
    };

    // ② geo 预填：空城市回落服务器出口 IP 定位（与原生同口径；属宿主服务，
    //    插件侧按缺参明确报错——预填是事件源侧职责，见设计文档）
    let (city_clean, province_eff) = if city.trim().is_empty() {
        match crate::plugins::geo::current() {
            Some(loc) => {
                let c = if loc.city.trim().is_empty() {
                    loc.province.clone()
                } else {
                    loc.city.clone()
                };
                (c, loc.province.clone())
            }
            None => {
                return json!({
                    "ok": false,
                    "speech": "未指定城市，且服务器未能定位默认地区（可能处于境外网络），请告诉我城市名称",
                })
            }
        }
    } else {
        (city.trim().to_string(), province.trim().to_string())
    };

    // ③ 派发：宿主同步驱动完整瀑布（缓存命中→单步；冷路径→3 次批量回灌）
    let req_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({
        "req_id": req_id,
        "city": city_clean,
        "province": province_eff,
        "range": range,
    })
    .to_string();
    if let Err(e) = host.handle_event(&plugin_id, "weather.forecast", &payload) {
        tracing::warn!(plugin = %plugin_id, "天气插件事件失败：{e:#}");
        return json!({"ok": false, "speech": "天气服务暂时不可用，请稍后再试"});
    }

    // ④ 结果通道：插件 kv `result:<req_id>`（失败也是 ok:false + 话术，同原生口径）
    match host.kv().get(&plugin_id, &format!("result:{req_id}")) {
        Some(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            tracing::warn!(plugin = %plugin_id, "天气结果损坏：{e}");
            json!({"ok": false, "speech": "天气服务暂时不可用，请稍后再试"})
        }),
        None => {
            tracing::warn!(plugin = %plugin_id, "天气插件未写入结果（流程中断）");
            json!({"ok": false, "speech": "天气服务暂时不可用，请稍后再试"})
        }
    }
}

/// 会话**观察者挂钩**派发（P2d）：workflow 绑定到 hook 位的 wasm 插件在此收
/// `hook.<position>` 事件（payload `{session_id, device_id, position, text?}`），
/// 返回效果由宿主**异步执行**（fire-and-forget），不阻塞对话轮、不回灌、
/// 任何失败只告警——旁路观察者永不打断轮次。调用方须在 spawn_blocking 内。
pub fn observe_hook(host: &WasmHost, plugin_id: &str, position: &str, payload: &str) {
    let event = format!("hook.{position}");
    match host.try_dispatch(plugin_id, &event, payload) {
        Ok(Some(effects)) => host.spawn_deferred(plugin_id, effects),
        Ok(None) => {}
        Err(e) => tracing::warn!(
            plugin = plugin_id,
            position,
            "观察者挂钩失败（轮次继续）：{e:#}"
        ),
    }
}

// ============================================================
// P4a：asr.provider 事件桥（utterance 级；推理留原生铁律不变——wasm 是
// 远程引擎变体通道，本地 SenseVoice 路径零改动）
// ============================================================

use base64::Engine as _;

///  utterance 转写：`asr.transcribe` 事件（payload JSON 含 f32-le PCM 的 base64），
/// 插件经 kv 结果通道返回 `{"text":"..."}`（weather 同款结果协议）。
/// 调用纪律：`recognize` 本就是同步 trait 且管线在 spawn_blocking 内调用——与原生
/// reqwest/引擎推理同一纪律。错误 = 显式 Err（会话侧按 on_error 语义处理）。
pub fn asr_transcribe(
    host: &WasmHost,
    plugin_id: &str,
    session_id: &str,
    samples: &[f32],
    sample_rate: u32,
) -> std::result::Result<String, String> {
    let plugin_id = host
        .observers()
        .into_iter()
        .find(|(id, _)| id == plugin_id)
        .map(|(id, _)| id)
        .ok_or_else(|| format!("ASR 插件 {plugin_id} 未装载"))?;
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
    host.handle_event(&plugin_id, "asr.transcribe", &payload)
        .map_err(|e| format!("ASR 插件事件失败：{e:#}"))?;
    let bytes = host
        .kv()
        .get(&plugin_id, &format!("asr-result:{req_id}"))
        .ok_or_else(|| "ASR 插件未返回结果（流程中断）".to_string())?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| format!("ASR 结果损坏：{e}"))?;
    Ok(value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

// ============================================================
// P4：tts.provider 控制策略桥（PCM/模型推理仍留原生）
// ============================================================

const MAX_TTS_TEXT_BYTES: usize = 16 * 1024;
const MAX_TTS_ROUTE_BYTES: usize = 64;

/// WASM 策略返回的控制结果；它不携带任何 PCM 或音频 base64。
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct TtsPrepareResult {
    pub text: String,
    #[serde(default)]
    pub speed: Option<f32>,
    #[serde(default)]
    pub speaker: Option<i32>,
    #[serde(default)]
    pub route: Option<String>,
}

/// 向 `tts.provider` 派发一次整句控制事件并读取策略结果。
///
/// 请求 payload 只包含文本与合成参数；结果至少包含 `text`，可选覆盖 `speed`、
/// `speaker`、`route`。宿主/插件缺失、事件错误、结果缺失或 JSON 非法均显式返回 Err，
/// 不回退到未经过策略的 native 文本。
pub fn tts_prepare(
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
    if host.observers().into_iter().all(|(id, _)| id != plugin_id) {
        return Err(format!("TTS 策略插件 {plugin_id} 未装载"));
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
            return Err(format!(
                "TTS 策略 route 超长（{} > {MAX_TTS_ROUTE_BYTES} bytes）",
                route.len()
            ));
        }
    }
    if let Some(result_speed) = result.speed {
        if !result_speed.is_finite() || result_speed <= 0.0 {
            return Err(format!("TTS 策略 speed 非法：{result_speed}"));
        }
    }
    Ok(result)
}

// ============================================================
// P4d：vad.provider 控制策略桥（逐帧 VAD/PCM 仍留 native）
// ============================================================

const MAX_VAD_ROUTE_BYTES: usize = 64;
const MAX_VAD_DURATION_SECONDS: f32 = 10.0;

/// WASM VAD 策略只返回 native 构造参数，不接收或返回 PCM/音频帧。
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct VadPrepareResult {
    #[serde(default)]
    pub threshold: Option<f32>,
    #[serde(default)]
    pub min_silence_duration: Option<f32>,
    #[serde(default)]
    pub min_speech_duration: Option<f32>,
    #[serde(default)]
    pub route: Option<String>,
}

/// 向 `vad.provider` 派发一次会话级策略事件。
///
/// 事件只携带采样率和当前 VAD 控制参数；插件返回的覆盖值用于构造 native
/// `VadEngine`。逐帧 `accept`/`flush` 调用不会进入 WASM。
pub fn vad_prepare(
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
    if host.observers().into_iter().all(|(id, _)| id != plugin_id) {
        return Err(format!("VAD 策略插件 {plugin_id} 未装载"));
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

// ============================================================
// P6a：device.echomuse 会话入口策略（WS/keepalive/hub/PTY 留 native）
// ============================================================

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

// ============================================================
