//! 服务配置：从 TOML 文件加载（`Config::load`），默认值即生产路径。
//!
//! ## 引擎与默认值
//!
//! - **无 mock**：ASR 恒为 SenseVoice（sherpa-onnx）、TTS 恒为 Kokoro（sherpa-onnx）、
//!   LLM 恒为 OpenAI 兼容 HTTP（Responses API）。`--features sherpa` 是运行真实引擎的
//!   前提，未启用时引擎构建直接报错（不再有 mock 回退）。
//! - 默认（无 `--config`）即 [`Config::default()`]：模型路径指向 `/models/...`
//!   （与 `config.example.toml`、容器挂载一致），LLM 需填 `api_base`/`api_key`。
//! - 旧配置文件里残留的 `backend = "..."` 键会被 serde 静默忽略（无 `deny_unknown_fields`），
//!   无需手工清理；管理页保存一次即写成新 schema。
//!
//! ## 加载优先级与持久化
//!
//! 配置从哪来、能否被管理页写回，由 `main.rs` 的 `load_config` 决定
//!（`XIAOZHI_CONFIG` → `--config` → 内置默认）；`GET/PUT /api/config` 的读写语义
//! 见 `../server/src/ws.rs`。本文件只负责结构与默认值。

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub asr: AsrConfig,
    #[serde(default)]
    pub vad: VadConfig,
    #[serde(default)]
    pub tts: TtsConfig,
    #[serde(default)]
    pub llm: LlmConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    /// 监听端口。服务器**永远监听 0.0.0.0**（容器/局域网可达性由端口映射或防火墙决定），
    /// 唯一可配的是端口。
    #[serde(default = "default_port")]
    pub port: u16,
    /// 期望的 Bearer token；为空字符串表示不校验 `Authorization`。
    #[serde(default)]
    pub expected_token: String,
    /// tokio 异步运行时的 worker 线程数；默认 2，避免占满低功耗主机（如 N5105 4 核）。
    #[serde(default = "default_worker_threads")]
    pub worker_threads: u32,
    /// 管理页面（h5App 构建产物）静态目录；server 在 `/` 直接托管该目录。
    /// 为空或目录不存在时，`/` 返回友好提示而非崩溃。默认 "../client/apps/h5App/web"。
    #[serde(default = "default_admin_dir")]
    pub admin_dir: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AudioConfig {
    /// 服务器下行（TTS）采样率，写入服务器 hello，设备据此解码播放。
    #[serde(default = "default_downlink_sr")]
    pub downlink_sample_rate: u32,
    #[serde(default = "default_frame_ms")]
    pub downlink_frame_duration_ms: u32,
    #[serde(default = "default_channels")]
    pub channels: u16,
    /// 默认下行二进制协议版本（1/2/3），也可改为跟随设备 hello 的 version。
    #[serde(default = "default_bin_ver")]
    pub binary_protocol_version: u8,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AsrConfig {
    /// SenseVoice INT8 模型路径（sherpa-onnx 离线识别器）。
    #[serde(default = "default_asr_model")]
    pub model: String,
    #[serde(default = "default_asr_tokens")]
    pub tokens: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_true")]
    pub use_itn: bool,
    #[serde(default = "default_num_threads")]
    pub num_threads: u32,
    #[serde(default = "default_provider")]
    pub provider: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VadConfig {
    /// Silero VAD 模型路径；为空视为未配置（sherpa 构建下引擎初始化会报错）。
    #[serde(default = "default_vad_model")]
    pub model: String,
    #[serde(default = "default_threshold")]
    pub threshold: f32,
    #[serde(default = "default_min_silence")]
    pub min_silence_duration: f32,
    #[serde(default = "default_min_speech")]
    pub min_speech_duration: f32,
}

impl VadConfig {
    /// 模型路径非空即视为配置了真实 VAD。
    #[allow(dead_code)]
    pub fn is_real(&self) -> bool {
        !self.model.is_empty()
    }
}

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

/// 科大讯飞在线语音合成（WebSocket v2/tts）凭据与音色。
/// 在讯飞开放平台创建「在线语音合成」应用后取得 APPID/APPKEY/APPSECRET。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct XfyunTtsConfig {
    /// 应用 APPID（请求 common.app_id）。
    #[serde(default)]
    pub app_id: String,
    /// APPKEY（签名 api_key=）。
    #[serde(default)]
    pub api_key: String,
    /// APPSECRET（HMAC-SHA256 签名密钥）。
    #[serde(default)]
    pub api_secret: String,
    /// 发音人（vcn），如 xiaoyan / x4_lingxiaoxuan_oral 等；默认 xiaoyan（讯飞默认女声）。
    #[serde(default = "default_xfyun_voice")]
    pub voice: String,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmConfig {
    /// OpenAI 兼容 Responses API 地址（如 `https://api.example.com/v1/responses`）。
    #[serde(default = "default_api_base")]
    pub api_base: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_llm_model")]
    pub model: String,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
    #[serde(default = "default_max_history")]
    pub max_history: usize,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// 是否流式（SSE）。默认 `true`：逐 token 返回，配合 session 按句下发降低首字延迟；
    /// `false` 退回整段（兼容不支持 SSE 的端点）。
    #[serde(default = "default_stream")]
    pub stream: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            port: default_port(),
            expected_token: String::new(),
            worker_threads: default_worker_threads(),
            admin_dir: default_admin_dir(),
        }
    }
}
impl Default for AudioConfig {
    fn default() -> Self {
        AudioConfig {
            downlink_sample_rate: default_downlink_sr(),
            downlink_frame_duration_ms: default_frame_ms(),
            channels: default_channels(),
            binary_protocol_version: default_bin_ver(),
        }
    }
}
impl Default for AsrConfig {
    fn default() -> Self {
        AsrConfig {
            model: default_asr_model(),
            tokens: default_asr_tokens(),
            language: default_language(),
            use_itn: default_true(),
            num_threads: default_num_threads(),
            provider: default_provider(),
        }
    }
}
impl Default for VadConfig {
    fn default() -> Self {
        VadConfig {
            model: default_vad_model(),
            threshold: default_threshold(),
            min_silence_duration: default_min_silence(),
            min_speech_duration: default_min_speech(),
        }
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
impl Default for LlmConfig {
    fn default() -> Self {
        LlmConfig {
            api_base: default_api_base(),
            api_key: String::new(),
            model: default_llm_model(),
            system_prompt: default_system_prompt(),
            max_history: default_max_history(),
            temperature: default_temperature(),
            stream: default_stream(),
        }
    }
}

impl Config {
    /// 读取并解析 TOML 配置文件。
    pub fn load(path: &str) -> anyhow::Result<Config> {
        let text = std::fs::read_to_string(Path::new(path))
            .map_err(|e| anyhow::anyhow!("读取配置文件 {path} 失败: {e}"))?;
        let cfg: Config = toml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("解析配置文件 {path} 失败: {e}"))?;
        Ok(cfg)
    }

    /// 是否处于「真实音频模式」：`sherpa` feature（唯一路径，无 mock）。
    /// 会话内多处需要此判定，统一在此派生，避免各处重复计算导致漂移。
    pub fn real_audio(&self) -> bool {
        cfg!(feature = "sherpa")
    }
}

// ---- 默认值辅助函数（默认即生产路径，与 config.example.toml / 容器挂载一致）----
fn default_port() -> u16 {
    8000
}
fn default_downlink_sr() -> u32 {
    24_000
}
fn default_frame_ms() -> u32 {
    60
}
fn default_channels() -> u16 {
    1
}
fn default_bin_ver() -> u8 {
    1
}
fn default_asr_model() -> String {
    "/models/SenseVoiceSmall/model.int8.onnx".into()
}
fn default_asr_tokens() -> String {
    "/models/SenseVoiceSmall/tokens.txt".into()
}
fn default_language() -> String {
    "auto".into()
}
fn default_true() -> bool {
    true
}
fn default_num_threads() -> u32 {
    2
}
fn default_vad_model() -> String {
    "/models/silero_vad.onnx".into()
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
fn default_tts_lang() -> String {
    "zh".into()
}
/// tokio worker 线程数默认 2：IO 为主的工作负载足够，为核心数留余量。
fn default_worker_threads() -> u32 {
    2
}

/// 管理页面静态目录默认指向上级 client/apps/h5App 的 web 产物目录
/// （:apps:h5App:publishWeb 汇聚 index.html + nativevue2.js + h5App.js）。
/// 容器部署由镜像内置的 XIAOZHI_ADMIN_DIR=/app/web 覆盖（见 main.rs load_config）。
fn default_admin_dir() -> String {
    "../client/apps/h5App/web".into()
}
fn default_provider() -> String {
    "cpu".into()
}
fn default_threshold() -> f32 {
    0.5
}
fn default_min_silence() -> f32 {
    0.25
}
fn default_min_speech() -> f32 {
    0.25
}
fn default_tts_backend() -> String {
    "sherpa".into()
}
fn default_xfyun_voice() -> String {
    "xiaoyan".into()
}
fn default_tts_model() -> String {
    "/models/Kokoro/model.int8.onnx".into()
}
fn default_tts_voices() -> String {
    "/models/Kokoro/voices.bin".into()
}
fn default_tts_tokens() -> String {
    "/models/Kokoro/tokens.txt".into()
}
fn default_tts_data_dir() -> String {
    "/models/Kokoro/espeak-ng-data".into()
}
fn default_tts_dict_dir() -> String {
    "/models/Kokoro/dict".into()
}
fn default_tts_lexicon() -> String {
    "/models/Kokoro/lexicon-us-en.txt,/models/Kokoro/lexicon-zh.txt".into()
}
fn default_speed() -> f32 {
    1.0
}
fn default_api_base() -> String {
    "https://api.example.com/v1/responses".into()
}
fn default_llm_model() -> String {
    "gpt-4o".into()
}
fn default_system_prompt() -> String {
    "你是一个有用且简洁的中文语音助手。".into()
}
fn default_max_history() -> usize {
    10
}
fn default_temperature() -> f32 {
    0.7
}
fn default_stream() -> bool {
    true
}

