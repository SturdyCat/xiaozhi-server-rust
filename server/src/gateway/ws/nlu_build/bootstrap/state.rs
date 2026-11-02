//! 自举状态机与持久化：`<data>/nlu-bootstrap/state.json`（**不写 SQLite**）。
//!
//! 为什么单独一个文件：状态迁移与退避是"跨重启可解释"的唯一依据，
//! 混在编排逻辑里最容易写成"凭内存判断重试次数"，重启就丢。

use crate::plugins::nlu::NluConfig;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::super::validate_buildable;

/// 状态文件 schema；不兼容变更时递增，读到旧 schema 直接重置（宁可重试也不读错字段）。
///
/// 新增字段一律 `#[serde(default)]`（对本结构体是全局 `#[serde(default)]`），
/// 这样**不需要**升 schema：升了会让存量部署的状态被重置、白跑一次训练。
const SCHEMA: u32 = 1;
/// 最大尝试次数：超过就 `giving_up`（人工删 `state.json` 恢复），不做无限重试。
pub(super) const MAX_ATTEMPTS: u32 = 3;
/// 退避表按"**已经失败过几次**"取，语义是"下一次尝试还要等多久"：
/// 第 1 次尝试立即（attempts=0 → 0s），失败 1 次后等 10min，失败 2 次后等 60min。
/// 用 wall-clock 记在 state.json 里，所以**重启也不会立刻重试**。
const BACKOFF_SECS: [u64; 2] = [600, 3600];
/// "环境还没就绪"的重查间隔：不消耗 `attempts`，也不结束任务。
///
/// 与退避分开是因为语义不同：退避是"我们做错了，等一会儿再试"，
/// 这里是"环境没准备好，等它准备好"——重试次数不该被环境问题吃掉。
pub(super) const RECHECK_SECS: u64 = 600;

// ---------------------------------------------------------------------------
// 开关
// ---------------------------------------------------------------------------

/// `NLU_BOOTSTRAP` 的三种取值。`adopt`（缺省）过门禁即自动启用；`build-only` 只训不启用；
/// `off` 完全关闭。**未知值按 off 处理**：宁可不动，也不能因为拼错而悄悄自动启用模型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Adopt,
    BuildOnly,
    Off,
}

pub(crate) fn mode_from_env(raw: Option<&str>) -> Mode {
    match raw.map(str::trim) {
        None | Some("") | Some("adopt") => Mode::Adopt,
        Some("build-only") => Mode::BuildOnly,
        Some("off") => Mode::Off,
        Some(other) => {
            tracing::warn!(
                target: "nlu_bootstrap",
                "NLU_BOOTSTRAP={other:?} 不是 adopt/build-only/off 之一，按 off 处理（不自动训练/启用）"
            );
            Mode::Off
        }
    }
}

// ---------------------------------------------------------------------------
// 状态机与持久化
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Phase {
    Idle,
    Pending,
    Running,
    /// 构建成功但未启用（`build-only` 模式的终点）。
    Success,
    /// 已自动采用（永不再自动重跑）。
    Adopted,
    /// 门禁没过：候选保留可手测，但配置一个字节都没改。
    GatedFailed,
    /// **构建依据的配置已过期**：训练期间有人改了技能目录，标签序可能已错位。
    ///
    /// 这是终态而不是可重试失败：拿同一个 `expected_revision` 重试**永远不可能成功**
    /// （只有 `current == expected` 才不冲突），重试只是把失败延迟 0.6 秒。
    /// 也不能"labels 没变就照样采用"：模板与 action 可能同批被改，模型是拿旧模板训的。
    Stale,
    /// 可重试失败（按退避再来）。
    Failed,
    /// 次数用尽或不可恢复（无 docker）。
    GivingUp,
    /// 前置条件不满足（人为决定），本条不再自动重试。
    Skipped,
}

impl Phase {
    /// 终态：不再自动重跑（`success` 也算——候选已经在手，下一步交给人工）。
    ///
    /// `stale` 必须在里面：否则容器重启会拿同一份过期配置反复重训一版注定不能采用的模型。
    pub(super) fn blocks_retry(self) -> bool {
        matches!(
            self,
            Self::Success
                | Self::Adopted
                | Self::GatedFailed
                | Self::Stale
                | Self::GivingUp
                | Self::Skipped
        )
    }
}

/// 冒烟门禁的实测值摘要（给 UI 显示，**不是**准确率）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct SmokeSummary {
    /// 正例接受且判对。
    pub(super) ok: usize,
    /// 危险失败：接受但判错类。**必须为 0**。
    pub(super) bad_class: usize,
    /// 负例被拒识。
    pub(super) rejected: usize,
    /// 正例总数（UI 显示"命中 4/4"用这个）。
    pub(super) positives: usize,
    /// 负例总数（当前恒 3，让 UI 不必硬编码）。
    pub(super) negatives: usize,
}

/// 一条错判/误接受的明细：运维据此判断"是语料歧义还是模型退化"。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct MissDetail {
    pub(super) text: String,
    pub(super) expected: String,
    /// 模型原始输出标签。
    pub(super) got: String,
    /// `rejected_other` / `below_threshold` / `wrong_intent:<技能名>`。
    pub(super) reason: String,
    pub(super) confidence: f32,
}

impl Default for MissDetail {
    fn default() -> Self {
        Self {
            text: String::new(),
            expected: String::new(),
            got: String::new(),
            reason: String::new(),
            confidence: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct State {
    pub(super) schema: u32,
    pub(super) phase: Phase,
    pub(super) attempts: u32,
    /// 首次判定"缺产物"的时间；0 = 未记录。
    pub(super) first_seen_at: i64,
    pub(super) last_attempt_at: Option<i64>,
    pub(super) last_error_kind: Option<String>,
    pub(super) last_error: Option<String>,
    pub(super) last_build_id: Option<String>,
    pub(super) last_base_model: Option<String>,
    pub(super) adopted_at: Option<i64>,
    /// 冒烟实测值（门禁跑到才写）。
    pub(super) smoke: Option<SmokeSummary>,
    /// 错类接受明细，**封顶 5 条**；总数看 `smoke.bad_class`。
    pub(super) bad_class_detail: Vec<MissDetail>,
    /// 负例误接受明细，封顶 5 条。
    pub(super) rejected_detail: Vec<MissDetail>,
    /// 构建时依据的配置指纹（`artifact.source_revision`）。
    pub(super) build_source_revision: Option<String>,
    /// 采用时读到的当前指纹。两者不同即 `stale`，这就是"该不该重建"的全部依据。
    pub(super) current_revision: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            phase: Phase::Idle,
            attempts: 0,
            first_seen_at: 0,
            last_attempt_at: None,
            last_error_kind: None,
            last_error: None,
            last_build_id: None,
            last_base_model: None,
            adopted_at: None,
            smoke: None,
            bad_class_detail: Vec::new(),
            rejected_detail: Vec::new(),
            build_source_revision: None,
            current_revision: None,
        }
    }
}

pub(super) fn state_path(data: &Path) -> PathBuf {
    data.join("nlu-bootstrap").join("state.json")
}

/// 读状态：缺失/损坏/schema 不认识 → 回到 `idle` 并 warn。
/// 绝不因为一个坏文件而 panic：自举是"尽力而为"的后台任务，不能拖垮服务。
pub(super) fn load_state(path: &Path) -> State {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return State::default(),
        Err(e) => {
            tracing::warn!(target: "nlu_bootstrap", "读取 {} 失败，按 idle 处理: {e}", path.display());
            return State::default();
        }
    };
    match serde_json::from_str::<State>(&text) {
        Ok(st) if st.schema == SCHEMA => st,
        Ok(st) => {
            tracing::warn!(
                target: "nlu_bootstrap",
                "状态文件 schema={} 与当前 {SCHEMA} 不符，按 idle 重置", st.schema
            );
            State::default()
        }
        Err(e) => {
            tracing::warn!(target: "nlu_bootstrap", "状态文件解析失败，按 idle 处理: {e}");
            State::default()
        }
    }
}

/// 原子写：同目录 tmp → fsync → rename（再 fsync 目录）。
/// 崩溃可能发生在任意时刻，半截 JSON 会让状态机读回 idle 并重跑一次构建——
/// 那是可接受的代价；但"读到半个文件就当成 adopted"绝不可接受。
pub(super) fn save_state(path: &Path, st: &State) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("state.json 缺少父目录"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".state.json.tmp-{}", std::process::id()));
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&serde_json::to_vec_pretty(st)?)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    if let Ok(handle) = std::fs::File::open(dir) {
        let _ = handle.sync_all();
    }
    Ok(())
}

pub(super) fn epoch_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(super) fn backoff_secs(attempts: u32) -> u64 {
    if attempts == 0 {
        return 0;
    }
    let index = (attempts - 1) as usize;
    BACKOFF_SECS.get(index).copied().unwrap_or(3600)
}

/// 距离下次可重试还差多少秒（`None` = 现在就可以）。
pub(super) fn backoff_remaining(st: &State, now: i64) -> Option<u64> {
    let wait = backoff_secs(st.attempts) as i64;
    if wait == 0 {
        return None;
    }
    let last = st.last_attempt_at?;
    // 夹到 ≥0：时钟回拨（NTP/手动改表）不能让退避变得更长。
    let elapsed = (now - last).max(0);
    (elapsed < wait).then_some((wait - elapsed) as u64)
}

// ---------------------------------------------------------------------------
// 产物存在性
// ---------------------------------------------------------------------------

/// 三条模型路径的状态。
///
/// 把"没有"和"读不出"分开是因为处置完全不同：
/// - 没有 → 该训练；
/// - 读不出（权限/IO/卷还没挂好）→ 训练也会失败，且会把 `attempts` 白白烧掉。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ArtifactState {
    /// 路径为空 / 文件不存在 / labels 为空：真缺产物。
    Missing,
    /// 文件在但读不出来：环境问题，等它好。
    Unreadable(String),
    /// 三件套齐全且 labels 可读非空：无需自举。
    Present,
}

/// 判断产物状态。
///
/// **不看 `bert_enabled`**：开关关着但文件齐全是"用户主动关掉语义识别"，
/// 不是"还没训过"，重训会把他刚关掉的开关又打开（自动采用时写 `bert_enabled=true`）。
pub(super) fn artifact_state(nlu: &NluConfig) -> ArtifactState {
    let paths = [
        nlu.bert_model.trim(),
        nlu.bert_vocab.trim(),
        nlu.bert_labels.trim(),
    ];
    if paths.iter().any(|p| p.is_empty()) || paths.iter().any(|p| !Path::new(p).is_file()) {
        return ArtifactState::Missing;
    }
    match std::fs::read_to_string(paths[2]) {
        Ok(text) if text.lines().any(|l| !l.trim().is_empty()) => ArtifactState::Present,
        // labels 文件存在但是空的：当成"还没训"处理，否则模型跑起来没有标签。
        Ok(_) => ArtifactState::Missing,
        // 文件在、读不出 = 权限或挂载问题，不是"没训练"。
        Err(e) => ArtifactState::Unreadable(format!("{}: {e}", paths[2])),
    }
}

// ---------------------------------------------------------------------------
// 前置检查
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub(super) enum Block {
    /// 终态跳过：记 `skipped` + 原因，本任务结束。
    Stop {
        reason: &'static str,
        detail: String,
    },
    /// 暂缓：保持当前 phase，睡够再来（不消耗 `attempts`）。
    Wait { reason: &'static str, seconds: u64 },
}

/// 触发前置检查。纯函数（产物状态由调用方算好传入），便于单测。
///
/// **哪些该 `Stop`、哪些该 `Wait` 是这条路线的关键**：`Stop` 意味着"此后不再自动跑，
/// 必须重启进程"。所以只有**人为决定**才配 `Stop`；环境没就绪、开关临时关掉都属于
/// "过一会儿再看"，必须 `Wait`——否则首个 30s tick 撞上 `[nlu].enabled=false`
/// 或数据卷还没挂好，自举就永久停摆（QA 复核发现的问题）。
pub(super) fn precheck(
    dev_mode: bool,
    mode: Mode,
    feature: bool,
    nlu: &NluConfig,
    artifact: ArtifactState,
    st: &State,
    now: i64,
) -> Option<Block> {
    if dev_mode {
        return Some(Block::Stop {
            reason: "dev_mode",
            detail: "UI 开发模式没有真实引擎，不训练模型".into(),
        });
    }
    if mode == Mode::Off {
        return Some(Block::Stop {
            reason: "bootstrap_off",
            detail: "NLU_BOOTSTRAP=off".into(),
        });
    }
    if !feature {
        return Some(Block::Stop {
            reason: "feature_disabled",
            detail: "本次编译未启用 nlu feature".into(),
        });
    }
    // 开关是配置项，随时可能被管理台打开：轮询而不是放弃。
    if !nlu.enabled {
        return Some(Block::Wait {
            reason: "nlu_disabled",
            seconds: RECHECK_SECS,
        });
    }
    if let Err(e) = validate_buildable(nlu) {
        return Some(Block::Stop {
            reason: "catalog_invalid",
            detail: e,
        });
    }
    // 文件在但读不出（权限/IO/卷没挂好）：训练也会失败，还会白烧 attempts。
    if let ArtifactState::Unreadable(detail) = &artifact {
        tracing::warn!(
            target: "nlu_bootstrap",
            "产物文件读不出（{detail}），{RECHECK_SECS}s 后再看（不消耗尝试次数）"
        );
        return Some(Block::Wait {
            reason: "artifact_unreadable",
            seconds: RECHECK_SECS,
        });
    }
    if artifact == ArtifactState::Present {
        return Some(Block::Stop {
            reason: "artifact_present",
            detail: "模型产物齐全，无需自举".into(),
        });
    }
    if st.phase.blocks_retry() {
        return Some(Block::Stop {
            reason: "phase_terminal",
            detail: format!("phase={:?}", st.phase),
        });
    }
    if st.attempts >= MAX_ATTEMPTS {
        return Some(Block::Stop {
            reason: "attempts_exhausted",
            detail: format!("已尝试 {} 次", st.attempts),
        });
    }
    if let Some(seconds) = backoff_remaining(st, now) {
        return Some(Block::Wait {
            reason: "backoff",
            seconds,
        });
    }
    None
}
