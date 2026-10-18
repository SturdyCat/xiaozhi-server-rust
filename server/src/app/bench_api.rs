//! 配置内联测试（P0-2 后端）：与具体 [`crate::app::session::Session`] 解耦的
//! 免费测量函数，供 HTTP 端点 `POST /api/test/{llm,tts,vad}` 复用。
//!
//! 复用现有 bench 测量语义（`build_llm` / `build_xfyun` / `engines.tts_for` /
//! `engines.new_vad`），但**不依赖 is_test WS 会话**、**不改动共享引擎实例**
//! （TTS 按磁盘最新配置 ad-hoc 构建，`new_vad` 每调用新建一个实例）。
//!
//! ⚠️ 密钥安全：请求里携带的 `api_base` / `api_key` 只用于本次 ad-hoc 构建，
//! **绝不写日志明文、绝不持久化**（不回写磁盘、不入共享 `Engines`）。
//!
//! 两套 feature 编译契约：
//! - 默认（无 `sherpa`）：LLM（OpenAI 兼容 HTTP）与 xfyun TTS（远程）可构建；
//!   kokoro TTS（本地 sherpa）不可 → 对应分支返回 `state="unavailable"`。
//! - `--features sherpa`：本地 Kokoro / Silero VAD 可用，走真实合成/检测路径。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::engine::Engines;
use crate::plugins::llm::{build_llm, LlmEvent};
use crate::plugins::prompt::{MemoryPosition, TurnPrompt};
use crate::plugins::tts::{build_xfyun, TtsEngine};

/// 一次内联测试的统一结果结构（HTTP 端点直接 `Json` 序列化返回）。
#[derive(serde::Serialize)]
pub struct BenchResult {
    /// `"ok"` | `"error"` | `"unavailable"`。
    pub state: &'static str,
    /// 总耗时（毫秒）：LLM = 整轮（首 token→结束）；TTS = 整段合成；VAD = 构建+accept。
    pub elapsed_ms: u64,
    /// 首包延迟（毫秒）：LLM = 首个文本 token 到达；TTS = 首个音频 chunk；
    /// VAD = 引擎构建（冷启动）耗时 / 首个语音段检出耗时。
    pub first_packet_ms: Option<u64>,
    /// 引擎标识（LLM=model；TTS=引擎名；VAD=实现名），便于客户端展示。
    pub engine: Option<String>,
    /// 文本结果/备注（LLM=回复摘要；其余为说明），错误时为 `None`。
    pub text: Option<String>,
    /// 错误文案（`state="error"` 时填充）。
    pub error: Option<String>,
}

impl BenchResult {
    fn ok(
        elapsed_ms: u64,
        first_packet_ms: Option<u64>,
        engine: Option<String>,
        text: Option<String>,
    ) -> Self {
        Self {
            state: "ok",
            elapsed_ms,
            first_packet_ms,
            engine,
            text,
            error: None,
        }
    }
    fn err(elapsed_ms: u64, engine: Option<String>, error: String) -> Self {
        Self {
            state: "error",
            elapsed_ms,
            first_packet_ms: None,
            engine,
            text: None,
            error: Some(error),
        }
    }
    /// 仅默认 feature（无 sherpa）的 kokoro 分支使用；sherpa 构建下该状态不可达。
    #[cfg(not(feature = "sherpa"))]
    fn unavailable(error: String) -> Self {
        Self {
            state: "unavailable",
            elapsed_ms: 0,
            first_packet_ms: None,
            engine: None,
            text: None,
            error: Some(error),
        }
    }
}

/// 按配置来源的最新内容读取 LLM 段；无来源或读取失败时退回启动快照。
fn load_disk_llm(engines: &Arc<Engines>) -> crate::plugins::llm::LlmConfig {
    if engines.config_path().is_none() {
        return engines.config.llm.clone(); // 内置默认配置：无来源可读
    }
    engines
        .config_store
        .load()
        .map(|c| c.llm)
        .unwrap_or_else(|e| {
            tracing::warn!("内联 LLM 测试读盘配置失败（用启动配置）: {e}");
            engines.config.llm.clone()
        })
}

/// 按配置来源的最新内容读取 TTS 段；无来源或读取失败时退回启动快照。
fn load_disk_tts(engines: &Arc<Engines>) -> crate::plugins::tts::TtsConfig {
    if engines.config_path().is_none() {
        return engines.config.tts.clone(); // 内置默认配置：无来源可读
    }
    engines
        .config_store
        .load()
        .map(|c| c.tts)
        .unwrap_or_else(|e| {
            tracing::warn!("内联 TTS 测试读盘配置失败（用启动配置）: {e}");
            engines.config.tts.clone()
        })
}

/// LLM 连通性 + 首 token 延迟测量（单轮无历史）。
///
/// 复用 `build_llm` 临时构建客户端；`api_base` / `api_key` 若提供则**仅本次**覆盖
/// （不写盘、不打印明文）。回调记录首个文本 token 到达时刻作为首包延迟。
pub async fn bench_llm(
    engines: &Arc<Engines>,
    text: &str,
    api_base: Option<&str>,
    api_key: Option<&str>,
) -> BenchResult {
    if text.trim().is_empty() {
        return BenchResult::err(0, None, "测试文本为空".to_string());
    }
    // 按磁盘最新配置临时构建：改完配置无需重启即可验证刚保存的 api_base/api_key/model。
    let mut llm_cfg = load_disk_llm(engines);
    if let Some(b) = api_base {
        llm_cfg.api_base = b.to_string();
    }
    if let Some(k) = api_key {
        // ⚠️ 仅本次 ad-hoc 使用，绝不打印、绝不持久化。
        llm_cfg.api_key = k.to_string();
    }
    let engine_label = llm_cfg.model.clone();
    let llm = build_llm(&llm_cfg);
    // 内联测试走"系统提示词"组装的 prompt（与人格生效口径一致，但不拉起记忆/灵魂引擎，
    // 保持与 Session 解耦）；`memory` 留空，仅测 LLM 连通性 + 首 token 延迟。
    let prompt = TurnPrompt {
        instructions: llm_cfg.system_prompt.clone(),
        memory: None,
        position: MemoryPosition::default(),
    };
    let t0 = Instant::now();
    let first = Arc::new(Mutex::new(None::<u64>));
    let first_cb = first.clone();
    let result = {
        let t0_cb = t0;
        llm.chat_stream(
            &[],
            text,
            &prompt,
            None,
            Box::new(move |ev: LlmEvent| {
                if matches!(ev, LlmEvent::Text(_)) {
                    let mut g = first_cb.lock().unwrap_or_else(|e| e.into_inner());
                    if g.is_none() {
                        *g = Some(t0_cb.elapsed().as_millis() as u64);
                    }
                }
                true
            }),
        )
        .await
    };
    let elapsed = t0.elapsed().as_millis() as u64;
    let first_packet_ms = first.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match result {
        Ok(r) => BenchResult::ok(elapsed, first_packet_ms, Some(engine_label), Some(r.text)),
        Err(e) => BenchResult::err(elapsed, Some(engine_label), format!("{e:#}")),
    }
}

/// TTS 首包延迟测量（流式合成，记录首个音频 chunk 到达时刻）。
///
/// 复用 `build_xfyun` / `engines.tts_for` 按磁盘最新配置 ad-hoc 构建（**不改动共享引擎**）。
/// - 引擎为 xfyun（远程，默认 feature 可用）：直接构建并合成，`vcn` 可覆盖音色。
/// - 引擎为 kokoro（本地 sherpa）：`--features sherpa` 走真实合成；默认 feature 下
///   本地 TTS 不可用 → 返回 `state="unavailable"`。
///
/// 推理闸门（缺口①）：与生产会话共用同一把 TTS 闸门（HTTP 内联测试与生产共享 CPU）。
pub async fn bench_tts(
    engines: &Arc<Engines>,
    text: &str,
    lang: Option<&str>,
    vcn: Option<&str>,
    speed: Option<f32>,
) -> BenchResult {
    if text.trim().is_empty() {
        return BenchResult::err(0, None, "测试文本为空".to_string());
    }
    let disk_tts = load_disk_tts(engines);
    let engine_id = disk_tts.engine_id();
    let speed = speed.unwrap_or(disk_tts.speed);
    let speaker = disk_tts.speaker;
    let lang = lang
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| disk_tts.kokoro.lang.as_str());
    let vcn_override = vcn.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let gate = engines.inference.tts();

    // xfyun（远程）路径：默认 feature 下也可构建。
    if engine_id == "xfyun" {
        let mut xfyun = disk_tts.xfyun.clone();
        if let Some(v) = &vcn_override {
            xfyun.voice = v.clone();
        }
        let tts = match build_xfyun(&xfyun) {
            Ok(e) => e,
            Err(e) => return BenchResult::err(0, Some("xfyun".to_string()), format!("{e:#}")),
        };
        let engine_name = tts.name().to_string();
        return run_tts_bench(tts, gate, text.to_string(), speed, speaker, engine_name).await;
    }

    // kokoro（本地 sherpa）路径。
    #[cfg(feature = "sherpa")]
    {
        let tts = engines.tts_for(&lang);
        let engine_name = tts.name().to_string();
        return run_tts_bench(tts, gate, text.to_string(), speed, speaker, engine_name).await;
    }
    #[cfg(not(feature = "sherpa"))]
    {
        let _ = (lang, vcn_override, speaker, gate);
        BenchResult::unavailable(
            "本地 TTS 需 sherpa 构建（本二进制未启用 sherpa feature，请用 --features sherpa 编译）"
                .to_string(),
        )
    }
}

/// 在 `spawn_blocking` 中跑同步阻塞的流式合成，记录首个音频 chunk 时刻（首包延迟）。
/// 过 TTS 推理闸门（缺口①）：与生产会话共用，permit move 进闭包 RAII 归还。
#[allow(clippy::too_many_arguments)] // 与既有参数 + 闸门入参同性质
async fn run_tts_bench(
    tts: Arc<dyn TtsEngine>,
    tts_gate: crate::app::inference::GateHandle,
    text: String,
    speed: f32,
    speaker: i32,
    engine_name: String,
) -> BenchResult {
    let first = Arc::new(Mutex::new(None::<u64>));
    let first_cb = first.clone();
    // 内联测试无会话级 abort：裸 acquire（排队等待不响应打断，测完为止）。
    let Some(permit) = tts_gate.acquire().await else {
        return BenchResult::err(0, Some(engine_name), "TTS 推理闸门已关闭".to_string());
    };
    let producer = tokio::task::spawn_blocking(move || {
        let _gate_permit = permit; // RAII：闭包退出即归还
        let t0 = Instant::now();
        let r = tts.synthesize_stream(
            &text,
            speed,
            speaker,
            Box::new(move |_sr, _chunk| {
                let mut g = first_cb.lock().unwrap_or_else(|e| e.into_inner());
                if g.is_none() {
                    *g = Some(t0.elapsed().as_millis() as u64);
                }
                true
            }),
        );
        (r, t0.elapsed().as_millis() as u64)
    });
    let (r, elapsed) = match producer.await {
        Ok(v) => v,
        Err(e) => {
            return BenchResult::err(0, Some(engine_name), format!("合成任务异常: {e}"));
        }
    };
    let first_packet_ms = first.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match r {
        Ok(()) => BenchResult::ok(elapsed, first_packet_ms, Some(engine_name), None),
        Err(e) => BenchResult::err(elapsed, Some(engine_name), format!("{e:#}")),
    }
}

/// VAD 构建 + 首次检测耗时测量（**全新**，bench 的 WS 测试台无 VAD 测试）。
///
/// 复用 `engines.new_vad()` 新建实例（默认 feature 下为 MockVad、零成本；
/// `--features sherpa` 下加载 Silero ONNX 模型，成本主要在构建）。
/// 对一段合成 PCM 调 `accept` 一次，记录引擎构建（冷启动）耗时与首个语音段检出耗时。
///
/// 整个构建+检测在 `spawn_blocking` 中执行：sherpa 下 `new_vad` 加载模型为同步 CPU 调用，
/// 不隔离会独占 tokio worker。
pub async fn bench_vad(engines: &Arc<Engines>) -> BenchResult {
    let engines = engines.clone();
    let engine_name = if cfg!(feature = "sherpa") {
        "silero".to_string()
    } else {
        "mock".to_string()
    };
    let result = tokio::task::spawn_blocking(move || {
        let t0 = Instant::now();
        let mut vad = match engines.new_vad() {
            Ok(v) => v,
            Err(e) => return Err((t0.elapsed().as_millis() as u64, format!("{e:#}"))),
        };
        let build_ms = t0.elapsed().as_millis() as u64;
        // 合成约 0.5s 的响度信号（正弦），用于触发能量门限类 VAD 的"语音段"检出；
        // 真实 Silero 对纯正弦未必判为语音，故检出失败属预期，fallback 为构建耗时。
        let sr: usize = 16_000;
        let n = sr / 2;
        let samples: Vec<f32> = (0..n)
            .map(|i| 0.8 * (2.0 * std::f32::consts::PI * i as f32 / 16.0).sin())
            .collect();
        let detect_t0 = Instant::now();
        let mut first_detect: Option<u64> = None;
        vad.accept(&samples, &mut |_seg| {
            if first_detect.is_none() {
                first_detect = Some(detect_t0.elapsed().as_millis() as u64);
            }
        });
        let elapsed = t0.elapsed().as_millis() as u64;
        Ok((build_ms, first_detect, elapsed))
    })
    .await;
    match result {
        Ok(Ok((build_ms, first_detect, elapsed))) => {
            // 首包延迟优先取"首个语音段检出耗时"，未检出则取引擎构建（冷启动）耗时。
            let first_packet_ms = first_detect.or(Some(build_ms));
            let note = if first_detect.is_none() {
                Some("未检出语音段（示例信号非真实语音，首包延迟取引擎构建耗时）".to_string())
            } else {
                None
            };
            BenchResult::ok(elapsed, first_packet_ms, Some(engine_name), note)
        }
        Ok(Err((build_ms, e))) => BenchResult::err(build_ms, Some(engine_name), e),
        Err(e) => BenchResult::err(0, Some(engine_name), format!("VAD 测试任务异常: {e}")),
    }
}

/// 模型列表结果（`POST /api/test/llm/models`）。
#[derive(serde::Serialize)]
pub struct ModelsResult {
    /// `"ok"` | `"error"`。
    pub state: &'static str,
    /// 模型 id 列表（排序去重）。
    pub models: Vec<String>,
    pub elapsed_ms: u64,
    /// 错误文案（`state="error"` 时填充）。
    pub error: Option<String>,
}

/// 由 Responses API 端点推导 `/models` 列表地址：`…/v1/responses` → `…/v1/models`；
/// 已以 `/models` 结尾则原样使用；其余结构直接追加 `/models`。
fn models_url(api_base: &str) -> String {
    let b = api_base.trim().trim_end_matches('/');
    if let Some(stripped) = b.strip_suffix("/responses") {
        format!("{stripped}/models")
    } else if b.ends_with("/models") {
        b.to_string()
    } else {
        format!("{b}/models")
    }
}

/// 拉取上游模型列表（OpenAI 兼容 `GET {base}/models`）。
///
/// 与 [`bench_llm`] 同口径：读磁盘最新 `[llm]` 配置，请求里的 `api_base` / `api_key`
/// 仅本次覆盖（绝不写日志明文、绝不持久化）；`api_key` 为空时多数本地网关
/// （ollama/llama.cpp 等）允许匿名访问，故不强制非空。
pub async fn llm_models(
    engines: &Arc<Engines>,
    api_base: Option<&str>,
    api_key: Option<&str>,
) -> ModelsResult {
    let t0 = Instant::now();
    let mut cfg = load_disk_llm(engines);
    if let Some(b) = api_base {
        if !b.trim().is_empty() {
            cfg.api_base = b.to_string();
        }
    }
    if let Some(k) = api_key {
        // ⚠️ 仅本次 ad-hoc 使用，绝不打印、绝不持久化。
        if !k.trim().is_empty() {
            cfg.api_key = k.to_string();
        }
    }
    if cfg.api_base.trim().is_empty() {
        return ModelsResult {
            state: "error",
            models: vec![],
            elapsed_ms: 0,
            error: Some("[llm].api_base 为空".into()),
        };
    }
    let url = models_url(&cfg.api_base);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let mut req = client.get(&url);
    let key = cfg.api_key.trim();
    if !key.is_empty() {
        req = req.bearer_auth(key);
    }
    let elapsed = t0.elapsed().as_millis() as u64;
    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                let body: String = body.chars().take(300).collect();
                return ModelsResult {
                    state: "error",
                    models: vec![],
                    elapsed_ms: elapsed,
                    error: Some(format!("HTTP {}：{body}", status.as_u16())),
                };
            }
            match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(v) => {
                    // OpenAI 形状 `{"data":[{"id":…}]}`；兼容 `{"models":[…]}`（字符串或
                    // {"name"/"id":…}，ollama 原生 /api/tags 形状）。
                    let mut models: Vec<String> = Vec::new();
                    let items = v
                        .get("data")
                        .and_then(|d| d.as_array())
                        .or_else(|| v.get("models").and_then(|m| m.as_array()));
                    if let Some(items) = items {
                        for it in items {
                            let id = if let Some(s) = it.as_str() {
                                Some(s.to_string())
                            } else {
                                it.get("id")
                                    .and_then(|x| x.as_str())
                                    .or_else(|| it.get("name").and_then(|x| x.as_str()))
                                    .map(|s| s.to_string())
                            };
                            if let Some(id) = id {
                                if !id.is_empty() {
                                    models.push(id);
                                }
                            }
                        }
                    }
                    models.sort();
                    models.dedup();
                    ModelsResult {
                        state: "ok",
                        models,
                        elapsed_ms: elapsed,
                        error: None,
                    }
                }
                Err(e) => ModelsResult {
                    state: "error",
                    models: vec![],
                    elapsed_ms: elapsed,
                    error: Some(format!("响应非 JSON：{e}")),
                },
            }
        }
        Err(e) => ModelsResult {
            state: "error",
            models: vec![],
            elapsed_ms: elapsed,
            error: Some(format!("请求失败：{e}")),
        },
    }
}
