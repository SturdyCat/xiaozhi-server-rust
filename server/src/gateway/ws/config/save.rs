//! `PUT /api/config` 与首启自举共用的**唯一写库路径**（`config/presets.rs` 另有既有 `persist`，
//! 不在本文件职责内）。
//!
//! 拆出原因有两条：
//! 1. `config.rs` 已达 §600 行约定上限，而这块逻辑自成一体（读现值 → 校验指纹 →
//!    合并补丁 → workflow 预编译 → persist → 计算生效档位）。
//! 2. 首启自举要"自动采用刚训完的候选"，最容易走歪的实现就是自己
//!    "load → 改 4 个 bert 字段 → persist"。那会绕过 [`super::merge_client_patch`]
//!    里的密钥 sanitize（GET 已打码、客户端回传空串 = 保留原值），一次自动采用就可能
//!    把别处的 `api_key` 写坏。所以自举必须复用本模块，**不允许第二条写库路径**。
//!
//! 两个入口只差"当前配置从哪来"这一行，主链完全共用：
//! - [`save_patch`]：`current_config()`，读库失败回退内存快照（管理台"库坏了也能打开页面
//!   重新保存一次来修复"的既有设计）；
//! - [`save_patch_strict`]：直接 `config_store.load()`，读失败返回 [`SaveError::Io`]。
//!   自举必须用这个：如果拿"回退的内存快照"当基线，算出的 revision 必然不等于
//!   `artifact.source_revision`，一次读库抖动就会被误判成"配置被别处改过"而永久 `stale`，
//!   理由还是假的。
//!
//! **这不是新增语义，而是纠既有违规**：`nlu_build/AGENTS.md` 第 5 条本来就要求构建链路
//! "只能读最新保存快照，**不能使用** `live_config()`、启动快照或**读取失败后的旧配置回退**"。
//! 也就是说 `current_config()` 的回退行为对构建/自举链路一直是越界的，本轮把自举拉回文档
//! 既有要求。反过来，`PUT /api/config` 保留回退语义与 `config.rs:42-43` 的既有设计一致
//! （"库坏了也要能打开页面，重新保存一次即可修复"），两边不冲突、也不会被互相拖改。
//! 评审时请按这个读，别把 `save_patch_strict` 当成范围扩张。
//!
//! 失败原因是 typed 枚举而不是 `Response`：HTTP 端点要状态码，自举要写进状态机的
//! 失败归因；返回响应体的话，自举就得反过来解析自己刚拼出来的 JSON。

use crate::config::Config;
use crate::runtime::Engines;

use super::impact::{save_impact, SaveImpact};
use super::{config_revision, current_config, merge_client_patch};

/// 日志前缀按来源区分，保持运维既有的 grep 规则（原来只有 `PUT /api/config：`）。
pub(crate) const ORIGIN_HTTP: &str = "PUT /api/config";
pub(crate) const ORIGIN_BOOTSTRAP: &str = "NLU bootstrap";

/// 一次保存的失败原因。
#[derive(Debug)]
pub(crate) enum SaveError {
    /// 保存态已被别处改动（`current` 是当前指纹，供 409 回传）。
    ///
    /// **不可重试**：用同一个 `expected_revision` 再试一次，结果仍然是冲突。
    RevisionConflict { current: String },
    /// 补丁**合并/解析**不合法（400 纯文本语义，与 `HEAD` 一字不差）。不可重试。
    ///
    /// 注意这里刻意只表示"JSON 结构/类型错了"这一类：`PUT` 在 `HEAD` 上对这类失败
    /// 返回的就是纯文本，把它一起改成 JSON 就不是"PUT 语义不变"了。
    Invalid { detail: String },
    /// workflow 预编译不合法（400 **JSON** 语义）。
    ///
    /// 与 [`Self::Invalid`] 分开的唯一原因：`HEAD` 上这两条失败的响应体格式本来就不同
    /// （workflow 是 `json_msg`，合并失败是纯文本）。合成一个变体后 handler 只能二选一，
    /// 于是 workflow 那条被降级成纯文本，客户端 `optString("message")` 取不到内容，
    /// 界面从"哪个节点不合法"退化成"保存失败: HTTP 400"。
    /// `detail` 里**已经带** `workflow 校验失败：` 前缀，handler 直接塞进 `json_msg` 即可，
    /// 不要再拼一次前缀。
    WorkflowInvalid { detail: String },
    /// 没有持久化落点（内置默认启动）。
    NoPersistence,
    /// 读取当前配置失败（只在 [`save_patch_strict`] 产生）。
    ///
    /// 与 [`Self::RevisionConflict`] 严格区分：这是"没读到"，不是"变了"，
    /// 归因成冲突会让人去重建一个本来没问题的模型。
    Io { detail: String },
    /// 写库失败（500 语义）。瞬时 IO，可重试。
    Persist { path: String, detail: String },
}

impl SaveError {
    /// 是否属于"瞬时 IO"（可重试）。`RevisionConflict`/`Invalid`/`NoPersistence` 都不算。
    pub(crate) fn is_retryable_io(&self) -> bool {
        matches!(self, Self::Io { .. } | Self::Persist { .. })
    }
}

/// 一次成功保存的结果。
pub(crate) struct SaveOutcome {
    /// 写入后的新指纹（客户端拿它继续做乐观并发控制，自举拿它写日志）。
    pub(crate) revision: String,
    pub(crate) path: String,
    pub(crate) impact: SaveImpact,
}

/// 读现值 → 校验指纹 → 合并补丁（密钥 sanitize 在 [`merge_client_patch`] 内）→
/// workflow 预编译 → persist。`PUT /api/config` 入口。
///
/// `expected_revision` 为 `None` 时跳过并发检查（旧客户端不传 revision 的兼容语义）。
pub(crate) fn save_patch(
    engines: &Engines,
    patch: &serde_json::Value,
    expected_revision: Option<&str>,
) -> Result<SaveOutcome, SaveError> {
    let current = current_config(engines);
    save_patch_with(engines, patch, expected_revision, current, ORIGIN_HTTP)
}

/// 自举专用入口：**读失败即失败**，绝不拿内存快照冒充"当前配置"。
///
/// 没有持久化落点时退回内存配置——那种情况下内存就是唯一真源，与"读库失败"不是一回事。
pub(crate) fn save_patch_strict(
    engines: &Engines,
    patch: &serde_json::Value,
    expected_revision: Option<&str>,
) -> Result<SaveOutcome, SaveError> {
    let Some(path) = engines.config_path().map(str::to_string) else {
        let current = (*engines.config).clone();
        return save_patch_with(engines, patch, expected_revision, current, ORIGIN_BOOTSTRAP);
    };
    let current = engines.config_store.load().map_err(|e| {
        tracing::error!("{ORIGIN_BOOTSTRAP}：读取配置库 {path} 失败（不回退内存快照）: {e:#}");
        SaveError::Io {
            detail: format!("{e:#}"),
        }
    })?;
    save_patch_with(engines, patch, expected_revision, current, ORIGIN_BOOTSTRAP)
}

/// 主链：两个入口唯一共用的部分。
///
/// **已知限制**（也记在 `nlu_build/AGENTS.md` 的已知边界）：本链是
/// `read → compare → persist`，**没有真正的 CAS**。`compare` 与 `persist` 之间的亚秒窗口
/// 里若有人写入，会被本次保存静默覆盖（last-write-wins）。彻底修需要 store 层支持
/// put-if-revision。自举的 `stale` 只挡得住"训练期间配置被改"，挡不住这个窗口。
fn save_patch_with(
    engines: &Engines,
    patch: &serde_json::Value,
    expected_revision: Option<&str>,
    current: Config,
    origin: &str,
) -> Result<SaveOutcome, SaveError> {
    let current_revision = config_revision(&current);
    if let Some(expected) = expected_revision {
        if expected != current_revision {
            return Err(SaveError::RevisionConflict {
                current: current_revision,
            });
        }
    }
    // 1) 现盘配置做基线 → 未出现的字段保持不动；密钥空串 = 保留原值（见 `sanitize_patch`）。
    // 2) 合并结果必须仍是合法 Config：类型/枚举错误在此报 400（带精确原因与字段路径）。
    let body: Config = merge_client_patch(&current, patch).map_err(|e| {
        tracing::error!("{origin}：客户端配置 JSON 合并/解析失败: {e}");
        SaveError::Invalid {
            detail: format!("配置 JSON 解析失败: {e}"),
        }
    })?;
    // Workflow is compiled before persistence: invalid plans can never become active.
    // 观察者目录来自当前装载的 wasm 插件（保存时严格校验；会话侧对"保存后卸载"宽松剔除）。
    if let Err(error) =
        crate::workflow::designer::compile_with_observers(&body.workflow, &engines.wasm_observers())
    {
        return Err(SaveError::WorkflowInvalid {
            detail: format!("workflow 校验失败：{error}"),
        });
    }
    let Some(path) = engines.config_path().map(str::to_string) else {
        return Err(SaveError::NoPersistence);
    };
    engines.config_store.persist(&body).map_err(|e| {
        // 健壮性：失败原因同时记入服务端日志（客户端只能看到状态码）
        tracing::error!("{origin}：写入 {path} 失败: {e:#}");
        SaveError::Persist {
            path: path.clone(),
            detail: format!("{e:#}"),
        }
    })?;
    let impact = save_impact(&current, &body);
    tracing::debug!(
        "{origin}：本次改动段 [{}]，字段 [{}]，生效档位 {}",
        impact.sections.join(", "),
        impact.paths.join(", "),
        impact.hot.as_str()
    );
    Ok(SaveOutcome {
        revision: config_revision(&body),
        path,
        impact,
    })
}
