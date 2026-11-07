//! 插件产物落盘（上传即启用模型）：`<数据卷>/plugins/<id>.wasm`。
//!
//! 分发语义（用户方向，2026-11-04）：**插件不编译进主系统**——服务端零本地构建，
//! 管理界面上传已构建的 wasm 组件 → 隔离验证（self-test/schema）→ 采用（写盘 + 热装载）
//! → 立即提供功能；删除文件即卸载。重启时 [`scan`](ArtifactDir::scan) 重装全部产物。
//!
//! 落点派生与 skills/tuya 同口径：配置库位置优先，`memory.db_path` 父目录兜底，
//! 不写容器层硬编码路径。上传的字节体本身就是"启用意图"；P3 起可在
//! `[plugins.entries."<id>"]` 持久化 `enabled` 与 `artifact` SHA-256 内容钉版。

use std::path::{Path, PathBuf};

use base64::Engine;
use ring::signature;
use sha2::{Digest, Sha256};

/// 单产物大小上限（组件实际 ~40-160KB；8MB 足够容纳带嵌入资源的插件）。
pub const MAX_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;

/// 产物目录（数据卷下的 `plugins/` 子目录）。
#[derive(Debug, Clone)]
pub struct ArtifactDir(PathBuf);

impl ArtifactDir {
    /// 从配置库位置派生：`<配置库父目录>/plugins`；无位置（内置配置）返回 None。
    pub fn for_location(location: Option<&str>) -> Option<Self> {
        let parent = location
            .map(Path::new)
            .and_then(Path::parent)
            .filter(|p| !p.as_os_str().is_empty())?;
        Some(Self(parent.join("plugins")))
    }

    fn path_for(&self, id: &str) -> PathBuf {
        // id 形状已由装载校验保证（小写字母数字与 -/_ 与 .），不含路径分隔符
        self.0.join(format!("{id}.wasm"))
    }

    /// 计算产物内容摘要（持久化钉版与上传响应共用此口径）。
    pub fn digest(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// 验证 detached Ed25519 attestation：签名覆盖摘要的 ASCII 文本。
    /// 空配置保持兼容；只有同时配置公钥和条目签名时才启用密码学 admission。
    pub fn verify_attestation(public_key: &str, signature_b64: &str, digest: &str) -> bool {
        let Ok(public_key) = base64::engine::general_purpose::STANDARD.decode(public_key) else {
            return false;
        };
        let Ok(signature_bytes) = base64::engine::general_purpose::STANDARD.decode(signature_b64)
        else {
            return false;
        };
        let key = signature::UnparsedPublicKey::new(&signature::ED25519, public_key);
        key.verify(digest.as_bytes(), &signature_bytes).is_ok()
    }

    /// 读取已落盘产物并计算摘要；缺失或不可读时返回 None。
    pub fn digest_for(&self, id: &str) -> Option<String> {
        std::fs::read(self.path_for(id))
            .ok()
            .map(|bytes| Self::digest(&bytes))
    }

    /// 保存产物（写盘即持久启用；临时文件 + 原子改名，半份文件不会被发现）。
    pub fn save(&self, id: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
        if bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(std::io::Error::other(format!(
                "产物超上限（{} > {MAX_ARTIFACT_BYTES} 字节）",
                bytes.len()
            )));
        }
        std::fs::create_dir_all(&self.0)?;
        let final_path = self.path_for(id);
        let tmp = self.0.join(format!(".{id}.wasm.tmp"));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &final_path)?;
        Ok(final_path)
    }

    /// 删除产物（卸载持久意图；文件不存在返回 false）。
    pub fn remove(&self, id: &str) -> bool {
        std::fs::remove_file(self.path_for(id)).is_ok()
    }

    /// 扫描全部产物：文件名（去 .wasm）= 插件 id。坏文件跳过并记录（不阻断其余插件）。
    pub fn scan(&self) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.0) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(id) = name.strip_suffix(".wasm") else {
                continue;
            };
            if id.is_empty() || id.starts_with('.') {
                continue;
            }
            match std::fs::read(&path) {
                Ok(bytes) if bytes.len() <= MAX_ARTIFACT_BYTES => out.push((id.to_string(), bytes)),
                Ok(_) => tracing::warn!(plugin = id, "产物超大小上限，跳过"),
                Err(e) => tracing::warn!(plugin = id, "产物读取失败，跳过：{e}"),
            }
        }
        out
    }

    /// 目录路径（日志/排查用）。
    pub fn path(&self) -> &Path {
        &self.0
    }

    // ============================================================
    // Registered 生命周期标记（`<id>.registered.json`，与产物同目录）
    // ============================================================

    fn marker_path(&self, id: &str) -> PathBuf {
        self.0.join(format!("{id}.registered.json"))
    }

    /// 写注册标记（Registered 态持久化：重启扫描据此恢复 Registered 而非 Staged）。
    pub fn write_registered_marker(&self, id: &str, contract_version: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.0)?;
        let registered_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let body = serde_json::json!({
            "registered_at": registered_at,
            "contract_version": contract_version,
        });
        let tmp = self.0.join(format!(".{id}.registered.json.tmp"));
        std::fs::write(&tmp, body.to_string())?;
        std::fs::rename(&tmp, self.marker_path(id))?;
        Ok(())
    }

    /// 读注册标记；缺失或损坏 → None（损坏按未注册处理，重新注册即补写）。
    pub fn read_registered_marker(&self, id: &str) -> Option<RegisteredMarker> {
        let text = std::fs::read_to_string(self.marker_path(id)).ok()?;
        let value: serde_json::Value = serde_json::from_str(&text).ok()?;
        Some(RegisteredMarker {
            registered_at: value.get("registered_at").and_then(|v| v.as_u64())?,
            contract_version: value
                .get("contract_version")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        })
    }

    /// 删除注册标记（移除插件时一并回收；文件不存在返回 false）。
    pub fn remove_registered_marker(&self, id: &str) -> bool {
        std::fs::remove_file(self.marker_path(id)).is_ok()
    }
}

/// 注册标记内容（信息性；完整性由产物 pin/attestation 门保障）。
#[derive(Debug, Clone)]
pub struct RegisteredMarker {
    pub registered_at: u64,
    pub contract_version: String,
}
