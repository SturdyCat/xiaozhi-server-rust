//! 服务配置：从 TOML 文件加载，并提供 mock 友好的默认值。
//!
//! - 真实部署：用 `--config config.toml` 加载完整配置（路径指向模型文件）。
//! - 本地无模型测试：不带 `--config` 时回退到 [`Config::default()`]，
//!   其 `asr.backend` / `tts.backend` 均为 `mock`，可直接 `cargo run`。

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
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
    #[serde(default = "default_listen")]
    pub listen: String,
    /// 期望的 Bearer token；为空字符串表示不校验 `Authorization`。
    #[serde(default)]
    pub expected_token: String,
    /// tokio 异步运行时的 worker 线程数；默认 2，避免占满低功耗主机（如 N5105 4 核）。
    #[serde(default = "default_worker_threads")]
    pub worker_threads: u32,
    /// 管理页面（h5App 构建产物）静态目录；server 在 `/` 直接托管该目录。
    /// 为空或目录不存在时，`/` 返回友好提示而非崩溃。默认 "../client/apps/h5App/dist"。
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

// 模型路径等字段仅 `sherpa` feature 下消费；mock 模式下仅作为配置 schema 保留。
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AsrConfig {
    /// `sherpa` 或 `mock`。
    #[serde(default = "default_asr_backend")]
    pub backend: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
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

// 模型路径等字段仅 `sherpa` feature 下消费；mock 模式下仅作为配置 schema 保留。
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VadConfig {
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_threshold")]
    pub threshold: f32,
    #[serde(default = "default_min_silence")]
    pub min_silence_duration: f32,
    #[serde(default = "default_min_speech")]
    pub min_speech_duration: f32,
}

// 模型路径等字段仅 `sherpa` feature 下消费；mock 模式下仅作为配置 schema 保留。
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TtsConfig {
    /// `sherpa` 或 `mock`。
    #[serde(default = "default_tts_backend")]
    pub backend: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub voices: String,
    #[serde(default)]
    pub tokens: String,
    #[serde(default)]
    pub data_dir: String,
    #[serde(default)]
    pub dict_dir: String,
    #[serde(default)]
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
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlmConfig {
    /// `mock` 使用本地回显（零配置联调）；`http` 调用真实 OpenAI 兼容接口。
    #[serde(default = "default_llm_backend")]
    pub backend: String,
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
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            listen: default_listen(),
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
            backend: default_asr_backend(),
            model: String::new(),
            tokens: String::new(),
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
            model: String::new(),
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
            model: String::new(),
            voices: String::new(),
            tokens: String::new(),
            data_dir: String::new(),
            dict_dir: String::new(),
            lexicon: String::new(),
            lang: default_tts_lang(),
            speaker: 0,
            speed: default_speed(),
            num_threads: default_tts_threads(),
        }
    }
}
impl Default for LlmConfig {
    fn default() -> Self {
        LlmConfig {
            backend: default_llm_backend(),
            api_base: default_api_base(),
            api_key: String::new(),
            model: default_llm_model(),
            system_prompt: default_system_prompt(),
            max_history: default_max_history(),
            temperature: default_temperature(),
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

    pub fn asr_is_mock(&self) -> bool {
        self.asr.backend.eq_ignore_ascii_case("mock")
    }

    pub fn tts_is_mock(&self) -> bool {
        self.tts.backend.eq_ignore_ascii_case("mock")
    }
}

// ---- 默认值辅助函数 ----
fn default_listen() -> String {
    "0.0.0.0:8000".into()
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
fn default_asr_backend() -> String {
    "mock".into()
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
/// TTS 合成线程数默认 1：与 ASR/VAD 错峰，避免 ASR+TTS 峰值占满 4 核。
fn default_tts_threads() -> u32 {
    1
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
    "mock".into()
}
fn default_speed() -> f32 {
    1.0
}
fn default_api_base() -> String {
    "https://api.example.com/v1/chat/completions".into()
}
fn default_llm_backend() -> String {
    "mock".into()
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

impl Default for Config {
    fn default() -> Self {
        Config {
            server: ServerConfig {
                listen: default_listen(),
                expected_token: String::new(),
                worker_threads: default_worker_threads(),
                admin_dir: default_admin_dir(),
            },
            audio: AudioConfig {
                downlink_sample_rate: default_downlink_sr(),
                downlink_frame_duration_ms: default_frame_ms(),
                channels: default_channels(),
                binary_protocol_version: default_bin_ver(),
            },
            asr: AsrConfig {
                backend: default_asr_backend(),
                model: String::new(),
                tokens: String::new(),
                language: default_language(),
                use_itn: default_true(),
                num_threads: default_num_threads(),
                provider: default_provider(),
            },
            vad: VadConfig {
                model: String::new(),
                threshold: default_threshold(),
                min_silence_duration: default_min_silence(),
                min_speech_duration: default_min_speech(),
            },
            tts: TtsConfig {
                backend: default_tts_backend(),
                model: String::new(),
                voices: String::new(),
                tokens: String::new(),
                data_dir: String::new(),
                dict_dir: String::new(),
                lexicon: String::new(),
                lang: default_tts_lang(),
                speaker: 0,
                speed: default_speed(),
                num_threads: default_num_threads(),
            },
            llm: LlmConfig {
                backend: default_llm_backend(),
                api_base: default_api_base(),
                api_key: String::new(),
                model: default_llm_model(),
                system_prompt: default_system_prompt(),
                max_history: default_max_history(),
                temperature: default_temperature(),
            },
        }
    }
}
