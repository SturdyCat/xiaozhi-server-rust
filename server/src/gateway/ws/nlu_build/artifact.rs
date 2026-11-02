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
    pub files: Vec<FileIdentity>,
    pub nlu: NluConfig,
}

pub(crate) fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id)
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
            "labels":self.labels,"catalog_source":self.catalog_source,"files":self.files,"output_dir":dir,
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

pub(crate) fn load_candidate(data: &Path, id: &str) -> Result<(Artifact, PathBuf)> {
    let root = data
        .join("models")
        .join("nlu-builds")
        .canonicalize()
        .context("no managed candidate builds exist")?;
    let requested = output_dir(data, id)?;
    let meta = std::fs::symlink_metadata(&requested)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        bail!("candidate directory is not managed");
    }
    let dir = requested.canonicalize()?;
    if dir.parent() != Some(root.as_path()) {
        bail!("candidate directory escapes managed root");
    }
    let manifest = controlled_file(&dir, "manifest.json")?;
    if std::fs::metadata(&manifest)?.len() > 1024 * 1024 {
        bail!("candidate manifest exceeds 1MiB");
    }
    let artifact: Artifact = serde_json::from_slice(&std::fs::read(manifest)?)?;
    if artifact.build_id != id {
        bail!("candidate manifest build_id mismatch");
    }
    if artifact.files.len() != FILES.len() {
        bail!("candidate manifest files mismatch");
    }
    for name in FILES {
        let expected = artifact
            .files
            .iter()
            .find(|f| f.name == name)
            .context("candidate manifest missing file")?;
        let actual = identity(&dir, name)?;
        if expected.bytes != actual.bytes || expected.sha256 != actual.sha256 {
            bail!("candidate integrity mismatch: {name}");
        }
    }
    if labels_at(&dir)? != artifact.labels {
        bail!("candidate labels changed");
    }
    Ok((artifact, dir))
}

#[cfg(test)]
#[path = "artifact_tests.rs"]
mod tests;
