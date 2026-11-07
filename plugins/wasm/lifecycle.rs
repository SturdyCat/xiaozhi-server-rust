//! 插件生命周期三态机：Staged（已上传）→ Registered（已注册）→ Active（运行）。
//!
//! 状态语义（契约锁定，2026-11-07）：
//!
//! - **Staged**：上传即登记不运行——只探测 manifest（形状校验），不收集注册包、
//!   不实例化常驻、不投影 UI/菜单；
//! - **Registered**：完整校验（manifest/权限/注册包/self-test）已过、注册包已收集，
//!   但**未装载**——重启经 `<id>.registered.json` 标记恢复本态；
//! - **Active**：运行中。`Phase`（active/failed/degraded）只是 Active 态的运行健康
//!   子状态；非 Active 条目一律投影 `{"state":"inactive"}`。
//!
//! 依赖（`manifest.requires`）只在**激活期**一次性判定：被依赖者须 Active 且版本
//! ≥ min-version（数字点分段，见 [`compare_versions`]）。停用被依赖者前须无 Active
//! 依赖者；移除仅限非 Active。同 id 上传 = 版本升级判定（新>旧才接受）。

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context, Result};

use super::contract::{
    ManifestDto, RegistrationDto, RequirementDto, compare_versions, validate_manifest,
};
use super::effects::Permissions;
use super::instance::LoadedPlugin;
use super::registry::{Phase, PluginEntry, WasmHost};

/// 生命周期三态（snapshot 投影字符串 = `as_str()`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    /// 已上传（manifest 已探测）；产物落盘、无注册标记。
    Staged,
    /// 已注册（完整校验 + 注册包已收集）；未装载，重启经标记恢复。
    Registered,
    /// 运行中（实例在表）。
    Active,
}

impl Lifecycle {
    pub fn as_str(&self) -> &'static str {
        match self {
            Lifecycle::Staged => "staged",
            Lifecycle::Registered => "registered",
            Lifecycle::Active => "active",
        }
    }
}

/// 非 Active 条目的统一运行子状态（phase 只在 Active 态有意义）。
pub(super) fn inactive_phase() -> Phase {
    Phase::Failed("inactive".to_string())
}

/// stage 的结果（升级判定信息随响应回显）。
#[derive(Debug)]
pub struct StageOutcome {
    pub id: String,
    pub version: String,
    /// true = 同 id 覆盖且版本更高（升级）；false = 新插件首次登记。
    pub upgraded: bool,
    pub previous_version: Option<String>,
    /// 升级时保留的原生命周期（Active 由调用方随后原地重装载）。
    pub preserved: Option<crate::plugins::wasm::lifecycle::Lifecycle>,
}

/// 依赖缺口（激活 409 的明细行）。
#[derive(Debug)]
pub struct DependencyGap {
    pub id: String,
    pub min_version: String,
    /// `missing`（未上传）/ `inactive`（存在但非 Active）/ `version`（Active 但版本低）。
    pub reason: &'static str,
    pub active_version: Option<String>,
}

impl DependencyGap {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "min_version": self.min_version,
            "reason": self.reason,
            "active_version": self.active_version,
        })
    }
}

/// 激活失败的结构化原因（端点据此映射状态码）。
#[derive(Debug)]
pub enum ActivateError {
    NotFound,
    /// 当前生命周期不允许激活（staged 未注册 / 已是 active）。
    WrongLifecycle {
        current: &'static str,
    },
    /// 依赖缺口（409 + 明细）。
    Dependencies(Vec<DependencyGap>),
    /// 实例构建失败（旧实例保留）。
    Build(String),
}

/// 停用失败的结构化原因。
#[derive(Debug)]
pub enum DeactivateError {
    NotFound,
    WrongLifecycle {
        current: &'static str,
    },
    /// 存在 Active 依赖者，停用会破坏它们（409 + 清单）。
    Dependents(Vec<String>),
}

/// 移除失败的结构化原因。
#[derive(Debug)]
pub enum RemoveError {
    NotFound,
    /// Active 插件须先停用（409）。
    Active,
}

/// 登记失败的结构化原因（端点据此映射状态码）。
#[derive(Debug)]
pub enum StageError {
    /// 同 id 版本未升级（新 ≤ 旧）→ 409。
    VersionNotNewer { new: String, current: String },
    /// manifest 探测/校验失败 → 400。
    Invalid(String),
}

impl std::fmt::Display for StageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StageError::VersionNotNewer { new, current } => {
                write!(
                    f,
                    "版本不升级：新版本 {new} ≤ 现有 {current}（升级须更高版本）"
                )
            }
            StageError::Invalid(msg) => write!(f, "{msg}"),
        }
    }
}

impl WasmHost {
    /// 探测组件 manifest（免落表）：实例化临时实例 → manifest() → 形状校验。
    pub fn probe_manifest(
        &self,
        expected_id: &str,
        wasm: &[u8],
    ) -> Result<ManifestDto, StageError> {
        let manifest = self.probe_manifest_inner(expected_id, wasm)?;
        validate_manifest(&manifest).map_err(|e| {
            StageError::Invalid(format!("组件 manifest 校验失败（{expected_id}）：{e:#}"))
        })?;
        Ok(manifest)
    }

    fn probe_manifest_inner(
        &self,
        expected_id: &str,
        wasm: &[u8],
    ) -> Result<ManifestDto, StageError> {
        let mut probe = LoadedPlugin::instantiate(
            &self.engine,
            wasm,
            expected_id,
            self.limits,
            Arc::clone(&self.kv),
        )
        .map_err(|e| StageError::Invalid(format!("组件实例化失败（{expected_id}）：{e:#}")))?;
        let manifest = probe.manifest().map_err(|e| {
            StageError::Invalid(format!("组件 manifest() 失败（{expected_id}）：{e:#}"))
        })?;
        if manifest.id != expected_id {
            return Err(StageError::Invalid(format!(
                "manifest.id ({}) 与请求 id ({expected_id}) 不一致（拒绝）",
                manifest.id
            )));
        }
        Ok(manifest)
    }

    /// 登记（Staged）：探测 manifest + 版本升级判定，**不装载、不注册**。
    /// 同 id 已存在：新版本 > 旧版本才接受（保持原生命周期）；否则报错。
    pub fn stage(&self, expected_id: &str, wasm: &[u8]) -> Result<StageOutcome, StageError> {
        let manifest = self.probe_manifest(expected_id, wasm)?;
        let mut entries = self.entries.lock().expect("entries lock poisoned");
        if let Some(e) = entries.get_mut(expected_id) {
            let previous = e.manifest.version.clone();
            let order = compare_versions(&manifest.version, &previous).ok_or_else(|| {
                StageError::Invalid(format!("版本不可比较：{} vs {previous}", manifest.version))
            })?;
            if order != std::cmp::Ordering::Greater {
                return Err(StageError::VersionNotNewer {
                    new: manifest.version.clone(),
                    current: previous,
                });
            }
            let preserved = e.lifecycle;
            // Active 条目的实例仍是旧版本：调用方落盘后须 reload_active 原地换新
            e.manifest = manifest;
            e.registration = RegistrationDto::default();
            e.phase = inactive_phase();
            e.instance = None;
            return Ok(StageOutcome {
                id: expected_id.to_string(),
                version: e.manifest.version.clone(),
                upgraded: true,
                previous_version: Some(previous),
                preserved: Some(preserved),
            });
        }
        let version = manifest.version.clone();
        entries.insert(
            expected_id.to_string(),
            PluginEntry {
                manifest,
                registration: RegistrationDto::default(),
                permissions: Permissions::default(),
                phase: inactive_phase(),
                instance: None,
                lifecycle: Lifecycle::Staged,
            },
        );
        tracing::info!(plugin = expected_id, "插件登记为 Staged（未装载）");
        Ok(StageOutcome {
            id: expected_id.to_string(),
            version,
            upgraded: false,
            previous_version: None,
            preserved: None,
        })
    }

    /// 注册（Staged → Registered）：完整校验（manifest/权限/注册包）+ self-test；
    /// 任一失败保留 Staged 并带原因。成功后不保留实例（未装载）。
    pub fn register(&self, expected_id: &str, wasm: &[u8]) -> Result<RegistrationDto> {
        let mut candidate = self.build_entry(expected_id, wasm)?;
        let results = candidate
            .instance
            .as_mut()
            .expect("build_entry 成功必有实例")
            .self_test()?;
        if let Some(bad) = results.iter().find(|r| !r.ok) {
            anyhow::bail!("self-test 未通过：{}：{}", bad.name, bad.detail);
        }
        let mut entries = self.entries.lock().expect("entries lock poisoned");
        let e = entries
            .get_mut(expected_id)
            .ok_or_else(|| anyhow::anyhow!("插件 {expected_id} 未上传（须先上传为 Staged）"))?;
        if e.lifecycle != Lifecycle::Staged {
            anyhow::bail!(
                "插件 {expected_id} 当前 lifecycle={}（仅 staged 可注册）",
                e.lifecycle.as_str()
            );
        }
        e.manifest = candidate.manifest;
        e.registration = candidate.registration.clone();
        e.permissions = candidate.permissions;
        e.lifecycle = Lifecycle::Registered;
        e.phase = inactive_phase();
        e.instance = None;
        tracing::info!(plugin = expected_id, "插件注册为 Registered（未装载）");
        Ok(candidate.registration)
    }

    /// 标记恢复（boot：`<id>.registered.json` 存在 → Staged 直升 Registered）。
    /// 标记本身证明了此前已过完整校验；产物完整性由 pin/attestation 门保障。
    pub fn mark_registered(&self, expected_id: &str) -> Result<()> {
        let mut entries = self.entries.lock().expect("entries lock poisoned");
        let e = entries
            .get_mut(expected_id)
            .ok_or_else(|| anyhow::anyhow!("插件 {expected_id} 未登记"))?;
        if e.lifecycle == Lifecycle::Staged {
            e.lifecycle = Lifecycle::Registered;
            tracing::info!(plugin = expected_id, "注册标记恢复为 Registered");
        }
        Ok(())
    }

    /// 激活（Registered → Active）：依赖前置判定 + 先建新后替换。
    /// 成功后配置门 `enabled=true` 由调用方持久化。
    pub fn activate_checked(&self, expected_id: &str, wasm: &[u8]) -> Result<(), ActivateError> {
        {
            let entries = self.entries.lock().expect("entries lock poisoned");
            let e = entries.get(expected_id).ok_or(ActivateError::NotFound)?;
            if e.lifecycle != Lifecycle::Registered {
                return Err(ActivateError::WrongLifecycle {
                    current: e.lifecycle.as_str(),
                });
            }
            let gaps = dependency_gaps(&entries, &e.manifest.requires);
            if !gaps.is_empty() {
                return Err(ActivateError::Dependencies(gaps));
            }
        }
        let entry = self
            .build_entry(expected_id, wasm)
            .map_err(|e| ActivateError::Build(format!("{e:#}")))?;
        let mut entries = self.entries.lock().expect("entries lock poisoned");
        entries.insert(expected_id.to_string(), entry);
        tracing::info!(plugin = expected_id, "插件激活（Registered → Active）");
        Ok(())
    }

    /// 原地换版本（Active 升级）：依赖复判 + 先建新后替换 + 注册包刷新。
    /// 失败保留旧实例（磁盘产物已是新版本，重启按新版本装载）。
    pub fn reload_active(&self, expected_id: &str, wasm: &[u8]) -> Result<()> {
        {
            let entries = self.entries.lock().expect("entries lock poisoned");
            let e = entries
                .get(expected_id)
                .ok_or_else(|| anyhow::anyhow!("插件 {expected_id} 未登记"))?;
            if e.lifecycle != Lifecycle::Active {
                anyhow::bail!(
                    "插件 {expected_id} lifecycle={}（非 Active 不做原地换版本）",
                    e.lifecycle.as_str()
                );
            }
            let gaps = dependency_gaps(&entries, &e.manifest.requires);
            if !gaps.is_empty() {
                anyhow::bail!(
                    "新版本依赖不满足：{}",
                    gaps.iter()
                        .map(|g| g.id.as_str())
                        .collect::<Vec<_>>()
                        .join("、")
                );
            }
        }
        let entry = self
            .build_entry(expected_id, wasm)
            .with_context(|| format!("插件 {expected_id} 新版本构建失败（旧实例保留）"))?;
        let mut entries = self.entries.lock().expect("entries lock poisoned");
        entries.insert(expected_id.to_string(), entry);
        tracing::info!(plugin = expected_id, "插件原地升级换版本（Active 保持）");
        Ok(())
    }

    /// 停用（Active → Registered）：存在 Active 依赖者时拒绝；实例卸载、
    /// 产物/标记/kv 全保留（重新激活即恢复运行上下文）。
    pub fn deactivate(&self, id: &str) -> Result<(), DeactivateError> {
        let mut entries = self.entries.lock().expect("entries lock poisoned");
        // 先只读判定（生命周期 + Active 依赖者），再取可变写实例字段（避免借用冲突）
        {
            let e = entries.get(id).ok_or(DeactivateError::NotFound)?;
            if e.lifecycle != Lifecycle::Active {
                return Err(DeactivateError::WrongLifecycle {
                    current: e.lifecycle.as_str(),
                });
            }
            let dependents = dependents_of(&entries, id);
            if !dependents.is_empty() {
                return Err(DeactivateError::Dependents(dependents));
            }
        }
        let e = entries.get_mut(id).expect("只读判定已确认条目存在");
        e.instance = None;
        e.lifecycle = Lifecycle::Registered;
        e.phase = inactive_phase();
        tracing::info!(plugin = id, "插件停用（Active → Registered；kv/产物保留）");
        Ok(())
    }

    /// 移除（仅 Staged/Registered）：注册表条目 + kv 命名空间回收；
    /// 产物/标记文件由调用方删除。
    pub fn remove(&self, id: &str) -> Result<(), RemoveError> {
        let mut entries = self.entries.lock().expect("entries lock poisoned");
        let Some(e) = entries.get(id) else {
            return Err(RemoveError::NotFound);
        };
        if e.lifecycle == Lifecycle::Active {
            return Err(RemoveError::Active);
        }
        entries.remove(id);
        drop(entries);
        self.kv.clear_plugin(id);
        tracing::info!(plugin = id, "插件移除（条目 + kv 回收）");
        Ok(())
    }

    /// 查询生命周期与版本（boot/端点用；未登记 → None）。
    pub fn lifecycle_of(&self, id: &str) -> Option<(Lifecycle, String)> {
        let entries = self.entries.lock().expect("entries lock poisoned");
        entries
            .get(id)
            .map(|e| (e.lifecycle, e.manifest.version.clone()))
    }

    /// Active 依赖者清单（停用 409 与 snapshot 投影共用）。
    pub fn active_dependents_of(&self, id: &str) -> Vec<String> {
        let entries = self.entries.lock().expect("entries lock poisoned");
        dependents_of(&entries, id)
    }

    /// 依赖投影行：`{id, min_version, satisfied, active_version}`。
    pub fn requires_view(&self, id: &str) -> Vec<serde_json::Value> {
        let entries = self.entries.lock().expect("entries lock poisoned");
        let Some(e) = entries.get(id) else {
            return Vec::new();
        };
        e.manifest
            .requires
            .iter()
            .map(|r| {
                let (satisfied, active_version) = match entries.get(&r.id) {
                    Some(dep) if dep.lifecycle == Lifecycle::Active => (
                        compare_versions(&dep.manifest.version, &r.min_version)
                            != Some(std::cmp::Ordering::Less),
                        Some(dep.manifest.version.clone()),
                    ),
                    Some(dep) => (
                        false,
                        Some(format!("{}（非 Active）", dep.manifest.version)),
                    ),
                    None => (false, None),
                };
                serde_json::json!({
                    "id": r.id,
                    "min_version": r.min_version,
                    "satisfied": satisfied,
                    "active_version": active_version,
                })
            })
            .collect()
    }

    /// 全量生命周期快照（boot 拓扑排序的数据源）：
    /// `id → (lifecycle, version, requires[(id, min_version)])`。
    pub fn lifecycle_map(&self) -> BTreeMap<String, (Lifecycle, String, Vec<RequirementDto>)> {
        let entries = self.entries.lock().expect("entries lock poisoned");
        entries
            .iter()
            .map(|(id, e)| {
                (
                    id.clone(),
                    (
                        e.lifecycle,
                        e.manifest.version.clone(),
                        e.manifest.requires.clone(),
                    ),
                )
            })
            .collect()
    }

    /// 管理台/`/api/plugins` 投影（P2 与静态注册表清单合并）。
    /// `registration`（含 UI）仅 Active 投影——未激活不渲染业务；`lifecycle`
    /// 与 `requires`/`dependents` 随行，客户端 Active 判定 =
    /// `lifecycle=="active" && phase.state=="active"`。
    pub fn snapshot_json(&self) -> serde_json::Value {
        let entries = self.entries.lock().expect("entries lock poisoned");
        let plugins: Vec<serde_json::Value> = entries
            .iter()
            .map(|(id, e)| {
                let active = e.lifecycle == Lifecycle::Active;
                let dependents = dependents_of(&entries, id);
                serde_json::json!({
                    "id": id,
                    "version": e.manifest.version,
                    "api_version": e.manifest.api_version,
                    "title": e.manifest.title,
                    "description": e.manifest.description,
                    "official": e.manifest.official,
                    "implements": e.manifest.implements,
                    "permissions": e.manifest.permissions,
                    "lifecycle": e.lifecycle.as_str(),
                    "phase": if active { phase_json(&e.phase) } else { serde_json::json!({"state": "inactive"}) },
                    "registration": if active {
                        serde_json::json!({
                            "menu": e.registration.menu,
                            "config_fields": e.registration.config_fields,
                            "actions": e.registration.actions,
                            // A2UI 透传：仅 Active 插件投影 UI（未注册插件不返回业务 UI）
                            "ui": super::contract::ui_json_value(e.registration.ui_json.as_deref()),
                        })
                    } else {
                        serde_json::Value::Null
                    },
                    "requires": e.manifest.requires.iter().map(|r| {
                        let (satisfied, active_version) = match entries.get(&r.id) {
                            Some(dep) if dep.lifecycle == Lifecycle::Active => (
                                compare_versions(&dep.manifest.version, &r.min_version)
                                    != Some(std::cmp::Ordering::Less),
                                Some(dep.manifest.version.clone()),
                            ),
                            Some(dep) => (
                                false,
                                Some(format!("{}（非 Active）", dep.manifest.version)),
                            ),
                            None => (false, None),
                        };
                        serde_json::json!({
                            "id": r.id, "min_version": r.min_version,
                            "satisfied": satisfied, "active_version": active_version,
                        })
                    }).collect::<Vec<_>>(),
                    "dependents": dependents,
                })
            })
            .collect();
        // 扩展点解析投影：只收集 Active 条目声明的扩展点（未激活插件不进入解析面）
        let mut points: Vec<String> = Vec::new();
        for e in entries
            .values()
            .filter(|e| e.lifecycle == Lifecycle::Active)
        {
            for p in &e.manifest.implements {
                if !points.contains(p) {
                    points.push(p.clone());
                }
            }
        }
        drop(entries);
        let (table, world) = self.views();
        let mut ext_map = serde_json::Map::new();
        for p in &points {
            ext_map.insert(p.clone(), resolution_json(&table.resolve(p, &world)));
        }
        let extensions = serde_json::Value::Object(ext_map);
        serde_json::json!({ "plugins": plugins, "extensions": extensions })
    }
}

/// 依赖缺口判定（激活/原地升级共用）：被依赖者须存在、Active 且版本 ≥ min。
fn dependency_gaps(
    entries: &BTreeMap<String, PluginEntry>,
    requires: &[RequirementDto],
) -> Vec<DependencyGap> {
    requires
        .iter()
        .filter_map(|r| match entries.get(&r.id) {
            None => Some(DependencyGap {
                id: r.id.clone(),
                min_version: r.min_version.clone(),
                reason: "missing",
                active_version: None,
            }),
            Some(dep) if dep.lifecycle != Lifecycle::Active => Some(DependencyGap {
                id: r.id.clone(),
                min_version: r.min_version.clone(),
                reason: "inactive",
                active_version: Some(dep.manifest.version.clone()),
            }),
            Some(dep) => {
                let order = compare_versions(&dep.manifest.version, &r.min_version);
                if order.is_none() || order == Some(std::cmp::Ordering::Less) {
                    Some(DependencyGap {
                        id: r.id.clone(),
                        min_version: r.min_version.clone(),
                        reason: "version",
                        active_version: Some(dep.manifest.version.clone()),
                    })
                } else {
                    None
                }
            }
        })
        .collect()
}

/// Active 依赖者（`requires` 含 `id` 且自身 Active）。
fn dependents_of(entries: &BTreeMap<String, PluginEntry>, id: &str) -> Vec<String> {
    entries
        .iter()
        .filter(|(other, oe)| {
            *other != id
                && oe.lifecycle == Lifecycle::Active
                && oe.manifest.requires.iter().any(|r| r.id == id)
        })
        .map(|(other, _)| other.clone())
        .collect()
}

fn phase_json(p: &Phase) -> serde_json::Value {
    match p {
        Phase::Active => serde_json::json!({"state": "active"}),
        Phase::Failed(e) => serde_json::json!({"state": "failed", "error": e}),
        Phase::Degraded { kind, detail } => {
            serde_json::json!({"state": "degraded", "trap": format!("{kind:?}"), "detail": detail})
        }
    }
}

fn resolution_json(r: &super::resolver::Resolution) -> serde_json::Value {
    match r {
        super::resolver::Resolution::Resolved(id) => {
            serde_json::json!({"state": "resolved", "implementation": id})
        }
        super::resolver::Resolution::Fallback {
            official,
            preferred,
            reason,
        } => serde_json::json!({
            "state": "fallback", "implementation": official, "preferred": preferred, "reason": reason,
        }),
        super::resolver::Resolution::Unresolved(reason) => {
            serde_json::json!({"state": "unresolved", "reason": reason})
        }
    }
}
