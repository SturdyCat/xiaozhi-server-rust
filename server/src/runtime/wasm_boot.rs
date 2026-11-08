//! WASM 插件生命周期引导（boot）：扫描 → 登记 → 标记恢复/迁移 → 拓扑激活。
//!
//! boot 契约（2026-11-07 生命周期语义）：
//!
//! 1. 扫描产物目录 → 逐个 **登记 Staged**（探测 manifest；pin/attestation 门不过 → 跳过）；
//! 2. `<id>.registered.json` 标记存在 → **恢复 Registered**（标记证明已过完整校验）；
//! 3. **迁移**（老部署：产物存在、无标记、旧缺省启用语义 `legacy_enabled`）→
//!    补注册一次 + 写标记 + 固化 `enabled=true`——含升级前「上传即启用」的所有存量；
//! 4. `enabled=true` 且 Registered → **按依赖拓扑序激活**（环/缺依赖/构建失败 →
//!    该插件停在 Registered 带 warn，其他插件不受影响）。
//!
//! 单个插件故障只跳过自身，绝不阻断核心语音服务启动。

use std::collections::{BTreeMap, HashSet};

use crate::config::store::ConfigStore;
use crate::config::PluginsConfig;
use crate::plugins::wasm::artifacts::{ArtifactDir, RegisteredMarker};
use crate::plugins::wasm::{ActivateError, Lifecycle, WasmHost};

fn contract_label() -> String {
    format!("{}.x", crate::plugins::wasm::CONTRACT_MAJOR_VERSION)
}

/// 持久化单插件激活意图（迁移固化与端点共用同一落点语义）。
fn persist_enabled(store: &dyn ConfigStore, id: &str, enabled: bool) -> anyhow::Result<()> {
    let mut cfg = store.load()?;
    cfg.plugins
        .entries
        .entry(id.to_string())
        .or_default()
        .enabled = enabled;
    store.persist(&cfg)
}

/// pin/attestation 门（严格完整性模式 + 可选签名核验；不过 → 跳过该产物）。
fn admission_allowed(
    config: &PluginsConfig,
    dir: &ArtifactDir,
    id: &str,
    digest: &str,
) -> Result<(), String> {
    let Some(entry) = config.entries.get(id) else {
        if config.require_artifact_pin {
            return Err("严格完整性模式要求 [plugins.entries] 条目（含 artifact pin）".into());
        }
        return Ok(());
    };
    if config.require_artifact_pin && entry.artifact.is_none() {
        return Err("严格完整性模式要求 artifact SHA-256 pin".into());
    }
    if !entry.artifact_matches(digest) {
        return Err(format!(
            "产物摘要与 [plugins.entries] 钉版不符（expected {:?}）",
            entry.artifact
        ));
    }
    if let Some(public_key) = config.attestation_public_key.as_deref() {
        let Some(attestation) = entry.attestation.as_deref() else {
            return Err("已配置 attestation 公钥但插件缺少签名".into());
        };
        if !ArtifactDir::verify_attestation(public_key, attestation, digest) {
            return Err("插件 artifact attestation 校验失败".into());
        }
    }
    let _ = dir; // 目录句柄只为签名一致保留
    Ok(())
}

/// 生命周期引导主入口（Engines::assemble 装配期调用一次）。
pub(super) fn boot_lifecycle(
    config: &PluginsConfig,
    dir: &ArtifactDir,
    host: &WasmHost,
    store: &dyn ConfigStore,
) {
    if !config.enabled {
        tracing::info!("[plugins].enabled=false（平台总闸），跳过插件引导");
        return;
    }

    // ① 扫描 → 登记 Staged
    let mut bytes_by_id: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (id, bytes) in dir.scan() {
        let digest = ArtifactDir::digest(&bytes);
        if let Err(reason) = admission_allowed(config, dir, &id, &digest) {
            tracing::warn!(plugin = %id, "插件产物跳过装载：{reason}");
            continue;
        }
        match host.stage(&id, &bytes) {
            Ok(_) => {
                bytes_by_id.insert(id, bytes);
            }
            Err(e) => tracing::warn!(plugin = %id, "插件登记失败（跳过）：{e:#}"),
        }
    }

    // ② 标记恢复 Registered / 迁移补注册；收集激活候选
    let mut to_activate: Vec<String> = Vec::new();
    let mut migrated: Vec<String> = Vec::new();
    let ids: Vec<String> = bytes_by_id.keys().cloned().collect();
    for id in &ids {
        let has_marker = dir.read_registered_marker(id).is_some();
        if has_marker {
            let _ = host.mark_registered(id);
        } else if config.legacy_enabled(id) {
            // 老部署迁移：无标记 + 旧缺省启用语义 → 补注册一次（完整校验 + self-test）
            let bytes = &bytes_by_id[id];
            match host.register(id, bytes) {
                Ok(_) => {
                    if let Err(e) = dir.write_registered_marker(id, &contract_label()) {
                        tracing::warn!(plugin = %id, "迁移注册标记写入失败：{e}");
                    }
                    // 「隐式缺省启用」固化为显式激活意图（下次 boot 走常规激活路径）。
                    // 传给本函数的是启动快照：迁移成功的插件直接进激活候选，
                    // 不依赖 allows() 复读同一份快照。
                    if let Err(e) = persist_enabled(store, id, true) {
                        tracing::warn!(plugin = %id, "迁移激活意图固化失败（本次运行有效）：{e:#}");
                    }
                    tracing::info!(plugin = %id, "老部署迁移：补注册完成（将按依赖拓扑激活）");
                    migrated.push(id.clone());
                }
                Err(e) => {
                    tracing::warn!(plugin = %id, "迁移注册失败（保持 Staged）：{e:#}");
                    continue;
                }
            }
        }
        if host.lifecycle_of(id).map(|(lc, _)| lc) == Some(Lifecycle::Registered)
            && (config.allows(id) || migrated.iter().any(|m| m == id))
        {
            to_activate.push(id.clone());
        }
    }

    // ③ 拓扑激活：每轮激活「依赖已全部 Active」的候选；无进展 = 剩余全是缺依赖/环
    loop {
        let map = host.lifecycle_map();
        let ready: Vec<String> = to_activate
            .iter()
            .filter(|id| {
                let Some((_, _, requires)) = map.get(id.as_str()) else {
                    return false;
                };
                requires.iter().all(|r| {
                    matches!(
                        map.get(r.id.as_str()),
                        Some((Lifecycle::Active, version, _))
                            if crate::plugins::wasm::version_at_least(version, &r.min_version)
                    )
                })
            })
            .cloned()
            .collect();
        if ready.is_empty() {
            break;
        }
        let mut progressed = false;
        for id in &ready {
            let bytes = &bytes_by_id[id];
            match host.activate_checked(id, bytes) {
                Ok(()) => {
                    tracing::info!(plugin = %id, "引导激活插件（依赖拓扑序）");
                    progressed = true;
                }
                Err(ActivateError::Dependencies(gaps)) => {
                    // 理论上已被 ready 过滤；防御式记录并放弃该候选
                    tracing::warn!(
                        plugin = %id,
                        "引导激活依赖缺失（停在 Registered）：{}",
                        gaps.iter().map(|g| g.id.as_str()).collect::<Vec<_>>().join("、")
                    );
                }
                Err(ActivateError::WrongLifecycle { current }) => {
                    tracing::warn!(plugin = %id, "引导激活 lifecycle={current}（跳过）");
                }
                Err(ActivateError::NotFound) => {
                    tracing::warn!(plugin = %id, "引导激活条目丢失（跳过）");
                }
                Err(ActivateError::Build(reason)) => {
                    tracing::warn!(plugin = %id, "引导激活构建失败（停在 Registered）：{reason}");
                }
            }
        }
        to_activate.retain(|id| host.lifecycle_of(id).map(|(lc, _)| lc) != Some(Lifecycle::Active));
        if !progressed {
            break;
        }
    }
    if !to_activate.is_empty() {
        let remaining: HashSet<&String> = to_activate.iter().collect();
        tracing::warn!(
            "引导激活未完成（缺依赖/依赖环/构建失败，停在 Registered）：{:?}",
            remaining
        );
    }
}

/// 标记信息回读（`RegisteredMarker` 调试投影；boot 只判存在性）。
#[allow(dead_code)]
pub(super) fn marker_of(dir: &ArtifactDir, id: &str) -> Option<RegisteredMarker> {
    dir.read_registered_marker(id)
}

#[cfg(test)]
mod tests;
