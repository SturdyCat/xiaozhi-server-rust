//! NLU 模型训练端点：`POST /api/nlu/model/build`（异步启动）+ `GET /api/nlu/model/build`（状态）。
//!
//! ## 流程（跑通「技能定义 → 训练 → 部署」闭环）
//!
//! 1. 读当前生效配置的 `[nlu]` 段，与内置提醒技能合并（[`NluConfig::with_builtins`]，
//!    与运行时技能目录同一份合并结果——训练标签与技能名必然一致）；
//! 2. 把 intents/slots 写成 `input.json` 落到数据目录 `<data>/nlu-train/`；
//! 3. 专属训练镜像：服务端**只 pull + run**，绝不在宿主 docker 里现场 build——
//!    build 是 CPU 重活且污染宿主镜像层。镜像由维护方预构建（仓库
//!    `tools/nlu-trainer` 的 Dockerfile，`docker build -t xiaozhi-nlu-trainer tools/nlu-trainer`
//!    或推到私有 registry），本机缺失时服务端自动 `docker pull`（`NLU_TRAINER_IMAGE`
//!    可指向任意 registry tag）。训练容器带 `--cpus/--memory` 上限，与实时推理隔离；
//!    产物三件套直接落 `<data>/models/nlu/`（挂载进容器的 `/work/output`）；
//! 4. 训练完成后把 `[nlu].bert_*` 指到该目录（`bert_vocab` 用 `tokenizer.json`）、
//!    `bert_enabled=true`，新会话生效。
//!
//! 前提：服务端容器能访问宿主 docker（compose 挂载 `/var/run/docker.sock`）且镜像内有
//! docker CLI。训练是 CPU 重活，与实时推理抢核——只在需要时手动触发。
//! ⚠️ 同一时间只允许一个训练任务（重复触发返回 409）。

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tokio::sync::Mutex;

use crate::config::Config;
use crate::engine::Engines;

const TRAINER_IMAGE: &str = "xiaozhi-nlu-trainer";

/// 一次训练任务的状态（进程内单例；重启清零，与 devices/sessions 同口径）。
#[derive(Default)]
struct BuildJob {
    running: bool,
    state: &'static str, // idle | running | success | failed
    log: String,         // 尾部日志（cap 8KB）
    error: String,
    output_dir: Option<String>,
}

impl BuildJob {
    fn new() -> Self {
        BuildJob {
            state: "idle",
            ..BuildJob::default()
        }
    }
}

static JOB: std::sync::OnceLock<Mutex<BuildJob>> = std::sync::OnceLock::new();

fn job() -> &'static Mutex<BuildJob> {
    JOB.get_or_init(|| Mutex::new(BuildJob::new()))
}

/// 数据目录：`[memory].db_path` 的父目录（与配置库同卷，默认 /data）。
fn data_dir(cfg: &Config) -> PathBuf {
    std::path::Path::new(&cfg.memory.db_path)
        .parent()
        .map(|p| p.to_path_buf())
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("/data"))
}

async fn push_log(line: &str) {
    // 只在写日志瞬间持锁：训练长达数十分钟，绝不能跨子进程持锁，
    // 否则 GET 状态端点（build_status 抢同一把锁）会被整个卡死。
    let mut job = job().lock().await;
    let job = &mut *job;
    job.log.push_str(&format!("{line}\n"));
    // 只留尾部 8KB：训练日志可能很长，状态接口只关心"最近发生了什么"。
    // 手动找 UTF-8 边界（floor_char_boundary 尚未稳定）。
    let limit = job.log.len().saturating_sub(8 * 1024);
    if limit > 0 {
        let mut cut = limit;
        while cut > 0 && !job.log.is_char_boundary(cut) {
            cut -= 1;
        }
        job.log.drain(..cut);
    }
}

/// 训练前置校验（纯函数，供端点与单测共用）：`[nlu]` 未启用 / 无技能时拒绝训练。
fn validate_buildable(nlu: &crate::plugins::nlu::NluConfig) -> Result<(), String> {
    if !nlu.enabled {
        return Err("[nlu].enabled=false：请先在配置里启用技能级联再训练模型".into());
    }
    if nlu.intents.is_empty() {
        return Err("[nlu].intents 为空：没有技能可训练".into());
    }
    Ok(())
}

/// `POST /api/nlu/model/build`：启动一次训练（异步，立即返回）。
pub(super) async fn start_build(State(engines): State<Arc<Engines>>) -> Response {
    let cfg = engines.live_config();
    let nlu = cfg.nlu.with_builtins();
    if let Err(msg) = validate_buildable(&nlu) {
        return axum::Json(json!({ "error": msg })).into_response();
    }
    {
        let mut job = job().lock().await;
        if job.running {
            return (
                axum::http::StatusCode::CONFLICT,
                axum::Json(json!({ "error": "已有训练任务在运行", "state": job.state })),
            )
                .into_response();
        }
        job.running = true;
        job.state = "running";
        job.error.clear();
        job.log.clear();
    }

    // 训练在独立 tokio 任务里跑（docker 命令是长阻塞子进程）。
    let work_dir = data_dir(&cfg).join("nlu-train");
    let out_dir = data_dir(&cfg).join("models").join("nlu");
    let intents = nlu
        .intents
        .iter()
        .map(|i| json!({ "name": i.name, "templates": i.templates }))
        .collect::<Vec<_>>();
    let slots = nlu
        .slots
        .iter()
        .map(|s| json!({ "name": s.name, "values": s.values, "optional": s.optional }))
        .collect::<Vec<_>>();
    tokio::spawn(async move {
        run_training(work_dir, out_dir, json!({ "intents": intents, "slots": slots })).await;
    });
    axum::Json(json!({ "started": true, "state": "running" })).into_response()
}

/// 子进程辅助：跑一条命令，把 stdout/stderr 追加进任务日志；返回是否成功。
async fn run_cmd(mut cmd: tokio::process::Command) -> anyhow::Result<()> {
    let prog = format!(
        "{} {}",
        cmd.as_std().get_program().to_string_lossy(),
        cmd.as_std()
            .get_args()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    );
    push_log(&format!("$ {prog}")).await;
    let out = cmd
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("docker 命令执行失败: {e}（容器内需要 docker CLI 与 /var/run/docker.sock）"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut lines: Vec<&str> = stdout.lines().chain(stderr.lines()).collect();
    // 留尾部 120 行：docker build 里 pip 失败的真实报错往往在更早的行，
    // 40 行会把 "ERROR: ..." 截掉，只剩退出码没法定位。
    let start = lines.len().saturating_sub(120);
    for line in lines.drain(start..) {
        push_log(line).await;
    }
    if !out.status.success() {
        anyhow::bail!("命令退出码 {:?}：{}", out.status.code(), stderr.trim());
    }
    Ok(())
}

async fn run_training(work_dir: PathBuf, out_dir: PathBuf, input: serde_json::Value) {
    let image = std::env::var("NLU_TRAINER_IMAGE").unwrap_or_else(|_| TRAINER_IMAGE.to_string());
    // 训练容器资源上限：训练是 CPU 重活，必须与实时推理隔离（可用 env 放宽）。
    let cpus = std::env::var("NLU_TRAINER_CPUS").unwrap_or_else(|_| "2".to_string());
    let memory = std::env::var("NLU_TRAINER_MEMORY").unwrap_or_else(|_| "4g".to_string());
    let result: anyhow::Result<()> = (|| async {
        tokio::fs::create_dir_all(&work_dir).await?;
        tokio::fs::create_dir_all(&out_dir).await?;
        tokio::fs::write(work_dir.join("input.json"), input.to_string()).await?;
        job().lock().await.output_dir = Some(out_dir.display().to_string());

        // 镜像由维护方预构建（tools/nlu-trainer 的 Dockerfile），这里只 pull，不 build。
        let inspect = tokio::process::Command::new("docker")
            .args(["image", "inspect", &image])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        match inspect {
            Ok(s) if s.success() => {}
            Ok(_) => {
                push_log(&format!("镜像 {image} 不存在，docker pull（首次较慢）…")).await;
                let mut pull = tokio::process::Command::new("docker");
                pull.args(["pull", &image]);
                run_cmd(pull).await?;
            }
            Err(e) => anyhow::bail!("无法调用 docker: {e}（需要 docker CLI 与 docker.sock 挂载）"),
        }

        push_log("开始训练（HF 拉权重 + 微调 + ONNX int8 导出）…").await;
        let mut run = tokio::process::Command::new("docker");
        run.args(["run", "--rm", "--cpus", &cpus, "--memory", &memory])
            .arg("-v")
            .arg(format!("{}:/work", work_dir.display()))
            .env("HF_ENDPOINT", "https://hf-mirror.com")
            .args([&image]);
        run_cmd(run).await?;

        // 产物三件套齐全才算成功。
        for f in ["model.onnx", "tokenizer.json", "labels.txt"] {
            if !out_dir.join(f).is_file() {
                anyhow::bail!("训练完成但缺少产物 {f}（目录 {}）", out_dir.display());
            }
        }
        push_log(&format!(
            "训练完成：{}（model.onnx / tokenizer.json / labels.txt）",
            out_dir.display()
        ))
        .await;
        job().lock().await.state = "success";
        Ok(())
    })()
    .await;

    {
        let mut job = job().lock().await;
        job.running = false;
        if let Err(e) = &result {
            push_log(&format!("[失败] {e:#}")).await;
            job.error = format!("{e:#}");
            job.state = "failed";
        }
    }
}

/// `GET /api/nlu/model/build`：当前训练任务状态 + 尾部日志 + 产物目录。
pub(super) async fn build_status() -> Response {
    let job = job().lock().await;
    axum::Json(json!({
        "state": job.state,
        "running": job.running,
        "error": job.error,
        "log_tail": job.log,
        "output_dir": job.output_dir,
        "expected_config": {
            "bert_enabled": true,
            "bert_model": "<output_dir>/model.onnx",
            "bert_vocab": "<output_dir>/tokenizer.json",
            "bert_labels": "<output_dir>/labels.txt",
        },
    }))
    .into_response()
}

#[cfg(test)]
mod tests;
