//! Versioned, checksummed managed candidates. No arbitrary model path is accepted.
use crate::plugins::nlu::NluConfig;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

const FILES: [&str; 3] = ["model.onnx", "tokenizer.json", "labels.txt"];
const MAX_FILE: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct FileIdentity {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Artifact {
    pub build_id: String,
    pub source_revision: String,
    pub source_input_sha256: String,
    pub trainer_image: String,
    pub trainer_image_id: String,
    pub trainer_script_sha256: String,
    pub labels: Vec<String>,
    /// Absent in older manifests: their NLU and label snapshots remain unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_source: Option<String>,
    /// 训练用预训练底座规范名（minirbt-h288）。与 `catalog_source` 同一兼容范式：
    /// 旧 manifest 没有该键时读成 None，仅作历史展示，不影响任何构建口径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_model: Option<String>,
    /// 产物封版时刻（RFC3339 / UTC，秒精度 `Z` 后缀）。三件套校验全过、manifest
    /// 即将落盘时打点，是"这份产物是哪次构建出的"唯一可信来源。
    ///
    /// **绝不用文件 mtime 兜底**：mtime 是文件系统记账时间，`docker cp`、rsync、
    /// 解包、备份恢复都会改写它，与构建无关。给一个看起来像真时间的假值，比承认
    /// "未记录"更糟（同 `unmeasured` 口径）——旧 manifest 无此键 → None，界面显示
    /// 「未记录构建时间」，绝不拒收。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    pub files: Vec<FileIdentity>,
    pub nlu: NluConfig,
}

/// 产物封版时刻的统一格式：固定秒精度 + `Z`，保证同一字段只有一种写法
/// （`+00:00` 与 `Z` 混用会让"字符串序 == 时间序"这个排序捷径失效）。
/// `clock` feature 已开；`SecondsFormat` 在 `default-features=false` 下同样可用。
pub(super) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub(crate) fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id)
}

// ---------------------------------------------------------------------------
// 列表页判据：类型化错误 + 四态 + O(1) 窄头部
// ---------------------------------------------------------------------------

/// 候选读取的**类型化**错误。与 `load_candidate` 旧 anyhow 形态的区别：
/// 每个变体对应一种运维动作，HTTP 映射（`candidate_http_error`）收敛在**一处**——
/// 404 只给"根/目录不存在"，其余一律 409 且错误码可区分。此前 6 种情况压成一个
/// anyhow，`/test` 靠 `io::ErrorKind::NotFound` 反推，会把"目录在但缺文件"也报成
/// 404，列表说半成品、点进去 404 不存在的错位就是这么来的。
#[derive(Debug)]
pub(crate) enum CandidateError {
    /// 管理根目录不存在（从未构建过任何东西）。
    RootMissing,
    /// 该 build 目录不存在 / 不是受管目录形态（非目录、符号链接、逃逸根外）。
    DirectoryMissing,
    /// manifest 在但缺文件（含 manifest 自身缺失）。
    Incomplete { missing: Vec<String> },
    /// manifest 解析失败、manifest build_id 与目录名不符、目录逃逸受管根、
    /// 文件是符号链接 / 超限等"形态坏了"的情况。
    Corrupt { detail: String },
    /// 三件套哈希与 manifest 记录不符（文件被改过）。
    HashMismatch {
        name: String,
        expected: String,
        actual: String,
    },
    /// labels.txt 与 manifest 记录的标签顺序不一致。
    LabelsChanged { detail: String },
}

impl std::fmt::Display for CandidateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RootMissing => write!(f, "no managed candidate builds exist"),
            Self::DirectoryMissing => write!(f, "candidate directory is not managed"),
            Self::Incomplete { missing } => {
                write!(
                    f,
                    "candidate is incomplete, missing: {}",
                    missing.join(", ")
                )
            }
            Self::Corrupt { detail } => write!(f, "candidate manifest is corrupt: {detail}"),
            Self::HashMismatch {
                name,
                expected,
                actual,
            } => write!(
                f,
                "candidate integrity mismatch: {name} (manifest {expected}, disk {actual})"
            ),
            Self::LabelsChanged { detail } => write!(f, "candidate labels changed: {detail}"),
        }
    }
}

impl std::error::Error for CandidateError {}

/// 列表行状态词表，定死四态（散落别处一律引用这里的常量，不许手写字符串）：
/// - `ok`：完整校验（含三件套哈希）这次进程里刚通过；
/// - `unverified`：manifest 与三件套都在，但哈希这次没算（列表页 O(1) 判据）；
/// - `incomplete`：manifest 在但缺文件；
/// - `corrupt`：manifest 解析不了 / 与目录不符 / 文件形态坏了。
/// `unverified` 必须与 `ok` 视觉分离——没验过就是没验过，不许装成验过。
pub(crate) const INTEGRITY_OK: &str = "ok";
pub(crate) const INTEGRITY_UNVERIFIED: &str = "unverified";
pub(crate) const INTEGRITY_INCOMPLETE: &str = "incomplete";
pub(crate) const INTEGRITY_CORRUPT: &str = "corrupt";

/// 错误 → HTTP 的**唯一**映射点：404 只给"根/目录不存在"（没这个东西，去找构建
/// 入口），其余一律 409（东西在，但与记录对不上，该查产物）。错误码在 body 里
/// 可区分，前端据此给不同的处置文案。
pub(crate) fn candidate_http_error(
    e: &CandidateError,
) -> (axum::http::StatusCode, &'static str, String) {
    use axum::http::StatusCode;
    match e {
        CandidateError::RootMissing => {
            (StatusCode::NOT_FOUND, "candidate_not_found", e.to_string())
        }
        CandidateError::DirectoryMissing => {
            (StatusCode::NOT_FOUND, "candidate_not_found", e.to_string())
        }
        CandidateError::Incomplete { .. } => {
            (StatusCode::CONFLICT, "candidate_incomplete", e.to_string())
        }
        CandidateError::Corrupt { .. } => {
            (StatusCode::CONFLICT, "candidate_corrupt", e.to_string())
        }
        CandidateError::HashMismatch { .. } => (
            StatusCode::CONFLICT,
            "candidate_hash_mismatch",
            e.to_string(),
        ),
        CandidateError::LabelsChanged { .. } => (
            StatusCode::CONFLICT,
            "candidate_labels_changed",
            e.to_string(),
        ),
    }
}

/// 候选行的窄头部：只取列表需要的字段。**绝不用 `Artifact` 全量反序列化**——
/// 它有必填字段，任何更老的 manifest 少一个键就会让整个列表 500。
/// `build_id` 一律取自目录名（唯一真相）；manifest 里若有 `build_id`，只做
/// 一致性校验，不符 → 整行 corrupt。
#[derive(Clone, Debug, Serialize)]
pub(crate) struct ManifestHead {
    pub build_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_model: Option<String>,
    pub labels: Vec<String>,
    pub files: Vec<FileIdentity>,
    /// `unverified`（文件齐全但没算哈希）/ `incomplete` / `corrupt` 之一；
    /// `ok` 只由完整校验路径（`load_candidate` 成功）产出。
    pub integrity: &'static str,
    /// `integrity == incomplete` 时非空：缺哪些文件。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ManifestFileProbe {
    name: String,
}

#[derive(Deserialize)]
#[serde(default)]
struct ManifestHeadRaw {
    build_id: Option<String>,
    created_at: Option<String>,
    base_model: Option<String>,
    labels: Vec<String>,
    files: Vec<ManifestFileProbe>,
}

impl Default for ManifestHeadRaw {
    fn default() -> Self {
        Self {
            build_id: None,
            created_at: None,
            base_model: None,
            labels: Vec::new(),
            files: Vec::new(),
        }
    }
}

/// 列表页判据：只读 `manifest.json` + 三件套**存在性**，O(1)，**不算哈希**。
/// 目录名是唯一真相：`id` 就是 build_id，manifest 里的只做一致性校验。
/// 连 manifest 都解析不了的行仍然要能出现在列表里（corrupt），所以这里返回
/// 结构化结果而不是让调用方丢弃。
pub(crate) fn load_manifest(data: &Path, id: &str) -> Result<ManifestHead, CandidateError> {
    let root = managed_root(data)?;
    let dir = managed_dir(&root, id)?;
    // manifest 自身：缺失 → incomplete；符号链接/超限/读失败 → corrupt；解析失败 → corrupt。
    let manifest_path = dir.join("manifest.json");
    let raw = match std::fs::symlink_metadata(&manifest_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ManifestHead {
                build_id: id.to_string(),
                created_at: None,
                base_model: None,
                labels: Vec::new(),
                files: Vec::new(),
                integrity: INTEGRITY_INCOMPLETE,
                missing: vec!["manifest.json".into()],
            });
        }
        Err(e) => {
            return Err(CandidateError::Corrupt {
                detail: format!("manifest.json 不可读: {e}"),
            })
        }
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(CandidateError::Corrupt {
                detail: "manifest.json is not a regular managed file".into(),
            })
        }
        Ok(meta) if meta.len() > 1024 * 1024 => {
            return Err(CandidateError::Corrupt {
                detail: "candidate manifest exceeds 1MiB".into(),
            })
        }
        Ok(_) => std::fs::read(&manifest_path).map_err(|e| CandidateError::Corrupt {
            detail: format!("manifest.json 不可读: {e}"),
        })?,
    };
    let head: ManifestHeadRaw =
        serde_json::from_slice(&raw).map_err(|e| CandidateError::Corrupt {
            detail: format!("manifest.json 解析失败: {e}"),
        })?;
    if let Some(manifest_id) = &head.build_id {
        if manifest_id != id {
            return Err(CandidateError::Corrupt {
                detail: "candidate manifest build_id mismatch".into(),
            });
        }
    }
    // manifest 记录的文件 + 磁盘存在性，双方都齐才算文件齐全。
    let mut missing: Vec<String> = Vec::new();
    let mut recorded: Vec<&str> = head.files.iter().map(|f| f.name.as_str()).collect();
    recorded.sort_unstable();
    for name in FILES {
        if !recorded.contains(&name) {
            missing.push(format!("{name} (manifest 未记录)"));
        } else {
            let path = dir.join(name);
            let present = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file());
            if !present {
                missing.push(name.into());
            }
        }
    }
    if !missing.is_empty() {
        return Ok(ManifestHead {
            build_id: id.to_string(),
            created_at: head.created_at,
            base_model: head.base_model,
            labels: head.labels,
            files: Vec::new(),
            integrity: INTEGRITY_INCOMPLETE,
            missing,
        });
    }
    Ok(ManifestHead {
        build_id: id.to_string(),
        created_at: head.created_at,
        base_model: head.base_model,
        labels: head.labels,
        files: Vec::new(),
        integrity: INTEGRITY_UNVERIFIED,
        missing: Vec::new(),
    })
}

/// 管理根目录：不存在（或不是目录）→ RootMissing。
fn managed_root(data: &Path) -> Result<PathBuf, CandidateError> {
    let root = data.join("models").join("nlu-builds");
    match std::fs::symlink_metadata(&root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(CandidateError::RootMissing),
        Err(e) => Err(CandidateError::Corrupt {
            detail: format!("managed root 不可读: {e}"),
        }),
        Ok(meta) if meta.is_dir() => root.canonicalize().map_err(|e| CandidateError::Corrupt {
            detail: format!("managed root 不可解析: {e}"),
        }),
        Ok(_) => Err(CandidateError::Corrupt {
            detail: "managed root is not a directory".into(),
        }),
    }
}

/// 受管 build 目录：规范化 UUID + 目录存在 + 不逃逸根外。
fn managed_dir(root: &Path, id: &str) -> Result<PathBuf, CandidateError> {
    if !valid_id(id) {
        return Err(CandidateError::DirectoryMissing);
    }
    let requested = root.join(id);
    let meta = std::fs::symlink_metadata(&requested).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CandidateError::DirectoryMissing
        } else {
            CandidateError::Corrupt {
                detail: format!("candidate directory 不可读: {e}"),
            }
        }
    })?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(CandidateError::DirectoryMissing);
    }
    let dir = requested
        .canonicalize()
        .map_err(|_| CandidateError::DirectoryMissing)?;
    if dir.parent() != Some(root) {
        return Err(CandidateError::Corrupt {
            detail: "candidate directory escapes managed root".into(),
        });
    }
    Ok(dir)
}

pub(super) fn output_dir(data: &Path, id: &str) -> Result<PathBuf> {
    if !valid_id(id) {
        bail!("invalid build_id: expected a canonical UUID");
    }
    Ok(data.join("models").join("nlu-builds").join(id))
}

fn controlled_file(dir: &Path, name: &str) -> Result<PathBuf> {
    let path = dir.join(name);
    let meta = std::fs::symlink_metadata(&path)?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        bail!("candidate file is not a regular managed file: {name}");
    }
    if meta.len() == 0 || meta.len() > MAX_FILE {
        bail!("candidate file has invalid size: {name}");
    }
    if path.canonicalize()?.parent() != Some(dir) {
        bail!("candidate file escapes managed directory: {name}");
    }
    Ok(path)
}

fn identity(dir: &Path, name: &str) -> Result<FileIdentity> {
    let path = controlled_file(dir, name)?;
    let mut file = File::open(path)?;
    let bytes = file.metadata()?.len();
    let mut digest = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        digest.update(&buf[..n]);
    }
    Ok(FileIdentity {
        name: name.into(),
        bytes,
        sha256: hex(&digest.finalize()),
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(super) fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub(super) fn labels_at(dir: &Path) -> Result<Vec<String>> {
    let path = controlled_file(dir, "labels.txt")?;
    if std::fs::metadata(&path)?.len() > 64 * 1024 {
        bail!("labels.txt exceeds 64KiB");
    }
    let labels = std::fs::read_to_string(path)?
        .lines()
        .map(str::trim)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let unique = labels.iter().collect::<HashSet<_>>();
    if labels.is_empty() || labels.iter().any(String::is_empty) || unique.len() != labels.len() {
        bail!("candidate labels must be nonempty and unique");
    }
    Ok(labels)
}

impl Artifact {
    pub(super) fn collect(mut self, dir: &Path) -> Result<Self> {
        let dir = dir.canonicalize()?;
        let actual = labels_at(&dir)?;
        if actual != self.labels {
            bail!("candidate labels do not match source intent order plus other");
        }
        self.files = FILES
            .iter()
            .map(|name| identity(&dir, name))
            .collect::<Result<_>>()?;
        Ok(self)
    }

    pub(crate) fn model_sha256(&self) -> &str {
        self.files
            .iter()
            .find(|f| f.name == "model.onnx")
            .map_or("", |f| f.sha256.as_str())
    }

    pub(crate) fn config(&self, dir: &Path) -> NluConfig {
        let mut cfg = self.nlu.clone();
        cfg.bert_enabled = true; // Isolated object, never persisted or published.
        cfg.bert_model = dir.join(FILES[0]).display().to_string();
        cfg.bert_vocab = dir.join(FILES[1]).display().to_string();
        cfg.bert_labels = dir.join(FILES[2]).display().to_string();
        cfg
    }

    pub(crate) fn public_json(&self, dir: &Path) -> Value {
        let cfg = self.config(dir);
        json!({"build_id":self.build_id,"source_revision":self.source_revision,
            "source_input_sha256":self.source_input_sha256,"trainer_image":self.trainer_image,
            "trainer_image_id":self.trainer_image_id,"trainer_script_sha256":self.trainer_script_sha256,"model_sha256":self.model_sha256(),
            "labels":self.labels,"catalog_source":self.catalog_source,
            "base_model":self.base_model,
            "created_at":self.created_at,
            "files":self.files,"output_dir":dir,
            "expected_config":{"bert_enabled":true,"bert_model":cfg.bert_model,
                "bert_vocab":cfg.bert_vocab,"bert_labels":cfg.bert_labels,
                "bert_threshold":cfg.bert_threshold,"bert_num_threads":cfg.bert_num_threads,
                "bert_max_len":cfg.bert_max_len,"bert_label_map":cfg.bert_label_map},
            "auto_activated":false,"validation":"artifacts_and_labels_verified_not_business_accuracy"})
    }
}

pub(super) fn write_manifest(dir: &Path, artifact: &Artifact) -> Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join("manifest.json"))?;
    file.write_all(&serde_json::to_vec_pretty(artifact)?)?;
    file.sync_all()?;
    Ok(())
}

/// anyhow 错误 → 类型化错误：NotFound 视为文件缺失（incomplete），
/// 其余（受管形态违规、大小超限等 bail）视为 corrupt。
fn classify_file_error(name: &str, e: anyhow::Error) -> CandidateError {
    if e.chain().any(|c| {
        c.downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
    }) {
        CandidateError::Incomplete {
            missing: vec![name.into()],
        }
    } else {
        CandidateError::Corrupt {
            detail: format!("{name}: {e:#}"),
        }
    }
}

pub(crate) fn load_candidate(data: &Path, id: &str) -> Result<(Artifact, PathBuf), CandidateError> {
    let root = managed_root(data)?;
    let dir = managed_dir(&root, id)?;
    let manifest = dir.join("manifest.json");
    // 受管目录形态由 managed_dir 保证；manifest 大小与内容在此校验。
    let meta = std::fs::symlink_metadata(&manifest).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            CandidateError::Incomplete {
                missing: vec!["manifest.json".into()],
            }
        } else {
            CandidateError::Corrupt {
                detail: format!("manifest.json 不可读: {e}"),
            }
        }
    })?;
    if meta.file_type().is_symlink() {
        return Err(CandidateError::Corrupt {
            detail: "manifest.json is not a regular managed file".into(),
        });
    }
    if meta.len() > 1024 * 1024 {
        return Err(CandidateError::Corrupt {
            detail: "candidate manifest exceeds 1MiB".into(),
        });
    }
    let raw = std::fs::read(&manifest).map_err(|e| CandidateError::Corrupt {
        detail: format!("manifest.json 不可读: {e}"),
    })?;
    let artifact: Artifact = serde_json::from_slice(&raw).map_err(|e| CandidateError::Corrupt {
        detail: format!("manifest.json 解析失败: {e}"),
    })?;
    if artifact.build_id != id {
        return Err(CandidateError::Corrupt {
            detail: "candidate manifest build_id mismatch".into(),
        });
    }
    if artifact.files.len() != FILES.len() {
        return Err(CandidateError::Corrupt {
            detail: "candidate manifest files mismatch".into(),
        });
    }
    for name in FILES {
        let expected = artifact
            .files
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| CandidateError::Incomplete {
                missing: vec![format!("{name} (manifest 未记录)")],
            })?;
        let actual = identity(&dir, name).map_err(|e| classify_file_error(name, e))?;
        if expected.bytes != actual.bytes || expected.sha256 != actual.sha256 {
            return Err(CandidateError::HashMismatch {
                name: name.into(),
                expected: expected.sha256.clone(),
                actual: actual.sha256,
            });
        }
    }
    if labels_at(&dir).map_err(|e| classify_file_error("labels.txt", e))? != artifact.labels {
        return Err(CandidateError::LabelsChanged {
            detail: "candidate labels.txt does not match manifest label order".into(),
        });
    }
    Ok((artifact, dir))
}

#[cfg(test)]
#[path = "artifact_tests.rs"]
mod tests;
