//! 服务配置：从 TOML 文件加载（`Config::load`），默认值即生产路径。
//!
//! ## 引擎与默认值
//!
//! - **无 mock**：ASR 恒为 SenseVoice（sherpa-onnx）、TTS 恒为 Kokoro（sherpa-onnx）、
//!   LLM 恒为 OpenAI 兼容 HTTP（Responses API）。`--features sherpa` 是运行真实引擎的
//!   前提，未启用时引擎构建直接报错（不再有 mock 回退）。
//! - 默认（无 `--config`）即 [`Config::default()`]：模型路径指向 `/data/models/...`
//!   （与 `config.example.toml`、容器挂载一致），LLM 需填 `api_base`/`api_key`。
//! - 旧配置文件里残留的 `backend = "..."` / `[tts].model` 等旧位置写法会被
//!   [`Config::normalize`] 读懂并搬到规范位置（管理页保存一次即写成新 schema）；
//!   真正未知的键仍被 serde 静默忽略（无 `deny_unknown_fields`），无需手工清理。
//!
//! ## 加载优先级与持久化
//!
//! 配置从哪来、能否被管理页写回，由 `main.rs` 的 `load_config` 决定
//!（`XIAOZHI_CONFIG` → `--config` → 内置默认）；`GET/PUT /api/config` 的读写语义
//! 见 `app/ws.rs`。本文件只负责结构与默认值。
//!
//! ## 配置约定（P4，见 `docs/plugin-architecture-unification.md` §3.3）
//!
//! 1. **`engine` 键**：每个能力段用 `engine = "<实现 id>"` 选择实现，id 即注册表描述符 id
//!    去掉 `"<cap>."` 前缀（`[tts] engine="kokoro"` ↔ 描述符 `tts.kokoro`）；
//! 2. **私有段**：实现私有字段放 `[<cap>.<id>]`（`[tts.kokoro]` / `[tts.xfyun]` / `[memory.extractor]`），
//!    本体只留**跨实现通用项**（如 `[tts].cache_entries`）；
//! 3. **`normalize()` 双读**：旧写法（`[tts].model` 直接放本体、`backend = "sherpa"`）仍可读，
//!    加载时搬到规范位置；保存只写新结构。**未知键继续静默忽略**（无 `deny_unknown_fields`）。
//!
//! 约定由 [`EngineSpec`] 表 + [`canonical_engine`] 唯一实现；注册表侧有漂移护栏测试
//! 断言"每个声明的 engine id 都必须有对应描述符"。

use serde::{Deserialize, Serialize};
use std::path::Path;

// 引擎子配置段随插件走（定义在各插件 mod.rs），此处转发以维持 `crate::config::*` 路径稳定：
pub use crate::plugins::aiui::AiuiConfig;
pub use crate::plugins::asr::AsrConfig;
pub use crate::plugins::command::CommandConfig;
pub use crate::plugins::llm::LlmConfig;
pub use crate::plugins::memory::MemoryConfig;
pub use crate::plugins::soul::SoulConfig;
pub use crate::plugins::tts::TtsConfig;
pub use crate::plugins::vad::VadConfig;
pub use crate::plugins::wakeup::WakeConfig;

/// 每设备配置 overlay（白名单合并 / 落盘路径，见 `config/overlay.rs`）。
pub mod overlay;

/// 配置层「部分更新」合并（JSON 递归合并）。
///
/// 语义：
/// - 基线与补丁都是对象时**递归**合并：只覆盖补丁里出现的键；
/// - 补丁里**显式 `null`** = 清除该键 → 反序列化回落 serde 默认值；
/// - 任一端不是对象（如 `"tts": 5`）→ 整体替换，随后由反序列化报出精确类型错误。
///
/// ⚠️ 必须在**解析成 `Config` 之前**于 JSON 层完成：`Config` 每个字段都带 serde default，
/// 先解析就无法区分"客户端发了默认值"与"客户端根本没发"。
///
/// `app::ws::config::merge_client_patch`（PUT /api/config 管线）与 `config::overlay`
/// （设备 overlay 合并）共用本函数，保证「出现即覆盖 / 显式 null 即清除」语义一致。
pub(crate) fn merge_patch(base: &mut serde_json::Value, patch: &serde_json::Value) {
    match (base.as_object_mut(), patch) {
        (Some(base_obj), serde_json::Value::Object(patch_obj)) => {
            for (k, v) in patch_obj {
                if v.is_null() {
                    base_obj.remove(k); // 清除 → 反序列化取默认值
                } else if let Some(slot) = base_obj.get_mut(k) {
                    merge_patch(slot, v);
                } else {
                    base_obj.insert(k.clone(), v.clone());
                }
            }
        }
        // 基线或补丁不是对象（如 `"tts": 5`）→ 整体替换，随后由反序列化报出精确类型错误
        _ => *base = patch.clone(),
    }
}

/// 一个能力**可选实现**的声明（配置层权威，不是注册表）。
///
/// `id` = 实现 id（= 注册表描述符 id 去掉 `"<cap>."` 前缀）；`aliases` = 历史写法（只读兼容，
/// 例：TTS 的 `sherpa` → `kokoro`）。
pub struct EngineSpec {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
}

impl EngineSpec {
    /// 该值（已小写化）是否命中本实现（id 或别名）。
    fn matches(&self, low: &str) -> bool {
        self.id == low || self.aliases.iter().any(|a| *a == low)
    }
}

/// `engine` 键规范化（**唯一权威**，各能力配置都走它）：
///
/// - 空 → 默认实现 id（未指定 engine 的旧配置照常工作）；
/// - id / 别名（大小写不敏感）→ 规范 id（`"sherpa"` → `"kokoro"`）；
/// - **未知值原样小写返回**，交由各能力 `validate` 报错——静默回退默认会让用户
///   以为"配置生效了"，这是本仓库最忌讳的一类故障（见 `docs/…unification.md` §7）。
pub fn canonical_engine(raw: &str, specs: &[EngineSpec], default_id: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return default_id.to_string();
    }
    let low = t.to_ascii_lowercase();
    for s in specs {
        if s.matches(&low) {
            return s.id.to_string();
        }
    }
    low
}

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
    /// AIUI 全链路（极速超拟人）模式：启用后设备流水线变为
    /// VAD 切段 → AIUI 交互 API（ASR+大模型+TTS 云端闭环）→ 下行音频，
    /// 本地 ASR / LLM / TTS 引擎闲置（可随时切回）。
    #[serde(default)]
    pub aiui: AiuiConfig,
    /// 灵魂（人格档案）：`[soul].enabled=false` 时完全退回 `[llm].system_prompt`。
    #[serde(default)]
    pub soul: SoulConfig,
    /// 记忆（图记忆）：默认关闭，关闭时零依赖、行为与今天一致。
    #[serde(default)]
    pub memory: MemoryConfig,
    /// 语音指令闸门（`[command]`）：ASR → LLM **之前**拦截「退下/闭嘴/关闭」这类指令，
    /// 命中即说一句告别语，再用 MCP 让设备结束聆听（设备不支持 MCP 时断开本次会话），
    /// 跳过 LLM 与正常回复。默认关闭。
    #[serde(default)]
    pub command: CommandConfig,
    /// 唤醒问候语（`[wakeup]`）：设备唤醒（固件上报 `listen state=detect`）时**立刻**说一句
    /// 预制话术（默认「我在」），跳过 LLM 与正常回复；成品音频按音色持久化复用，
    /// 稳态下每次唤醒零 LLM、零 TTS。默认关闭。
    #[serde(default)]
    pub wakeup: WakeConfig,
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
    /// 下行抖动缓冲（提前量，毫秒）：下发时间表整体比实时**提前** lead_ms，
    /// 使设备解码队列常备约 lead_ms 的音频存货，吸收网络/调度抖动**以及上游
    /// LLM 出句/TTS 合成的停顿**（长回复"前面流畅、后面断续"的根因，见 `app/prefetch.rs`）。
    ///
    /// 默认 1000ms：逼近设备侧硬上限（见 [`AudioConfig::downlink_queue_packets`]）。
    /// 设备缓冲是"后续断续"的唯一解药，而 ESP 固件 20 包队列（60ms 帧）只装得下 ~1.1s；
    /// 想要更多必须先改固件。安全上限 = `(队列包数 − 2) × 帧长`，超限由
    /// [`AudioConfig::normalize`] 收敛并告警（开局预充超队容量 = 固件静默丢包 = 回复开头少一截）。
    /// ⚠️ 代价（固件实测语义）：`tts stop` 不清队列，设备会把已缓存的 ~lead 音频播完才
    /// 进入聆听 → 提前量越大，**打断后的"停口"越晚**（240ms ≈ 一个字，1000ms ≈ 三个字）。
    #[serde(default = "default_downlink_lead_ms")]
    pub downlink_lead_ms: u32,
    /// 设备解码队列**包数**：必须与固件一致
    ///（`main/audio/audio_service.h` 的 `MAX_DECODE_PACKETS_IN_QUEUE`；官方默认 20 包）。
    ///
    /// ⚠️ 单位是**包**不是毫秒：队列能装多少音频取决于协商帧长（`20 × 帧长`，60ms 帧
    /// → 1200ms；40ms 帧 → 800ms）。服务端据此推出 [`Self::max_lead_ms`]。
    ///
    /// 想要"更多缓存"时它是唯一开关：① 改固件把队列包数放大（如 `4000/OPUS_FRAME_DURATION_MS`
    /// = 66 包）→ ② 重刷设备 → ③ 本项改为 66 → ④ 调大 `downlink_lead_ms`。
    /// 只改服务端不改固件无效：超出队容量的下行帧会被固件**静默丢弃**（零日志）。
    #[serde(default = "default_downlink_queue_packets")]
    pub downlink_queue_packets: u32,
    /// **服务器侧下行水库容量**（毫秒）：把已编码的 Opus 帧在服务器内存里多备几秒，
    /// 发送端从水库取（而不是"现合成、现发送"）。这是设备队列（~1.2s 硬上限，见
    /// [`Self::downlink_queue_packets`]）之外**唯一**能继续加深缓冲的地方——
    /// 用服务器内存换下行连续性。
    ///
    /// 为什么需要：上游（LLM 出句 / 在线 TTS 首片）只要持续慢于实时，设备那 ~1s 存货
    /// 必被抽干 → 长回复"从中间某句起断断续续"。水库把"已合成好的音频"提前备好，
    /// 发送端就能在**上游停顿**时继续按实时节奏供数；听感连续的前提是
    /// `停顿 ≤ 水库水位 + 设备队列`。
    ///
    /// 内存口径：水库里是**已编码帧**（约 2KB/s），30s ≈ 60KB/会话，代价可忽略；
    /// 真正的代价是**起跑延迟**（见 [`Self::downlink_buffer_start_ms`]）。
    /// 0 = 关闭水库缓冲（退化为逐帧直通，仅保留节流）。
    ///
    /// ⚠️ 水库只能"备住**已经合成出来**的音频"。上游**平均**慢于实时（如 LLM 逐字吐出
    /// 只有 0.6× 实时）时，缺口是**累积**的，要整轮不断音就得先把累计缺口攒出来：
    /// `所需缓冲 ≈ (1/生产速率 − 1) × 音频总长`（0.64× 生产 + 80s 音频 → 约 45s）。
    /// 因此 30s 默认值解决的是"上游**间歇**停顿"，不是"上游**持续**慢"——后者只有
    /// 提高 `[tts].speed`（音频变短）或换更快的 LLM/TTS 才是根治（见每轮吞吐日志）。
    #[serde(default = "default_downlink_buffer_ms")]
    pub downlink_buffer_ms: u32,
    /// **起跑闸**（毫秒）：本轮首帧下发前，先等水库攒够这么多音频再开播。
    ///
    /// 上游略慢于实时且立刻开播时，缺口会被摊成一整轮的断续（听感最差）；先攒一段再播，
    /// 缺口由水库吸收，代价是一次性起跑等待。生产端已收工（短回复）时闸门立刻放行，
    /// 不会白等。0 = 关闭（首帧一到就发，TTFA 最小）。
    /// 上限受 [`Self::downlink_buffer_ms`] 约束（攒不出比容量更多的音频）。
    #[serde(default = "default_downlink_buffer_start_ms")]
    pub downlink_buffer_start_ms: u32,
    /// 起跑闸**最长等待**（毫秒）：生产端持续慢于实时时不把首帧无限期拖住。
    /// 到点即以当前水位起跑（日志会写明"上游供数慢于实时"）。
    #[serde(default = "default_downlink_buffer_start_max_ms")]
    pub downlink_buffer_start_max_ms: u32,
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

impl AudioConfig {
    /// 下行抖动缓冲**安全上限**（毫秒）。
    ///
    /// 固件解码队列是 **N 包**（不是固定毫秒）：一包装 `帧长` 毫秒音频。节拍器开局预充
    /// `ceil(lead/帧长)+1` 帧（见 `app/downlink.rs`），队满即**静默丢包**（固件返回 false、
    /// 零日志）。故安全上限 = `(包数 − 2) × 帧长`——
    /// 60ms 帧 → `18 × 60 = 1080ms`；40ms 帧 → `18 × 40 = 720ms`（**不能**按 1200ms 算！）。
    pub fn max_lead_ms(&self) -> u32 {
        self.downlink_queue_packets
            .saturating_sub(2)
            .saturating_mul(self.frame_ms())
    }

    /// 协商帧长（毫秒），恒 ≥1：0 会让节拍器"零等待"退化成尽速灌入（见 `normalize`）。
    pub fn frame_ms(&self) -> u32 {
        self.downlink_frame_duration_ms.max(1)
    }

    /// **下载缓冲预算**（毫秒）：TTS 流下载在积压达到它时暂停（见
    /// `app::prefetch::DownloadBudget`），消费（编码入水库）释放额度后恢复。
    ///
    /// 为什么由水库容量**派生**而不是独立开关：两者的"多秒"是同一个运维意图
    ///（用服务器内存换下行连续），分两个旋钮只会让人调错；而两者又**不能等深**——
    /// 水库里是 Opus 帧（~2KB/s，30s ≈ 60KB），下载缓冲里是 PCM f32（~64KB/s @16kHz，
    /// 是前者的 ~30 倍），等深会把每会话内存从几十 KB 抬到几 MB。
    ///
    /// 故夹在 `[1s, 10s]`：下限保证"下载"与"编码入水库"真的解耦（上游一片一片地等消费
    /// 会把流式下载退化成逐片节拍），上限保证每会话 PCM 内存 ≤ ~640KB。
    /// 10s 的下载缓冲 + 多秒水库 = 服务器侧总领先量，见 [`Self::downlink_buffer_ms`] 文档。
    pub fn download_budget_ms(&self) -> u32 {
        self.downlink_buffer_ms.clamp(DOWNLOAD_BUDGET_MIN_MS, DOWNLOAD_BUDGET_MAX_MS)
    }

    /// 配置规范化：
    /// ① 帧长非法（0）→ 回落到固件默认 60ms（否则节流失效、`2×帧长` 也会算错）；
    /// ② 把超限的 `downlink_lead_ms` 收敛到安全上限并告警；
    /// ③ 水库三参数自洽（容量上限、关闭水库则关起跑闸、闸门不超过容量、最长等待 ≥ 闸门）。
    ///
    /// "调得越大越稳"在这里**不成立**：超过上限后开局预充本身就超队容量，会被固件静默丢弃
    /// ——表现为"回复开头少一截"，比抖动更难排查。故此处硬收敛（对齐 `session::run` 里
    /// `drain_ms.clamp(100, 1000)` 的护栏风格）。
    pub fn normalize(&mut self) {
        if self.downlink_frame_duration_ms == 0 {
            tracing::warn!(
                "[audio].downlink_frame_duration_ms=0 非法（节拍器会退化成零等待尽速灌入，\
                 必然撑爆设备解码队列），已回落到默认 {}ms",
                default_frame_ms()
            );
            self.downlink_frame_duration_ms = default_frame_ms();
        }
        let cap = self.max_lead_ms();
        if self.downlink_lead_ms > cap {
            tracing::warn!(
                "[audio].downlink_lead_ms={} 超过安全上限 {cap}ms（设备解码队列 {} 包 × {}ms 帧；\
                 开局预充 ceil(lead/帧长)+1 帧，超限即静默丢包），已收敛到 {cap}ms。\
                 想要更大缓冲请先放大固件 MAX_DECODE_PACKETS_IN_QUEUE，再同步 [audio].downlink_queue_packets",
                self.downlink_lead_ms,
                self.downlink_queue_packets,
                self.frame_ms(),
            );
            self.downlink_lead_ms = cap;
        }
        self.normalize_reservoir();
    }

    /// 服务器侧水库参数自洽（`downlink_buffer_*`）。
    ///
    /// 三条不变式：① 容量有上限（配置手滑写个 10^9 也不会把内存吃光——水库容量就是
    /// 每会话的缓冲上限）；② 水库关闭（0）时起跑闸必然无效，一并归零；
    /// ③ 起跑闸 ≤ 容量、最长等待 ≥ 起跑闸（否则语义互相矛盾）。
    fn normalize_reservoir(&mut self) {
        if self.downlink_buffer_ms > MAX_BUFFER_MS {
            tracing::warn!(
                "[audio].downlink_buffer_ms={} 超过上限 {MAX_BUFFER_MS}ms，已收敛（水库是每会话内存，\
                 {MAX_BUFFER_MS}ms 已远超任何上游停顿的合理吸收范围）",
                self.downlink_buffer_ms
            );
            self.downlink_buffer_ms = MAX_BUFFER_MS;
        }
        if self.downlink_buffer_ms == 0 {
            if self.downlink_buffer_start_ms > 0 {
                tracing::warn!(
                    "[audio].downlink_buffer_ms=0（水库关闭）但 downlink_buffer_start_ms={} > 0：\
                     没有水库可攒，起跑闸一并归零",
                    self.downlink_buffer_start_ms
                );
            }
            self.downlink_buffer_start_ms = 0;
            self.downlink_buffer_start_max_ms = 0;
            return;
        }
        if self.downlink_buffer_start_ms > self.downlink_buffer_ms {
            tracing::warn!(
                "[audio].downlink_buffer_start_ms={} 超过水库容量 {}ms，已收敛到容量（攒不出更多）",
                self.downlink_buffer_start_ms,
                self.downlink_buffer_ms
            );
            self.downlink_buffer_start_ms = self.downlink_buffer_ms;
        }
        if self.downlink_buffer_start_max_ms < self.downlink_buffer_start_ms {
            self.downlink_buffer_start_max_ms = self.downlink_buffer_start_ms;
        }
    }
}

/// 服务器侧水库容量上限（毫秒）：水库是每会话内存，180s 已远超任何上游停顿的合理吸收范围
/// （且 180s × ~2KB/s ≈ 360KB/会话，仍然很便宜）。
const MAX_BUFFER_MS: u32 = 180_000;
/// 下载缓冲（PCM）预算的下限/上限（毫秒），见 [`AudioConfig::download_budget_ms`]。
const DOWNLOAD_BUDGET_MIN_MS: u32 = 1_000;
const DOWNLOAD_BUDGET_MAX_MS: u32 = 10_000;

/// 官方固件的解码队列容量（**包数**）：ESP `AudioService::MAX_DECODE_PACKETS_IN_QUEUE`
/// = `1200 / OPUS_FRAME_DURATION_MS`，`OPUS_FRAME_DURATION_MS = 60` → 20 包。
/// 仅作 [`AudioConfig`] 的默认值；实测值以 `[audio].downlink_queue_packets` 为准
///（自刷固件放大队列后要同步改它）。
pub const FIRMWARE_DECODE_QUEUE_PACKETS: u32 = 20;

impl Default for AudioConfig {
    fn default() -> Self {
        AudioConfig {
            downlink_sample_rate: default_downlink_sr(),
            downlink_frame_duration_ms: default_frame_ms(),
            channels: default_channels(),
            binary_protocol_version: default_bin_ver(),
            downlink_lead_ms: default_downlink_lead_ms(),
            downlink_queue_packets: default_downlink_queue_packets(),
            downlink_buffer_ms: default_downlink_buffer_ms(),
            downlink_buffer_start_ms: default_downlink_buffer_start_ms(),
            downlink_buffer_start_max_ms: default_downlink_buffer_start_max_ms(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        // 生产默认（模型路径指向 /data/models/...）：先构造再规范化，
        // 使内存默认配置与"从空 TOML 加载"得到**同一形状**（engine 键已填、旧字段已搬）。
        let mut c = Config {
            server: ServerConfig::default(),
            audio: AudioConfig::default(),
            asr: AsrConfig::default(),
            vad: VadConfig::default(),
            tts: TtsConfig::default(),
            llm: LlmConfig::default(),
            aiui: AiuiConfig::default(),
            soul: SoulConfig::default(),
            memory: MemoryConfig::default(),
            command: CommandConfig::default(),
            wakeup: WakeConfig::default(),
        };
        c.normalize();
        c
    }
}

impl Config {
    /// 读取并解析 TOML 配置文件。
    pub fn load(path: &str) -> anyhow::Result<Config> {
        let text = std::fs::read_to_string(Path::new(path))
            .map_err(|e| anyhow::anyhow!("读取配置文件 {path} 失败: {e}"))?;
        let mut cfg: Config = toml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("解析配置文件 {path} 失败: {e}"))?;
        // 旧写法（[tts].model / backend / [memory].backend…）在此搬到规范位置（幂等）。
        cfg.normalize();
        Ok(cfg)
    }

    /// **配置规范化**（P4）：`engine` 键补全 + 旧位置字段搬到 `[<cap>.<id>]` 私有段。
    ///
    /// 幂等，且**只在配置入口调用**（[`Self::load`] / [`Self::default`] /
    /// `app::ws::config::merge_client_patch`），因此：
    /// - 运行期读到的一定是规范形状（GET /api/config、`signature`、`build` 都能直接取字段）；
    /// - 保存时只写新结构（旧字段标了 `skip_serializing`），文件随一次保存完成迁移。
    pub fn normalize(&mut self) {
        self.audio.normalize();
        self.asr.normalize();
        self.vad.normalize();
        self.tts.normalize();
        self.llm.normalize();
        self.memory.normalize();
        self.soul.normalize();
        self.command.normalize();
        self.wakeup.normalize();
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
fn default_downlink_lead_ms() -> u32 {
    // 逼近设备侧上限（1200−2×60=1080）：缓冲越深，长回复越抗上游停顿。
    1000
}
/// 设备解码队列包数默认值 = 官方固件 `MAX_DECODE_PACKETS_IN_QUEUE`（20 包）。
/// 自刷固件放大队列后必须同步本值（见 `AudioConfig::downlink_queue_packets` 文档）。
fn default_downlink_queue_packets() -> u32 {
    FIRMWARE_DECODE_QUEUE_PACKETS
}
/// 服务器侧水库容量默认 30s：比设备队列（~1.2s）深一个数量级，足以吸收 LLM 逐句吐字
/// 与在线合成首片的常见停顿；内存代价 ~60KB/会话（存的是已编码帧）。
/// 30s 是"上游**间歇**停顿"的解法；上游**持续**慢于实时属于吞吐问题，见
/// [`AudioConfig::downlink_buffer_ms`] 文档（要根治得提速上游，不是加缓冲）。
fn default_downlink_buffer_ms() -> u32 {
    30_000
}
/// 起跑闸默认 1500ms：约 4~5 个汉字的量。合成 ~7 倍速时几百 ms 就能攒到；
/// 短回复（生产端已收工）立即放行，不白等。
fn default_downlink_buffer_start_ms() -> u32 {
    1_500
}
/// 起跑闸最长等待默认 4000ms：上游持续慢于实时时不至于把首帧无限期拖住。
fn default_downlink_buffer_start_max_ms() -> u32 {
    4_000
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

#[cfg(test)]
mod tests;

