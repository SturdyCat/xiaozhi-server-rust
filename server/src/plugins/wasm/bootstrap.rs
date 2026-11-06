//! 官方默认插件启动引导（用户方向 2026-11-04：产物不入库，缺产物系统自建）。
//!
//! 语义：数据卷 `plugins/<id>.wasm` 已在（上传或上次引导）→ 什么都不做（assemble 的
//! scan 已装载）；缺失 → 按 `plugins-src/manifest.toml`（数据表，内核零插件知识）
//! 调 Docker 构建镜像执行 `build.sh` → 隔离验证 → 采用（写盘 + 热装载）。
//! Docker/镜像/源缺失 → 降级告警并提示管理界面上传替代，**绝不阻断启动**。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use super::artifacts::ArtifactDir;
use super::registry::WasmHost;

/// 构建镜像（可用环境变量 `XZ_PLUGIN_BUILDER_IMAGE` 覆盖）。
pub const DEFAULT_BUILDER_IMAGE: &str = "xiaozhi-plugin-builder:latest";

/// 引导表条目。
#[derive(Debug, Clone, PartialEq)]
pub struct BootstrapEntry {
    pub id: String,
    pub dir: String,
}

/// 解析 `plugins-src/manifest.toml`（引导表是数据：新增官方默认插件不改内核代码）。
pub fn load_manifest(repo_root: &Path) -> Result<Vec<BootstrapEntry>> {
    let path = repo_root.join("plugins-src/manifest.toml");
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("插件引导表缺失 {}（官方默认插件的登记处）", path.display()))?;
    let value: toml::Value = raw.parse().context("引导表 TOML 解析失败")?;
    let Some(list) = value.get("plugins").and_then(toml::Value::as_array) else {
        anyhow::bail!("引导表缺少 [[plugins]] 条目");
    };
    let mut out = Vec::new();
    for item in list {
        let id = item
            .get("id")
            .and_then(toml::Value::as_str)
            .unwrap_or_default();
        let dir = item
            .get("dir")
            .and_then(toml::Value::as_str)
            .unwrap_or_default();
        if id.is_empty() || dir.is_empty() {
            anyhow::bail!("引导表条目缺 id/dir：{item:?}");
        }
        out.push(BootstrapEntry {
            id: id.to_string(),
            dir: dir.to_string(),
        });
    }
    Ok(out)
}

/// 引导一个默认插件：产物在 → 跳过；缺 → Docker 构建到夹具输出路径 → 验证 → 采用。
/// 返回是否新装载（供调用方记日志/投影）。
pub fn ensure_plugin(
    host: &Arc<WasmHost>,
    artifacts: &ArtifactDir,
    entry: &BootstrapEntry,
    repo_root: &Path,
    docker_available: bool,
    attestation: Option<(&str, &str)>,
) -> Result<bool> {
    // 已采用（上传或上次引导）→ 无事可做
    if artifacts.path().join(format!("{}.wasm", entry.id)).exists() {
        return Ok(false);
    }
    let plugin_dir = repo_root.join("plugins-src").join(&entry.dir);
    let build_script = plugin_dir.join("build.sh");
    if !build_script.is_file() {
        anyhow::bail!(
            "插件源目录缺失 {}（仓库分发不完整；可经管理界面上传已构建组件替代）",
            plugin_dir.display()
        );
    }
    if !docker_available {
        anyhow::bail!(
            "Docker 不可用：无法构建默认插件 {}（安装 Docker 并准备镜像 {DEFAULT_BUILDER_IMAGE}，或上传已构建组件）",
            entry.id
        );
    }
    let image =
        std::env::var("XZ_PLUGIN_BUILDER_IMAGE").unwrap_or_else(|_| DEFAULT_BUILDER_IMAGE.into());
    let repo_root = canonicalize(repo_root);
    // 构建在容器内进行（源/目标都以仓库根挂载；产物落在 server/tests/fixtures/wasm/）
    let status = std::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{}:/work", repo_root.display()),
            "-w",
            &format!("/work/plugins-src/{}", entry.dir),
            &image,
            "build.sh",
        ])
        .status()
        .context("docker 启动失败")?;
    if !status.success() {
        anyhow::bail!("Docker 构建失败（{}，exit={:?}）", entry.id, status.code());
    }
    let built = repo_root
        .join("server/tests/fixtures/wasm")
        .join(format!("{}.wasm", entry.id));
    let bytes = std::fs::read(&built).with_context(|| {
        format!(
            "构建产物未出现在 {}（检查 build.sh 输出路径）",
            built.display()
        )
    })?;
    // 与上传同门：先验证后采用（写盘 + 装载）
    host.validate(&entry.id, &bytes)
        .context("构建产物隔离验证失败（未采用）")?;
    if let Some((public_key, signature)) = attestation {
        let digest = ArtifactDir::digest(&bytes);
        if !ArtifactDir::verify_attestation(public_key, signature, &digest) {
            anyhow::bail!("构建产物 artifact attestation 校验失败（未采用）");
        }
    }
    host.load(&entry.id, &bytes).context("构建产物装载失败")?;
    artifacts
        .save(&entry.id, &bytes)
        .context("构建产物写盘失败")?;
    Ok(true)
}

fn canonicalize(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// 后台引导全部默认插件（main 启动时 spawn；Docker 构建走 spawn_blocking 不占运行时）。
/// 逐插件独立降级：任何一个失败只告警，不影响其余插件与核心语音服务。
pub async fn ensure_defaults(
    host: &Arc<WasmHost>,
    artifacts: &ArtifactDir,
    plugins_cfg: &crate::config::PluginsConfig,
) {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let entries = match load_manifest(&repo_root) {
        Ok(entries) if !entries.is_empty() => entries,
        Ok(_) => return,
        Err(e) => {
            tracing::warn!("默认插件引导表不可用（跳过引导）：{e:#}");
            return;
        }
    };
    let docker = docker_available();
    for entry in entries {
        // [plugins] 配置意图：禁用的插件不引导构建（上传替代路径同样被配置闸挡住）
        if !plugins_cfg.allows(&entry.id) {
            tracing::info!(plugin = %entry.id, "配置禁用（[plugins]），跳过引导");
            continue;
        }
        let entry_config = plugins_cfg.entries.get(&entry.id);
        if plugins_cfg.require_artifact_pin
            && entry_config
                .and_then(|config| config.artifact.as_deref())
                .is_none()
        {
            tracing::warn!(plugin = %entry.id, "严格完整性模式要求 artifact SHA-256 pin，跳过未钉版引导");
            continue;
        }
        if plugins_cfg.attestation_public_key.is_some()
            && entry_config
                .and_then(|config| config.attestation.as_deref())
                .is_none()
        {
            tracing::warn!(plugin = %entry.id, "已配置 attestation 公钥但插件缺少签名，跳过引导");
            continue;
        }
        let attestation = plugins_cfg
            .attestation_public_key
            .as_deref()
            .and_then(|public_key| {
                entry_config
                    .and_then(|config| config.attestation.as_deref())
                    .map(|signature| (public_key.to_owned(), signature.to_owned()))
            });
        let result = tokio::task::spawn_blocking({
            let host = Arc::clone(host);
            let artifacts = artifacts.clone();
            let entry = entry.clone();
            let repo = repo_root.clone();
            move || {
                let attestation = attestation
                    .as_ref()
                    .map(|(public_key, signature)| (public_key.as_str(), signature.as_str()));
                ensure_plugin(&host, &artifacts, &entry, &repo, docker, attestation)
            }
        })
        .await;
        match result {
            Ok(Ok(true)) => tracing::info!(plugin = %entry.id, "默认插件已构建并采用"),
            Ok(Ok(false)) => tracing::debug!(plugin = %entry.id, "默认插件产物已在（跳过构建）"),
            Ok(Err(e)) => {
                tracing::warn!(plugin = %entry.id, "默认插件引导失败（可经管理界面上传替代）：{e:#}")
            }
            Err(e) => tracing::warn!(plugin = %entry.id, "默认插件引导任务失败：{e}"),
        }
    }
}

/// docker 二进制是否可用（PATH 探测；不做守护进程探测——真正的失败在构建时报告）。
pub fn docker_available() -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .any(|dir| dir.join("docker").exists() || dir.join("docker.exe").exists())
        })
        .unwrap_or(false)
}
