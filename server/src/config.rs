//! 服务配置：从 TOML 文件加载（`Config::load`），默认值即生产路径。
//!
//! ## 引擎与默认值
//!
//! - **无 mock**：ASR 恒为 SenseVoice（sherpa-onnx）、TTS 恒为 Kokoro（sherpa-onnx）、
//!   LLM 恒为 OpenAI 兼容 HTTP（Responses API）。`--features sherpa` 是运行真实引擎的
//!   前提，未启用时引擎构建直接报错（不再有 mock 回退）。
//! - 默认（无 `--config`）即 [`Config::default()`]：模型路径指向 `/data/models/...`
//!   （与 `config.example.toml`、容器挂载一致），LLM 需填 `api_base`/`api_key`。
//! - 旧配置文件里残留的 `backend = "..."` 键会被 serde 静默忽略（无 `deny_unknown_fields`），
//!   无需手工清理；管理页保存一次即写成新 schema。
//!
//! ## 加载优先级与持久化
//!
//! 配置从哪来、能否被管理页写回，由 `main.rs` 的 `load_config` 决定
//!（`XIAOZHI_CONFIG` → `--config` → 内置默认）；`GET/PUT /api/config` 的读写语义
//! 见 `app/ws.rs`。本文件只负责结构与默认值。

use serde::{Deserialize, Serialize};
use std::path::Path;

// 引擎子配置段随插件走（定义在各插件 mod.rs），此处转发以维持 `crate::config::*` 路径稳定：
pub use crate::plugins::aiui::AiuiConfig;
pub use crate::plugins::asr::AsrConfig;
pub use crate::plugins::llm::LlmConfig;
pub use crate::plugins::tts::{TtsBackendKind, TtsConfig};
// 对外 API 面保留（bin crate 内暂无直接引用者）
#[allow(unused_imports)]
pub use crate::plugins::tts::XfyunTtsConfig;
pub use crate::plugins::vad::VadConfig;

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
    /// AIUI 全链路（极速超拟人）模式：启用后设备流水线变为
    /// VAD 切段 → AIUI 交互 API（ASR+大模型+TTS 云端闭环）→ 下行音频，
    /// 本地 ASR / LLM / TTS 引擎闲置（可随时切回）。
    #[serde(default)]
    pub aiui: AiuiConfig,
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

