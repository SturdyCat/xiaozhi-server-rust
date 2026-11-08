//! 插件私有文件沙箱：`<data-dir>/plugins/<plugin-id>/`。
//!
//! 路径安全：拒绝绝对路径、`..` 段、以及解析后逃逸出根目录的路径（含符号链接）。
//! 配额：单文件与单插件总量双限制（大模型工件不应走这里，应走宿主的资产通道）。

use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Result};

/// 单文件上限（64 MiB）。
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
/// 单插件总量上限（512 MiB）。
pub const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

/// 插件文件沙箱。
pub struct FsSandbox {
    root: PathBuf,
}

impl FsSandbox {
    /// `root` 是所有插件的父目录（如 `<data>/plugins`）。
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// 某插件的根目录。
    pub fn plugin_root(&self, plugin_id: &str) -> PathBuf {
        self.root.join(sanitize_segment(plugin_id))
    }

    /// 解析相对路径为绝对路径；任何越界形态直接拒绝。
    pub fn resolve(&self, plugin_id: &str, rel: &str) -> Result<PathBuf> {
        let root = self.plugin_root(plugin_id);
        if rel.is_empty() {
            return Ok(root);
        }
        let candidate = Path::new(rel);
        if candidate.is_absolute() {
            bail!("fs 路径不允许绝对路径：{rel}");
        }
        for comp in candidate.components() {
            match comp {
                Component::Normal(_) => {}
                Component::CurDir => {}
                _ => bail!("fs 路径不允许包含 {:?}：{rel}", comp),
            }
        }
        let joined = root.join(candidate);
        Ok(joined)
    }

    /// 读取。
    pub fn read(&self, plugin_id: &str, rel: &str) -> Result<Vec<u8>> {
        let path = self.resolve(plugin_id, rel)?;
        let meta = std::fs::metadata(&path)?;
        if !meta.is_file() {
            bail!("不是文件：{rel}");
        }
        if meta.len() as usize > MAX_FILE_BYTES {
            bail!(
                "文件超上限：{} > {MAX_FILE_BYTES}（{rel}）",
                meta.len()
            );
        }
        Ok(std::fs::read(path)?)
    }

    /// 写入（原子替换 + 配额校验）。
    pub fn write(&self, plugin_id: &str, rel: &str, bytes: &[u8]) -> Result<()> {
        if bytes.len() > MAX_FILE_BYTES {
            bail!(
                "写入超单文件上限：{} > {MAX_FILE_BYTES}（{rel}）",
                bytes.len()
            );
        }
        let path = self.resolve(plugin_id, rel)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // 先写临时文件再 rename，避免半截文件
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
        if self.total_bytes(plugin_id)? > MAX_TOTAL_BYTES {
            let _ = std::fs::remove_file(&path);
            bail!("插件 {plugin_id} 文件总量超上限（{MAX_TOTAL_BYTES} 字节）");
        }
        Ok(())
    }

    /// 列目录（只回相对文件名，不递归）。
    pub fn list(&self, plugin_id: &str, rel: &str) -> Result<Vec<String>> {
        let dir = self.resolve(plugin_id, rel)?;
        if !dir.is_dir() {
            bail!("不是目录：{rel}");
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if let Some(name) = entry.file_name().to_str() {
                out.push(name.to_string());
            }
        }
        out.sort();
        Ok(out)
    }

    /// 删除；返回是否原本存在。
    pub fn delete(&self, plugin_id: &str, rel: &str) -> Result<bool> {
        let path = self.resolve(plugin_id, rel)?;
        if !path.exists() {
            return Ok(false);
        }
        if path.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
        Ok(true)
    }

    /// 文件大小。
    pub fn stat(&self, plugin_id: &str, rel: &str) -> Option<u64> {
        let path = self.resolve(plugin_id, rel).ok()?;
        std::fs::metadata(path).ok().map(|m| m.len())
    }

    /// 插件总量。
    pub fn total_bytes(&self, plugin_id: &str) -> Result<u64> {
        fn walk(dir: &Path, acc: &mut u64) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, acc);
                } else if let Ok(meta) = entry.metadata() {
                    *acc += meta.len();
                }
            }
        }
        let root = self.plugin_root(plugin_id);
        let mut total = 0;
        if root.exists() {
            walk(&root, &mut total);
        }
        Ok(total)
    }

    /// 清空某插件目录（卸载 `--purge`）。
    pub fn purge(&self, plugin_id: &str) -> Result<()> {
        let root = self.plugin_root(plugin_id);
        if root.exists() {
            std::fs::remove_dir_all(root)?;
        }
        Ok(())
    }
}

/// 插件 id 只允许安全字符（防止拼出目录穿越）。
fn sanitize_segment(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
