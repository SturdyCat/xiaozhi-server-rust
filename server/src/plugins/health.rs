//! 引擎健康探测：capability 级的运行期轻量检查（P1-6）。
//!
//! ## 定位与纪律
//!
//! [`PluginHost::plan`] 的 `validate` 是**纯配置校验**（无 I/O，因此可在无 sherpa feature
//! 的 `cargo test` 里全量断言状态逻辑）。但"配置写对了"不等于"机器上真有那个文件"——
//! 模型文件被删、挂载卷只读这类**运行期**问题，plan 看不见，部署者只能翻启动日志。
//! 本模块补上这一层：按 capability 做本地文件系统探测，产出 [`ProbeReport`]。
//!
//! ⚠️ **硬纪律（PRD P1-6 验收 6 的结构保证）**：探测只允许本地 fs 检查
//! （`metadata` / 创建临时文件后立即删除）；**禁止网络请求、禁止引擎构建**。
//! 好处：探测幂等（重复调用无累积副作用）、<1ms 量级、断网环境行为不变。
//! 任何"顺手"加一个 HTTP 探活的改动都违反本模块的边界——网络探测属配置级
//! （凭据/上游连通），不在引擎健康自检的范围。
//!
//! ## 为什么是独立模块而不是 `PluginDescriptor.healthcheck` 字段
//!
//! 1. 保住 `plan()` 的无 I/O 性质：描述符里任何允许 I/O 的钩子都会模糊
//!    "cargo test 全量断言状态逻辑"这条边界（默认 feature 下探测分支不跑，测试依然绿，
//!    上了真机才炸）；
//! 2. 探测是 capability 级而非 impl 级：11 个能力只有 4 个有探测项，tts 的探测只在
//!    kokoro 被选中时才有意义——这是"当前配置"的函数，不是描述符的静态属性；
//! 3. capability 级 `match` 天然穷尽：[`Capability`] 是闭枚举，漏一个变体编译不过，
//!    这就是免费的漂移护栏（新增能力时编译器强制补探测项），不靠测试兜底。
//!
//! ## 探测项一览
//!
//! | Capability | 探测内容 | 失败含义 |
//! |---|---|---|
//! | Asr | `cfg.asr.model` 文件存在 | 必需能力缺模型 → Failed（与启动失败一致） |
//! | Vad | `cfg.vad.model` 文件存在 | 同上 |
//! | Tts | 仅 engine=kokoro：model/voices/tokens/data_dir 四路径存在 | 同上 |
//! | Memory | db_path 父目录存在且可写（临时文件即删；`:memory:` 直接 Ok） | 可选能力 → Degraded |
//! | 其余 7 个 | 无本地探测（凭据/上游连通属配置级，被验收 6 禁止） | — |

use std::path::Path;

use crate::config::Config;
use crate::plugins::registry::Capability;

/// 探测结论（第一版两档；`Warn` 预留给"能跑但受限"的场景，如磁盘水位）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProbeState {
    /// 探测通过。
    Ok,
    /// 探测失败：`detail` 必填，含「怎么修」中文指引。
    Fail,
}

impl ProbeState {
    pub const fn as_str(self) -> &'static str {
        match self {
            ProbeState::Ok => "ok",
            ProbeState::Fail => "fail",
        }
    }
}

/// 一次探测的结论。
#[derive(Clone, Debug)]
pub struct ProbeReport {
    pub state: ProbeState,
    /// 失败时的修复指引（含「怎么修」）；成功时为 `None`。
    pub detail: Option<String>,
}

impl ProbeReport {
    fn ok() -> Self {
        Self { state: ProbeState::Ok, detail: None }
    }

    fn fail(detail: String) -> Self {
        Self { state: ProbeState::Fail, detail: Some(detail) }
    }

    /// JSON 投影（`GET /api/health/engines` 的 `probe` 字段）。
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "state": self.state.as_str(),
            "detail": self.detail,
        })
    }
}

/// 对某能力做运行期探测；`None` = 该能力无需探测（或未启用——探测只在 enabled 时跑，
/// 关掉的能力谈健康没有意义，PRD 验收 2：显式关闭不是错误）。
///
/// ⚠️ 纪律（见模块注释）：只做本地 fs 检查，禁止网络、禁止引擎构建。
pub fn probe(cfg: &Config, cap: Capability) -> Option<ProbeReport> {
    match cap {
        Capability::Asr => Some(probe_file_exists(&cfg.asr.model, "[asr].model")),
        Capability::Vad => Some(probe_file_exists(&cfg.vad.model, "[vad].model")),
        Capability::Tts => Some(probe_tts(cfg)),
        Capability::Memory => Some(probe_memory_dir(&cfg.memory.db_path)),
        // 无本地探测项：凭据/上游连通属配置级（网络探测被 PRD 验收 6 禁止）。
        // match 全量穷尽是刻意的编译期护栏——新增 Capability 变体时这里必须补一行。
        Capability::Llm
        | Capability::FullChain
        | Capability::Voices
        | Capability::Firmware
        | Capability::Soul
        | Capability::Command
        | Capability::Wake
        | Capability::Nlu => None,
    }
}

/// 单个文件存在性探测（ASR 模型 / VAD 模型共用；口径对齐 `impls.rs::validate_asr` 的文案风格）。
fn probe_file_exists(path: &str, field: &str) -> ProbeReport {
    if path.trim().is_empty() {
        // 空路径属**配置级**问题（validate 已拦），这里不重复报错，按 Ok 放行
        // ——phase 合并时 plan 的 Failed/Degraded 自然胜出，不会掩盖。
        return ProbeReport::ok();
    }
    if Path::new(path.trim()).is_file() {
        ProbeReport::ok()
    } else {
        ProbeReport::fail(format!(
            "{field} 指向的文件不存在：{path}。请检查挂载卷；容器内缺失模型时入口脚本会自动下载，\
             离线环境请先预置模型文件后重启。"
        ))
    }
}

/// TTS 探测：只在 kokoro（本地离线引擎）被选中时才有意义——远程服务（xfyun）的
/// "健康"取决于凭据与上游连通，属配置级，网络探测被验收 6 禁止。
/// 四个路径对齐 `impls.rs::validate_tts_kokoro` 的必要路径集合。
fn probe_tts(cfg: &Config) -> ProbeReport {
    if cfg.tts.engine_id() != "kokoro" {
        return ProbeReport::ok();
    }
    let k = &cfg.tts.kokoro;
    let missing: Vec<(&str, &str)> = [
        ("model", k.model.as_str()),
        ("voices", k.voices.as_str()),
        ("tokens", k.tokens.as_str()),
        ("data_dir", k.data_dir.as_str()),
    ]
    .into_iter()
    .filter(|(_, v)| v.trim().is_empty() || !Path::new(v.trim()).exists())
    .collect();
    if missing.is_empty() {
        ProbeReport::ok()
    } else {
        let list = missing
            .iter()
            .map(|(n, v)| format!("{n}={v}"))
            .collect::<Vec<_>>()
            .join("、");
        ProbeReport::fail(format!(
            "[tts.kokoro] 以下路径在磁盘上不存在：{list}。请检查挂载卷；\
             容器内缺失模型时入口脚本会自动下载，离线环境请先预置 Kokoro 模型包（默认 \
             /data/models/Kokoro/…）后重启，或把 [tts].engine 切到在线服务商（如 xfyun）。"
        ))
    }
}

/// 记忆探测：db_path 父目录存在且可写。
///
/// 可写性用"创建 `.health-probe` 临时文件 → 立即删除"验证（`metadata` 区分不了
/// "目录存在但只读"）。SQLite 库文件本身不存在是**正常**情况（首次启动会创建），
/// 但父目录不可写意味着记忆必然写不进——那正是 PRD 验收 3 的场景。
/// `:memory:`（内存库）直接放行。
fn probe_memory_dir(db_path: &str) -> ProbeReport {
    let p = db_path.trim();
    if p.is_empty() || p == ":memory:" {
        return ProbeReport::ok();
    }
    let Some(parent) = Path::new(p).parent() else {
        return ProbeReport::ok();
    };
    // 父目录为空（相对路径紧贴根）视为当前目录，无需特判：`.` 可写即放行。
    if !parent.is_dir() {
        return ProbeReport::fail(format!(
            "[memory].db_path 所在目录不存在：{}。请先创建目录（容器部署建议放挂载卷，\
             如 docker-compose 的 ./server-data 对应的 /data）。",
            parent.display()
        ));
    }
    let probe_file = parent.join(".health-probe");
    let writable = std::fs::write(&probe_file, b"").is_ok();
    // 无论成败都清掉临时文件（探测幂等：重复调用不残留）。
    let _ = std::fs::remove_file(&probe_file);
    if writable {
        ProbeReport::ok()
    } else {
        ProbeReport::fail(format!(
            "[memory].db_path 所在目录不可写：{}。请确认挂载卷权限（docker-compose 的 \
             ./server-data 需对容器内用户可写），或把 db_path 改到可写目录。",
            parent.display()
        ))
    }
}

#[cfg(test)]
mod tests;
