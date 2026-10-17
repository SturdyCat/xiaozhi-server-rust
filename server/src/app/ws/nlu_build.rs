//! NLU 模型训练端点：`POST /api/nlu/model/build`（异步启动）+ `GET /api/nlu/model/build`（状态）
//! + `GET /api/nlu/model/build/log`（**WebSocket 实时日志流**，无需轮询）。
//!
//! ## 流程（跑通「技能定义 → 训练 → 部署」闭环）
//!
//! 1. 读当前生效配置的 `[nlu]` 段，与内置提醒技能合并（[`NluConfig::with_builtins`]，
//!    与运行时技能目录同一份合并结果——训练标签与技能名必然一致）；
//! 2. 把 intents/slots 写成 `input.json` 落到数据目录 `<data>/nlu-train/`；
//! 3. **固定官方镜像 + 共享卷依赖**（不构建、不拉取任何专属镜像）：训练脚本与
//!    `requirements.txt` 随服务端镜像内置（`/opt/nlu-trainer`，见根 Dockerfile），
//!    每次训练复制进 `<data>/nlu-train/`；Python 依赖（torch 等）首次训练时由官方
//!    `python:3.11-slim` 容器装进 `<data>/nlu-train-env/`（共享卷，跨任务复用，
//!    清单变化自动重装）。训练就是一条 `docker run --rm`：挂工作目录 + 依赖卷 +
//!    产物目录，`--cpus/--memory` 上限与实时推理隔离；跑完容器自动删除，
//!    产物三件套留在 `<data>/models/nlu/`；
//! 4. 训练完成后把 `[nlu].bert_*` 指到该目录（`bert_vocab` 用 `tokenizer.json`）、
//!    `bert_enabled=true`，新会话生效。
//!
//! 前提：服务端容器能访问宿主 docker（compose 挂载 `/var/run/docker.sock`）且镜像内有
//! docker CLI。训练是 CPU 重活，与实时推理抢核——只在需要时手动触发。
//! ⚠️ 同一时间只允许一个训练任务（重复触发返回 409）。

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tokio::sync::{broadcast, Mutex};

use crate::config::Config;
use crate::engine::Engines;

/// 运行训练的**官方**基础镜像（Docker Hub 公共 tag，任何环境都能 pull 到；
/// 可用 `NLU_TRAINER_IMAGE` 指向内网镜像仓库的同版本 tag）。
const TRAINER_IMAGE: &str = "python:3.11-slim";
/// 服务端镜像内置的训练资产目录（train.py + requirements.txt，见根 Dockerfile COPY）。
const TRAINER_ASSETS: &str = "/opt/nlu-trainer";

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

/// WS 日志订阅者事件：一行日志，或任务结束（收到后服务端推终态帧并关闭）。
#[derive(Clone)]
enum BuildEvent {
    Log(String),
    /// 收尾信号（载荷仅为区分变体，值不读取）。
    Done(()),
}

/// 日志广播通道（进程内单例）：`push_log` 发布，WS 客户端订阅实时接收。
/// 容量 256 行——慢客户端丢行时由 [`build_log_ws`] 补一条 lag 提示，不阻塞训练。
static LOG_TX: std::sync::OnceLock<broadcast::Sender<BuildEvent>> = std::sync::OnceLock::new();

fn log_tx() -> &'static broadcast::Sender<BuildEvent> {
    LOG_TX.get_or_init(|| broadcast::channel(256).0)
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
    // 只在写日志瞬间持锁（显式块尾即释放）：训练长达数十分钟，绝不能跨子进程持锁，
    // 否则 GET 状态端点（build_status 抢同一把锁）会被整个卡死。
    {
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
    // 镜像进全局日志流（`GET /api/logs`）：管理台/AI 在服务器日志里也能看到训练进度。
    tracing::info!(target: "nlu_build", "{line}");
    // 广播给训练 WS 订阅者（无订阅者时 Err 被忽略）。锁已在块尾释放，不会互锁。
    let _ = log_tx().send(BuildEvent::Log(line.to_string()));
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
    let data = data_dir(&cfg);
    let work_dir = data.join("nlu-train");
    let env_dir = data.join("nlu-train-env");
    let out_dir = data.join("models").join("nlu");
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
        run_training(work_dir, env_dir, out_dir, json!({ "intents": intents, "slots": slots })).await;
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

/// 依赖是否需要（重）装：依赖卷里的清单标记与当前 requirements 不一致时重装。
/// 纯函数供单测；标记文件内容就是完整 requirements 文本（原样比对，不做哈希）。
fn deps_up_to_date(marker: Option<&str>, requirements: &str) -> bool {
    marker.map(str::trim) == Some(requirements.trim())
}

/// 从服务端内置资产目录把训练脚本与依赖清单复制进工作目录（每次训练都刷新，
/// 保证脚本版本与部署的服务端一致）；返回 requirements 文本。
async fn stage_assets(assets: &str, work_dir: &std::path::Path) -> anyhow::Result<String> {
    let req_bytes = tokio::fs::read(format!("{assets}/requirements.txt"))
        .await
        .map_err(|e| anyhow::anyhow!("读取内置训练资产失败（{assets}/requirements.txt）: {e}"))?;
    let requirements = String::from_utf8(req_bytes)?;
    tokio::fs::copy(format!("{assets}/train.py"), work_dir.join("train.py"))
        .await
        .map_err(|e| anyhow::anyhow!("复制训练脚本失败（{assets}/train.py）: {e}"))?;
    tokio::fs::write(work_dir.join("requirements.txt"), &requirements).await?;
    Ok(requirements)
}

/// 把传给宿主 docker 的挂载路径从容器内路径换算成宿主真实路径。
///
/// 服务端自身跑在容器里时（compose 把宿主 `./server-data` 挂为 `/data`），
/// `docker.sock` 是**宿主** docker：`-v /data/nlu-train:/work` 会在宿主上解析，
/// 得到一个自动新建的空目录，容器内写好的脚本/清单根本不在里面。自省
/// `/proc/self/mountinfo` 找到覆盖该路径的最长 bind mount，把前缀替换为宿主源路径；
/// 不在容器里（读不到 mountinfo 或无匹配）时原样返回。纯文本解析供单测。
fn mount_host_path(path: &str, mountinfo: &str) -> String {
    let mut best: Option<(usize, String)> = None;
    for line in mountinfo.lines() {
        // 格式：… root mountpoint opts … - fstype source superopts
        let Some((left, right)) = line.split_once(" - ") else { continue };
        let lf: Vec<&str> = left.split_whitespace().collect();
        let rf: Vec<&str> = right.split_whitespace().collect();
        if lf.len() < 5 || rf.len() < 2 {
            continue;
        }
        let (root, mnt, source) = (lf[3], lf[4], rf[1]);
        // 先匹配再取后缀：mountpoint 可能比目标路径长（兄弟挂载点），先切片会越界。
        let Some(suffix) = path.strip_prefix(mnt) else { continue };
        if !(suffix.is_empty() || suffix.starts_with('/')) {
            continue;
        }
        let rel = suffix.strip_prefix('/').unwrap_or("");
        // root 是该挂载在源文件系统里的子树位置（bind 目录挂载时为 /）；
        // 宿主真实路径 = 源 + 子树 + 容器内相对路径。
        let base = if root == "/" {
            source.to_string()
        } else {
            format!("{source}{}", root.trim_end_matches('/'))
        };
        let mapped = if rel.is_empty() { base } else { format!("{base}/{rel}") };
        if best.as_ref().map_or(true, |(len, _)| mnt.len() > *len) {
            best = Some((mnt.len(), mapped));
        }
    }
    best.map(|(_, p)| p).unwrap_or_else(|| path.to_string())
}

/// 精确路径换算：查**自己容器**的 inspect（经 docker.sock），`Mounts[]` 里
/// Destination→Source 就是宿主真实 bind 源。覆盖 NAS 等设备子路径挂载场景
/// （mountinfo 里 source 只是 `/dev/xxx` 设备名，设备在宿主的挂载点容器内
/// 不可见，纯文本解析原理上不可解）。裸机或查不到时返回 None。
async fn mount_from_own_container(path: &str) -> Option<String> {
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
    let mounts = v.get(0)?.get("Mounts")?.as_array()?;
    let pairs: Vec<(String, String)> = mounts
        .iter()
        .filter_map(|m| {
            Some((
                m.get("Destination")?.as_str()?.to_string(),
                m.get("Source")?.as_str()?.to_string(),
            ))
        })
        .collect();
    map_via_mounts(path, &pairs)
}

/// 在 (容器内挂载点, 宿主源路径) 映射表中取覆盖 path 的最长挂载点做前缀替换。
/// 纯函数供单测。
fn map_via_mounts(path: &str, mounts: &[(String, String)]) -> Option<String> {
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

async fn docker_host_path(path: &std::path::Path) -> String {
    let s = path.display().to_string();
    if let Some(mapped) = mount_from_own_container(&s).await {
        return mapped;
    }
    match tokio::fs::read_to_string("/proc/self/mountinfo").await {
        Ok(info) => mount_host_path(&s, &info),
        Err(_) => s, // 裸机部署（不在容器里）：路径本来就是宿主路径。
    }
}

async fn run_training(
    work_dir: PathBuf,
    env_dir: PathBuf,
    out_dir: PathBuf,
    input: serde_json::Value,
) {
    let image = std::env::var("NLU_TRAINER_IMAGE").unwrap_or_else(|_| TRAINER_IMAGE.to_string());
    let assets = std::env::var("NLU_TRAINER_ASSETS").unwrap_or_else(|_| TRAINER_ASSETS.to_string());
    // 训练容器资源上限：训练是 CPU 重活，必须与实时推理隔离（可用 env 放宽）。
    let cpus = std::env::var("NLU_TRAINER_CPUS").unwrap_or_else(|_| "2".to_string());
    let memory = std::env::var("NLU_TRAINER_MEMORY").unwrap_or_else(|_| "4g".to_string());
    // 容器内挂载点：/work=工作目录（脚本+input） /opt/env=共享依赖卷 /work/output=产物。
    let marker_path = env_dir.join(".requirements");
    let result: anyhow::Result<()> = (|| async {
        tokio::fs::create_dir_all(&work_dir).await?;
        tokio::fs::create_dir_all(&env_dir).await?;
        tokio::fs::create_dir_all(&out_dir).await?;
        tokio::fs::write(work_dir.join("input.json"), input.to_string()).await?;
        let requirements = stage_assets(&assets, &work_dir).await?;
        job().lock().await.output_dir = Some(out_dir.display().to_string());

        // 基础镜像缺失时 pull 一次（官方公共 tag；内网环境可把 NLU_TRAINER_IMAGE
        // 指到镜像仓库同版本 tag，同样只 pull 不 build）。
        let inspect = tokio::process::Command::new("docker")
            .args(["image", "inspect", &image])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        match inspect {
            Ok(s) if s.success() => {}
            Ok(_) => {
                push_log(&format!("基础镜像 {image} 不存在，docker pull（首次较慢）…")).await;
                let mut pull = tokio::process::Command::new("docker");
                pull.args(["pull", &image]);
                run_cmd(pull).await?;
            }
            Err(e) => anyhow::bail!("无法调用 docker: {e}（需要 docker CLI 与 docker.sock 挂载）"),
        }

        // 依赖引导：首次训练（或 requirements 变化）时把依赖装进共享卷，之后离线复用。
        // torch 主源走官方 CPU 轮子仓库，失败回退清华 PyPI 镜像。
        let marker = tokio::fs::read_to_string(&marker_path).await.ok();
        // 宿主 docker 按宿主路径解析挂载：容器内路径必须先换算（见 mount_host_path）。
        let host_work = docker_host_path(&work_dir).await;
        let host_env = docker_host_path(&env_dir).await;
        let host_out = docker_host_path(&out_dir).await;
        if !deps_up_to_date(marker.as_deref(), &requirements) {
            push_log("安装训练依赖到共享卷（仅首次或清单变化时，torch 较大请耐心）…").await;
            let mut pip = tokio::process::Command::new("docker");
            pip.args(["run", "--rm", "--cpus", &cpus, "--memory", &memory])
                .arg("-v")
                .arg(format!("{}:/work", host_work))
                .arg("-v")
                .arg(format!("{}:/opt/env", host_env))
                .env("PIP_NO_CACHE_DIR", "1")
                .env("PIP_RETRIES", "5")
                .env("PIP_TIMEOUT", "60")
                .args([&image, "sh", "-lc", &format!(
                    "(pip install --index-url https://download.pytorch.org/whl/cpu \
                     -r /work/requirements.txt --target /opt/env \
                     || pip install -i https://pypi.tuna.tsinghua.edu.cn/simple \
                     -r /work/requirements.txt --target /opt/env)"
                )]);
            run_cmd(pip).await?;
            tokio::fs::write(&marker_path, &requirements).await?;
        }

        // 训练：PYTHONPATH 指向共享依赖卷；产物直接写进挂载的正式产物目录。
        push_log("开始训练（HF 拉权重 + 微调 + ONNX int8 导出）…").await;
        let mut run = tokio::process::Command::new("docker");
        run.args(["run", "--rm", "--cpus", &cpus, "--memory", &memory])
            .arg("-v")
            .arg(format!("{}:/work", host_work))
            .arg("-v")
            .arg(format!("{}:/work/output", host_out))
            .arg("-v")
            .arg(format!("{}:/opt/env", host_env))
            .env("PYTHONPATH", "/opt/env")
            .env("HF_ENDPOINT", "https://hf-mirror.com")
            .env("MODEL_NAME", "hfl/rbt3")
            .args([&image, "python", "/work/train.py"]);
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

    // 先写日志（push_log 内部要抢同一把锁），再拿锁收尾——tokio Mutex 不可重入，
    // 持锁调 push_log 会自死锁，GET 状态端点会被永久卡住。
    if let Err(e) = &result {
        push_log(&format!("[失败] {e:#}")).await;
    }
    let mut job = job().lock().await;
    job.running = false;
    if let Err(e) = &result {
        job.error = format!("{e:#}");
        job.state = "failed";
    }
    // 通知 WS 订阅者任务已结束（客户端收到后拿终态并断开）。
    let _ = log_tx().send(BuildEvent::Done(()));
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

/// 状态快照（GET 端点与 WS 终态帧共用同一份 JSON 形状）。
fn status_json(job: &BuildJob) -> serde_json::Value {
    json!({
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
    })
}

/// `GET /api/nlu/model/build/log`：WebSocket 实时训练日志。
///
/// 帧协议（全部为 JSON 文本帧）：`{"type":"log","line":…}` 逐行推送；
/// `{"type":"state",…}` 为快照/终态（字段与 `GET /api/nlu/model/build` 一致），
/// 终态帧之后服务端主动关闭连接。
pub(super) async fn build_log_ws(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(stream_build_log)
}

/// 推送一帧当前状态快照（与 GET 端点同形状）。
async fn send_status(socket: &mut WebSocket) {
    let payload = {
        let job = job().lock().await;
        status_json(&job)
    };
    let _ = socket.send(Message::Text(payload.to_string().into())).await;
}

async fn stream_build_log(mut socket: WebSocket) {
    // 先订阅广播再快照，避免"快照后、订阅前"的日志窗口丢失（重叠行由快照去重）。
    let mut rx = log_tx().subscribe();
    // 快照：任务已结束则推终态后立即关闭；运行中则继续推实时日志。
    if !job().lock().await.running {
        send_status(&mut socket).await;
        let _ = socket.send(Message::Close(None)).await;
        return;
    }
    loop {
        match rx.recv().await {
            Ok(BuildEvent::Log(line)) => {
                let frame = json!({ "type": "log", "line": line }).to_string();
                if socket.send(Message::Text(frame.into())).await.is_err() {
                    return; // 客户端断开：训练不受影响，广播通道自动回收。
                }
            }
            Ok(BuildEvent::Done(())) => {
                send_status(&mut socket).await;
                let _ = socket.send(Message::Close(None)).await;
                return;
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                // 慢客户端丢行：提示缺口，不中断流。
                let frame = json!({ "type": "log", "line": format!("…（日志过快，丢失 {n} 行）") })
                    .to_string();
                if socket.send(Message::Text(frame.into())).await.is_err() {
                    return;
                }
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

#[cfg(test)]
mod tests;
