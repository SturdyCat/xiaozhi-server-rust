//! Docker process orchestration. Version directories are exclusive; failures never touch old models.
use super::{
    artifact::{self, Artifact},
    job, log_tx,
    mounts::docker_host_path,
    push_log, BuildEvent,
};
use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

/// 逐行转发子进程输出。
///
/// **必须带跨 chunk 的半行缓冲**：管道按 4096 字节切，一行（尤其是训练脚本末尾那行
/// `[train] smoke={...}`）很容易跨边界；以前每块直接 `.lines()` 会把一行劈成两行，
/// 冒烟 JSON 因此在 `job.log` 里坏掉 → 被误判 `smoke_missing`。
async fn stream(mut pipe: impl AsyncRead + Unpin) {
    let mut buf = [0u8; 4096];
    let mut pending: Vec<u8> = Vec::new();
    loop {
        match pipe.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                pending.extend_from_slice(&buf[..n]);
                // 只切到最后一个换行：剩下的半行留到下一块，保证每行完整。
                let cut = pending.iter().rposition(|b| *b == b'\n').map(|i| i + 1);
                if let Some(cut) = cut {
                    let complete: Vec<u8> = pending.drain(..cut).collect();
                    for line in String::from_utf8_lossy(&complete).lines() {
                        push_log(line).await;
                    }
                }
                // 防御：单行超过 64KiB（无换行的病态输出）时强制 flush，避免无限吃内存。
                if pending.len() > 64 * 1024 {
                    let flushed: Vec<u8> = pending.drain(..).collect();
                    for line in String::from_utf8_lossy(&flushed).lines() {
                        push_log(line).await;
                    }
                }
            }
        }
    }
    if !pending.is_empty() {
        for line in String::from_utf8_lossy(&pending).lines() {
            push_log(line).await;
        }
    }
}

async fn run_cmd(mut cmd: Command) -> Result<()> {
    cmd.kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .context("cannot start Docker CLI (docker.sock + CLI required)")?;
    let stdout = child.stdout.take().context("missing Docker stdout")?;
    let stderr = child.stderr.take().context("missing Docker stderr")?;
    let readers = tokio::spawn(async move {
        tokio::join!(stream(stdout), stream(stderr));
    });
    let status = tokio::time::timeout(std::time::Duration::from_secs(3600), child.wait()).await;
    let status = match status {
        Ok(status) => status?,
        Err(_) => {
            let _ = child.kill().await;
            readers.abort();
            bail!("Docker command exceeded 3600 seconds; training may require operator cleanup on host");
        }
    };
    let _ = readers.await;
    if !status.success() {
        bail!("Docker command failed: {status}");
    }
    Ok(())
}

pub(super) async fn run_training(
    work: PathBuf,
    out: PathBuf,
    input: serde_json::Value,
    mut candidate: Artifact,
) {
    let result: Result<Artifact> = async {
        tokio::fs::create_dir_all(work.parent().context("work parent missing")?).await?;
        tokio::fs::create_dir_all(out.parent().context("output parent missing")?).await?;
        // UUID path collisions fail rather than reuse/overwrite another build.
        tokio::fs::create_dir(&work).await?;
        tokio::fs::create_dir(&out).await?;
        tokio::fs::write(work.join("input.json"), input.to_string()).await?;
        push_log(&format!(
            "build={} base_model={} base_hub={} source_revision={} input_sha256={}",
            candidate.build_id,
            super::BASE_MODEL.name,
            super::BASE_MODEL.hub,
            candidate.source_revision,
            candidate.source_input_sha256
        ))
        .await;
        // The image supplies dependencies, not the training script. Reuse cached environments
        // offline; pull only when missing. Pin image id so concurrent tag updates cannot alter it.
        let mut inspect = Command::new("docker")
            .args([
                "image",
                "inspect",
                "--format",
                "{{.Id}}",
                &candidate.trainer_image,
            ])
            .kill_on_drop(true)
            .output()
            .await
            .context("cannot start Docker CLI (docker.sock + CLI required)")?;
        if !inspect.status.success() {
            push_log(&format!(
                "Dependency image missing; pulling {}",
                candidate.trainer_image
            ))
            .await;
            let mut pull = Command::new("docker");
            pull.args(["pull", &candidate.trainer_image]);
            run_cmd(pull).await?;
            inspect = Command::new("docker")
                .args([
                    "image",
                    "inspect",
                    "--format",
                    "{{.Id}}",
                    &candidate.trainer_image,
                ])
                .kill_on_drop(true)
                .output()
                .await
                .context("cannot start Docker CLI (docker.sock + CLI required)")?;
        }
        if !inspect.status.success() {
            bail!("cannot inspect trainer dependency image");
        }
        let image_id = String::from_utf8(inspect.stdout)?.trim().to_string();
        if !image_id.starts_with("sha256:") || image_id.len() != 71 {
            bail!("unexpected Docker image id");
        }
        candidate.trainer_image_id = image_id;
        let cpus = std::env::var("NLU_TRAINER_CPUS").unwrap_or_else(|_| "2".into());
        let memory = std::env::var("NLU_TRAINER_MEMORY").unwrap_or_else(|_| "4g".into());
        let host_work = docker_host_path(&work).await;
        let host_out = docker_host_path(&out).await;
        let mut run = Command::new("docker");
        run.args(["run", "--rm", "--cpus", &cpus, "--memory", &memory])
            // 底座与超参只经 env 传给训练脚本；input.json 的字段语义保持不变，
            // 也不新增第四个训练产物文件（三件套白名单在 artifact.rs 的 FILES）。
            .args(super::BASE_MODEL.env_args())
            .arg("-v")
            .arg(format!("{host_work}:/work"))
            .arg("-v")
            .arg(format!("{host_out}:/work/output"))
            .args(["--entrypoint", "python"])
            .arg(&candidate.trainer_image_id)
            .arg("-c")
            .arg(super::TRAINER_SOURCE);
        push_log(&format!(
            "开始训练（底座 {}，独立版本目录，不覆盖当前模型）…",
            super::BASE_MODEL.name
        ))
        .await;
        run_cmd(run).await?;
        let output = out.clone();
        let artifact = tokio::task::spawn_blocking(move || {
            let mut artifact = candidate.collect(&output)?;
            // 封版时刻 = 三件套校验全过、manifest 即将落盘。不给构建发起时刻：
            // 几十分钟训练会让"最新产物"显示成发起时间，正好抵消这个字段的用途。
            artifact.created_at = Some(super::artifact::now_rfc3339());
            artifact::write_manifest(&output, &artifact)?;
            Ok::<_, anyhow::Error>(artifact)
        })
        .await??;
        push_log(&format!(
            "训练产物验收完成 model_sha256={}；尚未启用，仍需独立效果测试",
            artifact.model_sha256()
        ))
        .await;
        Ok(artifact)
    }
    .await;
    if let Err(e) = &result {
        push_log(&format!("[失败] {e:#}")).await;
    }
    let mut j = job().lock().await;
    j.running = false;
    match result {
        Ok(artifact) => {
            j.state = "success";
            j.artifact = Some(artifact);
        }
        Err(e) => {
            j.state = "failed";
            j.error = format!("{e:#}");
        }
    }
    let _ = log_tx().send(BuildEvent::Done);
}

#[cfg(test)]
#[path = "training_tests.rs"]
mod tests;
