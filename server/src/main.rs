//! xiaozhi-server-rust 入口（单二进制、单进程）。
//!
//! 启动流程：[`load_config`] 加载配置 → 构建进程级共享引擎 [`crate::engine::Engines`]
//! （ASR/TTS/LLM，模型重、只加载一次）→ 启动 tokio 多线程 runtime（worker 数由
//! `[server].worker_threads` 控制，默认 2，避免占满低功耗主机）→ `axum::serve` 监听。
//!
//! ## 配置加载优先级（权威说明）
//!
//! 见 [`load_config_inner`]：
//! 1. 环境变量 `XIAOZHI_CONFIG` 指向的 TOML 文件（成功则记录 `config_path`）；
//! 2. 否则命令行 `--config <path>`；
//! 3. 两者都缺失或读取失败 → 回退 [`crate::config::Config::default()`]（/data/models 生产路径）。
//!
//! 任一级失败都告警后回退，保证进程总能起来；引擎在启动时构建，模型缺失会给出明确报错。
//! 返回的 `(Config, Option<config_path>)` 中，`config_path` 为 `None` 表示用的是内置默认，
//! 此时 `PUT /api/config` 无法持久化（需以 `--config` 指定文件后重启）。
//!
//! 环境变量覆盖（**仅影响本次运行，不写回文件**）：
//! - `XIAOZHI_CONFIG`：同优先级的配置文件路径。
//! - `XIAOZHI_ADMIN_DIR`：覆盖 `[server].admin_dir`（Docker 镜像内置 `/app/web` 即此机制）。
//! `GET /api/config` 展示的仍是文件原值，不会体现 env 覆盖。

mod app;
mod config;
mod engine;
mod plugins;

use anyhow::Result;
use axum::serve;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::engine::Engines;
use crate::app::ws::router;

fn main() -> Result<()> {
    // 配置在 runtime 构建前加载（worker_threads 需要它）。
    let (config, config_path) = load_config();

    // tokio 多线程 runtime：worker 线程数由 [server].worker_threads 控制（默认 2），
    // 避免默认按 CPU 核数拉满，在 N5105 这类 4 核主机上挤占其他服务。
    let workers = config.server.worker_threads.max(1) as usize;
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()?
        .block_on(run(config, config_path))
}

async fn run(config: Config, config_path: Option<String>) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!(
        "配置加载完成：ASR=SenseVoice, TTS=Kokoro, LLM=http, 监听=0.0.0.0:{}, tokio worker={} 线程",
        config.server.port,
        config.server.worker_threads.max(1)
    );

    let engines = Engines::new(&config, config_path)?;
    // 配置来源可见性：把实际加载的文件路径打到启动日志（"改配置不生效"类问题第一现场）
    match &engines.config_path {
        Some(p) => {
            if crate::app::ws::config_is_persistent(p) {
                tracing::info!("配置文件：{p}（挂载卷上，持久化 ✓）");
            } else {
                tracing::warn!(
                    "配置文件：{p} ⚠️ 不在挂载卷上——容器重建会丢失！请按 docker-compose.yml 挂载 ./server-data:/data 并设 XIAOZHI_CONFIG=/data/config.toml"
                );
            }
        }
        None => tracing::warn!(
            "未加载任何配置文件（XIAOZHI_CONFIG/--config 均未提供或读取失败）：使用内置默认配置，保存不会持久化（PUT /api/config 返回 400）"
        ),
    }
    // 发音人目录：读盘缓存/离线兜底 + 配置了控制台会话时后台刷新一次（失败非致命）
    crate::app::voices::init();
    crate::app::voices::spawn_startup_probe(engines.clone());
    let app = router(engines);

    // 监听：永远 0.0.0.0，唯一可配的是端口（[server].port，默认 8000）——
    // 容器/局域网可达性由端口映射或防火墙决定，不存在"绑错地址"一类的问题。
    let addr = format!("0.0.0.0:{}", config.server.port);
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            return Err(anyhow::anyhow!(e).context(format!(
                "绑定 {addr} 失败：端口被占用。检查是否已有实例在跑，或修改 [server].port。"
            )));
        }
        Err(e) => return Err(anyhow::anyhow!(e).context(format!("绑定 {addr} 失败"))),
    };
    tracing::info!("xiaozhi-server-rust 监听于 {addr}");
    serve(listener, app).await?;
    Ok(())
}

/// 配置加载优先级：`XIAOZHI_CONFIG` 环境变量 → `--config` 参数 → 内置默认（/data/models 生产路径）。
/// 返回加载到的配置与（若有）配置文件路径，供 `PUT /api/config` 写回使用。
fn load_config() -> (Config, Option<String>) {
    let (mut config, path) = load_config_inner();
    // [server].admin_dir 支持环境变量覆盖（Docker 镜像内置 XIAOZHI_ADMIN_DIR=/app/web，
    // 挂载的 config.toml 无需为容器单独改路径）。与 XIAOZHI_CONFIG 同语义：env 只影响
    // 本次运行的生效配置，不写回文件（GET /api/config 展示的仍是文件原值）。
    if let Ok(dir) = std::env::var("XIAOZHI_ADMIN_DIR") {
        if !dir.is_empty() {
            config.server.admin_dir = dir;
        }
    }
    (config, path)
}

fn load_config_inner() -> (Config, Option<String>) {
    if let Ok(p) = std::env::var("XIAOZHI_CONFIG") {
        if let Ok(c) = Config::load(&p) {
            return (c, Some(p));
        }
        tracing::warn!("加载配置文件 {p} 失败，回退内置默认配置");
    }
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--config") {
        if let Some(p) = args.get(pos + 1) {
            if let Ok(c) = Config::load(p) {
                return (c, Some(p.clone()));
            }
        }
    }
    (Config::default(), None)
}
