//! Host-side bind mount resolution for the Docker daemon behind docker.sock.
use std::path::Path;

pub(super) fn mount_host_path(path: &str, mountinfo: &str) -> String {
    let mut best: Option<(usize, String)> = None;
    for line in mountinfo.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let lf: Vec<&str> = left.split_whitespace().collect();
        let rf: Vec<&str> = right.split_whitespace().collect();
        if lf.len() < 5 || rf.len() < 2 {
            continue;
        }
        let (root, mnt, source) = (lf[3], lf[4], rf[1]);
        let Some(suffix) = path.strip_prefix(mnt) else {
            continue;
        };
        if !(suffix.is_empty() || suffix.starts_with('/')) {
            continue;
        }
        let rel = suffix.strip_prefix('/').unwrap_or("");
        let base = if root == "/" {
            source.to_string()
        } else {
            format!("{source}{}", root.trim_end_matches('/'))
        };
        let mapped = if rel.is_empty() {
            base
        } else {
            format!("{base}/{rel}")
        };
        if best.as_ref().map_or(true, |(len, _)| mnt.len() > *len) {
            best = Some((mnt.len(), mapped));
        }
    }
    best.map(|(_, p)| p).unwrap_or_else(|| path.to_string())
}

pub(super) fn map_via_mounts(path: &str, mounts: &[(String, String)]) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for (dest, src) in mounts {
        let Some(suffix) = path.strip_prefix(dest.as_str()) else {
            continue;
        };
        if !(suffix.is_empty() || suffix.starts_with('/')) {
            continue;
        }
        if best.as_ref().map_or(true, |(len, _)| dest.len() > *len) {
            best = Some((dest.len(), format!("{src}{suffix}")));
        }
    }
    best.map(|(_, p)| p)
}

async fn own_container(path: &str) -> Option<String> {
    let id = std::env::var("HOSTNAME").ok()?;
    let out = tokio::process::Command::new("docker")
        .args(["inspect", &id])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let pairs = v
        .get(0)?
        .get("Mounts")?
        .as_array()?
        .iter()
        .filter_map(|m| {
            Some((
                m.get("Destination")?.as_str()?.to_string(),
                m.get("Source")?.as_str()?.to_string(),
            ))
        })
        .collect::<Vec<_>>();
    map_via_mounts(path, &pairs)
}

pub(super) async fn docker_host_path(path: &Path) -> String {
    let s = path.display().to_string();
    if let Some(mapped) = own_container(&s).await {
        return mapped;
    }
    match tokio::fs::read_to_string("/proc/self/mountinfo").await {
        Ok(info) => mount_host_path(&s, &info),
        Err(_) => s,
    }
}
