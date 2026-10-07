//! 语音合成（TTS）引擎：本地 `sherpa-onnx` Kokoro INT8（`backend = "sherpa"`）
//! 或科大讯飞在线合成（`backend = "xfyun"`，见 [`super::xfyun`]）。无 mock。
//!
//! 合成结果为单声道 f32 PCM；下行前由音频层做（按需）重采样与 Opus 编码。
//!
//! ## 流式接口
//! [`TtsEngine::synthesize_stream`] 是**首选入口**：合成过程中按片回调 PCM（增量），
//! 调用方边收边下发，显著降低首字延迟（讯飞为服务端分片返回；Kokoro 为逐段回调）。
//! [`TtsEngine::synthesize`] 保留给需要整段的场景，默认按流式收集实现（反之亦然）。
//!
//! ⚠️ 两者都是**同步阻塞调用**（本地推理实测 8~20s；在线为网络等待），调用方
//!（[`crate::app::session`]）必须用 `spawn_blocking` 隔离，否则独占 tokio worker，
//! 期间同 runtime 的其他会话/HTTP 全部卡死。

use serde::{Deserialize, Serialize};
/// TTS 配置分两层：
/// - **本地模型**：`[tts]` 本体的 model/voices/tokens/... 字段（当前引擎 `sherpa` = Kokoro INT8）；
/// - **远程服务**：`[tts.<provider>]` 独立凭据段（现有 `xfyun`；未来 azure/openai/... 各自一段）。
///
/// 扩展新远程供应商 = ① `[tts].backend` 加可选 id；② 新增 `[tts.<id>]` 凭据段 + serde 结构；
/// ③ `build_tts` 加分支 + 引擎实现（参照 xfyun_tts.rs）；④ 客户端 ConfigFormState 的
/// `ttsRemoteEngines` 注册表追加一项 + 卡片字段区按 id 追加 vif。UI 按「本地/远程」两级下拉区分。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TtsConfig {
    /// 引擎 id：`"sherpa"`（本地 Kokoro，默认）| `"xfyun"`（科大讯飞在线）| 未来其他远程供应商 id。
    #[serde(default = "default_tts_backend")]
    pub backend: String,
    /// Kokoro INT8 模型路径（sherpa-onnx 离线合成器）。
    #[serde(default = "default_tts_model")]
    pub model: String,
    #[serde(default = "default_tts_voices")]
    pub voices: String,
    #[serde(default = "default_tts_tokens")]
    pub tokens: String,
    #[serde(default = "default_tts_data_dir")]
    pub data_dir: String,
    #[serde(default = "default_tts_dict_dir")]
    pub dict_dir: String,
    #[serde(default = "default_tts_lexicon")]
    pub lexicon: String,
    /// Kokoro 语言（模型创建时固定；"zh"/"en"，中文场景用 "zh"）。
    #[serde(default = "default_tts_lang")]
    pub lang: String,
    #[serde(default)]
    pub speaker: i32,
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(default = "default_tts_threads")]
    pub num_threads: u32,
    /// 科大讯飞在线 TTS（backend = "xfyun" 时使用）。
    #[serde(default)]
    pub xfyun: XfyunTtsConfig,
}
/// TTS 引擎种类（由 `[tts].backend` 派生；未知值按 sherpa 处理）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtsBackendKind {
    /// 本地 Kokoro INT8（sherpa-onnx）。
    Sherpa,
    /// 科大讯飞在线语音合成（WebSocket v2/tts）。
    Xfyun,
}

impl TtsBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TtsBackendKind::Sherpa => "sherpa",
            TtsBackendKind::Xfyun => "xfyun",
        }
    }
}

impl TtsConfig {
    /// `backend` 字段 → 引擎种类（`"xfyun"` 忽略大小写匹配；其余一律 sherpa）。
    pub fn backend_kind(&self) -> TtsBackendKind {
        if self.backend.eq_ignore_ascii_case("xfyun") {
            TtsBackendKind::Xfyun
        } else {
            TtsBackendKind::Sherpa
        }
    }

    /// 引擎签名（热切换判定用）：backend 或关键参数变化时才重建引擎。
    /// ⚠️ 仅内存内部使用，**不要打日志**（含密钥字段）。
    pub fn engine_signature(&self) -> String {
        match self.backend_kind() {
            TtsBackendKind::Xfyun => format!(
                "xfyun|{}|{}|{}|{}",
                self.xfyun.app_id, self.xfyun.api_key, self.xfyun.api_secret, self.xfyun.voice
            ),
            TtsBackendKind::Sherpa => format!("sherpa|{}|{}|{}", self.model, self.lang, self.num_threads),
        }
    }
}
fn default_tts_backend() -> String {
    "sherpa".into()
}

fn default_tts_model() -> String {
    "/data/models/Kokoro/model.int8.onnx".into()
}

fn default_tts_voices() -> String {
    "/data/models/Kokoro/voices.bin".into()
}

fn default_tts_tokens() -> String {
    "/data/models/Kokoro/tokens.txt".into()
}

fn default_tts_data_dir() -> String {
    "/data/models/Kokoro/espeak-ng-data".into()
}

fn default_tts_dict_dir() -> String {
    "/data/models/Kokoro/dict".into()
}

fn default_tts_lexicon() -> String {
    "/data/models/Kokoro/lexicon-us-en.txt,/data/models/Kokoro/lexicon-zh.txt".into()
}

fn default_tts_lang() -> String {
    "zh".into()
}

fn default_speed() -> f32 {
    1.0
}

/// TTS 合成线程数默认 4。
/// 旧默认 1 的理由是「与 ASR 错峰」——实机实测该顾虑不成立：语音流水线本身
/// ASR → LLM → TTS 串行，TTS 合成时 ASR 并不在跑；单线程让合成只剩 1/4 算力，
/// 实测 RTF≈7（4 个字要 8+ 秒，CPU 仅 25%）。多设备并发是吞吐问题、4 核本来就不够，
/// 不该牺牲单次合成的延迟。容器部署已由 docker-compose.yml 的 cpus:"3.5" 限核
/// （留 0.5 核给宿主机），4 线程在配额内调度；如需更保守可下调到 3。
fn default_tts_threads() -> u32 {
    4
}

#[cfg(feature = "sherpa")]
mod kokoro;
// pub(crate)：AIUI 全链路插件复用同厂商的 HMAC 签名算法
pub(crate) mod xfyun;

// bin crate 内部暂无直接引用者：作为插件对外 API 面保留
#[allow(unused_imports)]
pub use xfyun::{XfyunTts, XfyunTtsConfig};

use anyhow::Result;
#[cfg(feature = "sherpa")]
use anyhow::Context;
#[cfg(feature = "sherpa")]
use kokoro::SherpaTts;
use std::sync::Arc;

/// 流式分片回调：`(输出采样率, 本片增量 PCM)`；返回 `false` 表示调用方要求取消
///（如打断/连接断开），引擎应尽快停止合成并返回 `Ok(())`。
pub type TtsChunkCallback = Box<dyn FnMut(u32, &[f32]) -> bool + Send>;

/// 合成引擎接口：文本 → 单声道 f32 PCM + 采样率。
/// `speaker` 为语音角色（Kokoro sid / 讯飞忽略），由调用方按请求传入。
pub trait TtsEngine: Send + Sync {
    /// 流式合成（**引擎必须实现**）：按片回调**增量** PCM（非累计），返回时合成结束。
    /// 回调返回 `false` = 调用方取消（打断/断开），引擎应尽快停止并返回 `Ok(())`。
    fn synthesize_stream(
        &self,
        text: &str,
        speed: f32,
        speaker: i32,
        chunk_cb: TtsChunkCallback,
    ) -> Result<()>;

    /// 整段合成（默认：收集 [`Self::synthesize_stream`] 的输出）。
    /// 当前调用方均走流式；保留为接口完整性（如离线批量合成）。
    #[allow(dead_code)]
    fn synthesize(&self, text: &str, speed: f32, speaker: i32) -> Result<(Vec<f32>, u32)> {
        // 回调要求 'static（且引擎实现内部可能再 move）：状态放共享容器，外部读取
        let collected: Arc<std::sync::Mutex<(Vec<f32>, u32)>> =
            Arc::new(std::sync::Mutex::new((Vec::new(), 0)));
        let sink = collected.clone();
        self.synthesize_stream(
            text,
            speed,
            speaker,
            Box::new(move |rate, chunk| {
                let mut c = sink.lock().unwrap_or_else(|e| e.into_inner());
                c.1 = rate;
                c.0.extend_from_slice(chunk);
                true
            }),
        )?;
        let out = std::mem::take(&mut *collected.lock().unwrap_or_else(|e| e.into_inner()));
        Ok(out)
    }

    /// 合成输出采样率（下发链路的重采样源采样率）。
    fn output_sample_rate(&self) -> u32 {
        24_000
    }

    /// 引擎标识（测试台 tts_test 结果回报用）：`sherpa` / `xfyun`。
    fn name(&self) -> &'static str {
        "unknown"
    }
}

/// sherpa 真引擎构造（两入口共用）：集中 `SherpaTts::new` 与报错文案，避免重复。
#[cfg(feature = "sherpa")]
fn build_sherpa_tts(cfg: &TtsConfig) -> Result<Arc<dyn TtsEngine>> {
    let engine = SherpaTts::new(cfg).context("创建 Kokoro TTS 失败")?;
    Ok(Arc::new(engine))
}

/// 根据配置构造 TTS 引擎：`backend` 选择 `sherpa`（本地 Kokoro）或 `xfyun`（讯飞在线）。
pub fn build_tts(cfg: &TtsConfig) -> Result<Arc<dyn TtsEngine>> {
    match cfg.backend_kind() {
        TtsBackendKind::Xfyun => Ok(Arc::new(xfyun::XfyunTts::new(&cfg.xfyun)?)),
        TtsBackendKind::Sherpa => build_sherpa(cfg),
    }
}

/// 按指定语言构造 TTS 引擎（测试台的多语言切换用；仅对 Kokoro 有意义）。
/// Kokoro 的 `lang` 在建模时固定，切语言 = 重建引擎（engine.rs 按语言缓存）；
/// 讯飞在线引擎由 `[tts.xfyun].voice` 决定音色，忽略 `lang`。
pub fn build_tts_with_lang(cfg: &TtsConfig, lang: &str) -> Result<Arc<dyn TtsEngine>> {
    match cfg.backend_kind() {
        TtsBackendKind::Xfyun => build_tts(cfg),
        TtsBackendKind::Sherpa => {
            #[cfg(feature = "sherpa")]
            {
                let mut c = cfg.clone();
                c.lang = lang.to_string();
                // build_sherpa_tts 已返回 Arc<dyn TtsEngine>——不要再 Arc::new 包一层
                //（曾因双层 Arc 致 Docker sherpa 构建失败，见 18b6e8d）。
                build_sherpa_tts(&c).with_context(|| format!("创建 lang={lang} 的 Kokoro TTS 失败"))
            }
            #[cfg(not(feature = "sherpa"))]
            {
                let _ = lang;
                anyhow::bail!("本二进制未启用 `sherpa` feature，无法构建 TTS 引擎（请用 --features sherpa 编译）");
            }
        }
    }
}

/// sherpa（本地 Kokoro）分支：未启用 feature 直接报错。
fn build_sherpa(cfg: &TtsConfig) -> Result<Arc<dyn TtsEngine>> {
    #[cfg(feature = "sherpa")]
    {
        build_sherpa_tts(cfg)
    }
    #[cfg(not(feature = "sherpa"))]
    {
        let _ = cfg;
        anyhow::bail!("本二进制未启用 `sherpa` feature，无法构建 TTS 引擎（请用 --features sherpa 编译）");
    }
}

impl Default for TtsConfig {
    fn default() -> Self {
        TtsConfig {
            backend: default_tts_backend(),
            model: default_tts_model(),
            voices: default_tts_voices(),
            tokens: default_tts_tokens(),
            data_dir: default_tts_data_dir(),
            dict_dir: default_tts_dict_dir(),
            lexicon: default_tts_lexicon(),
            lang: default_tts_lang(),
            speaker: 0,
            speed: default_speed(),
            num_threads: default_tts_threads(),
            xfyun: XfyunTtsConfig::default(),
        }
    }
}
