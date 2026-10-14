//! xiaozhi-server-rust 入口（单二进制、单进程）。
//!
//! 启动流程：[`load_config`] 加载配置 → 构建进程级共享引擎 [`crate::engine::Engines`]
//! （ASR/TTS/LLM，模型重、只加载一次）→ 启动 tokio 多线程 runtime（worker 数由
//! `[server].worker_threads` 控制，默认 2，避免占满低功耗主机）→ `axum::serve` 监听。
//!
//! ## 配置加载优先级（权威说明）
//!
//! 见 [`load_config_inner`]：
//! 1. 环境变量 `XIAOZHI_CONFIG` 指向的 TOML 文件（成功则装配文件存储后端）；
//! 2. 否则命令行 `--config <path>`；
//! 3. 两者都缺失或读取失败 → 回退 [`crate::config::Config::default()`]（/data/models 生产路径）。
//!
//! 任一级失败都告警后回退，保证进程总能起来；引擎在启动时构建，模型缺失会给出明确报错。
//! 返回的 `(Config, Arc<dyn ConfigStore>)` 中，存储后端的 `location()` 为 `None`
//! （[`crate::config::store::DefaultConfigStore`]）表示用的是内置默认，
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

use std::sync::Arc;

use anyhow::Result;
use axum::serve;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

use crate::config::store::sqlite::SqliteConfigStore;
use crate::config::store::{ConfigStore, DefaultConfigStore, FileConfigStore};
use crate::config::Config;
use crate::engine::Engines;
use crate::app::ws::router;

fn main() -> Result<()> {
    // 配置在 runtime 构建前加载（worker_threads 需要它）。
    let (config, config_store) = load_config();

    // tokio 多线程 runtime：worker 线程数由 [server].worker_threads 控制（默认 2），
    // 避免默认按 CPU 核数拉满，在 N5105 这类 4 核主机上挤占其他服务。
    let workers = config.server.worker_threads.max(1) as usize;
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()?
        .block_on(run(config, config_store))
}

async fn run(config: Config, config_store: Arc<dyn ConfigStore>) -> Result<()> {
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

    let engines = Engines::new(&config, config_store)?;
    // 延迟/事件聚合（P1-1）：进程最早的订阅者——spawn 越早，broadcast 环形缓冲越不丢
    // 早期事件（LAGGED 只告警；聚合口径是近似值，见 app/metrics 模块注释）。
    tokio::spawn(crate::app::metrics::subscriber(
        engines.metrics.clone(),
        engines.event_bus.subscribe(),
    ));
    // 配置来源可见性：把实际加载的配置来源打到启动日志（"改配置不生效"类问题第一现场）
    match engines.config_store.location() {
        Some(p) => {
            let kind = engines.config_store.backend();
            if crate::app::ws::config_is_persistent(p) {
                tracing::info!("{kind} 配置存放于：{p}（挂载卷上，持久化 ✓）");
            } else {
                tracing::warn!(
                    "{kind} 配置存放于：{p} ⚠️ 不在挂载卷上——容器重建会丢失！请按 docker-compose.yml 挂载 ./server-data:/data"
                );
            }
        }
        None => tracing::warn!(
            "未加载任何配置来源（XIAOZHI_CONFIG/--config 均未提供或读取失败）：使用内置默认配置，保存不会持久化（PUT /api/config 返回 400）"
        ),
    }
    // 发音人目录：读盘缓存/离线兜底 + 配置了控制台会话时后台刷新一次（失败非致命）
    crate::app::voices::init();
    crate::app::voices::spawn_startup_probe(engines.clone());
    // 唤醒问候语的音频复用是否可用（"为什么重启后又要重新合成"的第一现场）：
    // 只有加载了配置文件才有落盘目录（与 PUT /api/config 的持久化前提同源）。
    match (engines.hot_audio.is_persistent(), engines.hot_audio.dir()) {
        (true, Some(dir)) => tracing::info!(
            "热句音频（唤醒问候语）落盘目录：{}（重启后仍免合成 ✓）",
            dir.display()
        ),
        _ => tracing::warn!(
            "未加载配置文件：唤醒问候语（[wakeup]）只能在进程内复用音频，重启后需重新合成"
        ),
    }
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

/// 配置加载优先级（SQLite 配置后端为默认真源）：
///
/// 1. 解析 TOML 配置来源（`XIAOZHI_CONFIG` → `--config`）——**仅用于兼容导入与读取
///    `[memory].db_path` 作为配置库默认路径**；
/// 2. 解析配置库路径：`XIAOZHI_CONFIG_DB` → `--config-db` → `[memory].db_path`
///    → 兜底 `/data/xiaozhi.db`；
/// 3. 打开 SQLite 配置库：空库且有旧 TOML → **兼容导入**（source=bootstrap，DB 优先）；
///    打开失败则降级到旧的 TOML 文件后端（保证进程总能起来）。
///
/// 返回加载到的配置与配置**存储后端**（SQLite / 文件 / 内置默认），供 `PUT /api/config` 写回使用。
fn load_config() -> (Config, Arc<dyn ConfigStore>) {
    let (mut config, store) = load_config_inner();
    // [server].admin_dir 支持环境变量覆盖（Docker 镜像内置 XIAOZHI_ADMIN_DIR=/app/web，
    // 挂载的 config.toml 无需为容器单独改路径）。与 XIAOZHI_CONFIG 同语义：env 只影响
    // 本次运行的生效配置，不写回文件（GET /api/config 展示的仍是原值）。
    if let Ok(dir) = std::env::var("XIAOZHI_ADMIN_DIR") {
        if !dir.is_empty() {
            config.server.admin_dir = dir;
        }
    }
    (config, store)
}

fn load_config_inner() -> (Config, Arc<dyn ConfigStore>) {
    // 1) 解析 TOML 配置来源（兼容导入用；同时读 [memory].db_path）
    let toml_path: Option<String> = std::env::var("XIAOZHI_CONFIG")
        .ok()
        .filter(|p| !p.is_empty())
        .or_else(|| {
            let args: Vec<String> = std::env::args().collect();
            args.iter()
                .position(|a| a == "--config")
                .and_then(|pos| args.get(pos + 1).cloned())
                .filter(|p| !p.is_empty())
        });
    let toml_config: Option<Config> =
        toml_path.as_ref().and_then(|p| Config::load(p).ok());

    // 2) 解析配置库路径
    let mut db_path: String = std::env::var("XIAOZHI_CONFIG_DB")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let args: Vec<String> = std::env::args().collect();
            args.iter()
                .position(|a| a == "--config-db")
                .and_then(|pos| args.get(pos + 1).cloned())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            // 默认与 memory 共用同一 db 文件（用户要的"与 memory 共用 db"）；
            // :memory: 不能当配置库（进程内临时、重建即丢）→ 跳过。
            toml_config
                .as_ref()
                .map(|c| c.memory.db_path.trim().to_string())
                .filter(|s| !s.is_empty() && s != ":memory:")
        })
        .unwrap_or_else(|| "/data/xiaozhi.db".to_string());

    // :memory: 兜底：回退到文件路径并告警
    if db_path == ":memory:" {
        tracing::warn!(
            "配置的 [memory].db_path 为 :memory:，不能作为配置库（进程内临时、重建即丢）；回退到 /data/xiaozhi.db"
        );
        db_path = "/data/xiaozhi.db".to_string();
    }

    // 3) 打开 SQLite 配置库；失败降级到文件/默认后端
    match SqliteConfigStore::open(&db_path) {
        Ok(store) => {
            // 空库 + 旧 TOML → 兼容导入（DB 优先：导入后即成为真源，旧 TOML 不再被读取）
            if store.is_empty() {
                if let Some(cfg) = &toml_config {
                    match store.bootstrap_import(cfg) {
                        Ok(()) => tracing::info!(
                            "已从旧配置文件 {} 导入配置到数据库 {}（source=bootstrap，后续以数据库为准）",
                            toml_path.as_deref().unwrap_or(""),
                            db_path
                        ),
                        Err(e) => {
                            tracing::warn!("首启导入 TOML 配置到数据库失败（使用内置默认）: {e:#}")
                        }
                    }
                } else {
                    tracing::info!(
                        "配置库 {} 为空且无旧配置文件，将以内置默认配置启动（保存后即落库）",
                        db_path
                    );
                }
            }
            let config = match store.load() {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("读取配置库失败，回退内置默认配置: {e:#}");
                    Config::default()
                }
            };
            (config, store)
        }
        Err(e) => {
            tracing::warn!(
                "打开 SQLite 配置库 {db_path} 失败，降级到文件/默认后端（旧行为）: {e:#}"
            );
            if let Some(p) = &toml_path {
                if let Ok(c) = Config::load(p) {
                    return (c, Arc::new(FileConfigStore::new(p.clone())));
                }
            }
            (Config::default(), Arc::new(DefaultConfigStore))
        }
    }
}
