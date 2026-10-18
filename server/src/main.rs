//! xiaozhi-server-rust 入口（单二进制、单进程）。
//!
//! 启动流程：[`load_config`] 加载配置 → 构建进程级共享引擎 [`crate::engine::Engines`]
//! （ASR/TTS/LLM，模型重、只加载一次）→ 启动 tokio 多线程 runtime（worker 数由
//! `[server].worker_threads` 控制，默认 2，避免占满低功耗主机）→ `axum::serve` 监听。
//!
//! ## 配置加载（权威说明）
//!
//! 见 [`load_config_inner`]：**唯一真源是 SQLite 配置库**（`data.db`），不存在外置
//! config.toml 文件来源；DB 打不开则进程直接报错退出（配置是必需能力，无降级）。
//! DB 路径：`XIAOZHI_CONFIG_DB` → `--config-db` → `/data/data.db`。
//! 空库时自举以内置 [`crate::config::Config::default()`] 落库（首次安装即有完整默认配置）。
//!
//! 环境变量覆盖（**仅影响本次运行，不写回**）：
//! - `XIAOZHI_ADMIN_DIR`：覆盖 `[server].admin_dir`（Docker 镜像内置 `/app/web` 即此机制）。

mod app;
mod config;
mod engine;
mod plugins;

use std::sync::Arc;

use anyhow::Result;
use axum::serve;
use tokio::net::TcpListener;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::EnvFilter;

use crate::app::ws::router;
use crate::config::store::sqlite::SqliteConfigStore;
use crate::config::store::ConfigStore;
use crate::config::Config;
use crate::engine::Engines;

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
    // registry + 各层手工组装：日志采集层（/api/logs WS）需要插在 fmt 之前，
    // SubscriberBuilder 的 `.with` 只接受格式层，无法直接挂自定义 Layer。
    // 默认 info，但压掉 ort::logging（ONNX 会话初始化的分配明细等 INFO 噪声，
    // 只留 WARN+）；RUST_LOG 显式设置时完全尊重用户配置。
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,ort=warn"));
    let subscriber = tracing_subscriber::registry()
        .with(filter) // 全局日志采集层：每条 INFO+ 事件进入环形缓冲 + 广播，
        // 供 `GET /api/logs` WS 客户端（macApp / AI 调试）实时读取。
        .with(crate::app::ws::log_stream::layer())
        .with(tracing_subscriber::fmt::layer());
    tracing::subscriber::set_global_default(subscriber)?;

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
        None => tracing::warn!("配置库路径不可见（location() == None）：配置将无法持久化"),
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
    // 运行端口落盘（HEALTHCHECK 消费）：镜像 HEALTHCHECK 无法读 SQLite 配置库，
    // 启动时把实际监听端口写到配置库同目录的 `.runtime_port`，健康检查读它——
    // 改 [server].port 后无需再同步改 Dockerfile（读不到文件时健康检查兜底 8000）。
    if let Some(p) = engines.config_store.location() {
        if let Some(dir) = std::path::Path::new(&p).parent() {
            if let Err(e) =
                std::fs::write(dir.join(".runtime_port"), config.server.port.to_string())
            {
                tracing::warn!("运行端口写入 .runtime_port 失败（健康检查将退回 8000）：{e}");
            }
        }
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
    // 把用户真正要填的连接地址打出来：macApp/设备需要 IP:port（监听永远 0.0.0.0，
    // 打 bind 地址没意义）。IP 无配置项，动态探测部署机默认出口。
    match crate::app::local_ipv4() {
        Some(ip) => tracing::info!(
            "局域网连接地址（macApp / 设备填写）: ws://{ip}:{}",
            config.server.port
        ),
        None => tracing::warn!(
            "无法探测局域网 IPv4（容器部署常见），macApp 请手动填宿主机 IP，端口 {}",
            config.server.port
        ),
    }
    // EchoMuse 设备发现（`[echomuse].enabled=true` 时）：广播 `_emcontroller._tcp`，
    // emOS 设备在同网段即可发现本服务器。失败只告警（设备也可凭 endpoint 文件直连）。
    if config.echomuse.enabled {
        crate::app::echomuse::spawn_mdns(config.server.port);
    }
    serve(listener, app).await?;
    Ok(())
}

/// 配置加载（SQLite 配置库为**唯一**真源，无外置文件配置）：
///
/// 1. 解析配置库路径：`XIAOZHI_CONFIG_DB` → `--config-db` → 兜底 `/data/data.db`；
/// 2. 打开 SQLite 配置库：空库（首次安装）自举写入内置 `Config::default()` 后再读；
/// 3. 打开失败 → **直接报错退出**（配置是必需能力；不再降级到 TOML 文件后端——
///    外置 config.toml 配置逻辑已移除，`data.db` 同时承载配置与 memory）。
///
/// 返回加载到的配置与配置存储后端（SQLite），供 `PUT /api/config` 写回使用。
fn load_config() -> (Config, Arc<dyn ConfigStore>) {
    let (mut config, store) = load_config_inner();
    // [server].admin_dir 支持环境变量覆盖（Docker 镜像内置 XIAOZHI_ADMIN_DIR=/app/web）。
    // env 只影响本次运行的生效配置，不写回（GET /api/config 展示的仍是库内原值）。
    if let Ok(dir) = std::env::var("XIAOZHI_ADMIN_DIR") {
        if !dir.is_empty() {
            config.server.admin_dir = dir;
        }
    }
    (config, store)
}

fn load_config_inner() -> (Config, Arc<dyn ConfigStore>) {
    // 1) 解析配置库路径
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
        .unwrap_or_else(|| "/data/data.db".to_string());

    // :memory: 兜底：回退到文件路径并告警（进程内临时、重建即丢，不能当配置库）
    if db_path == ":memory:" {
        tracing::warn!(
            "XIAOZHI_CONFIG_DB/--config-db 指向 :memory:，不能作为配置库（进程内临时、重建即丢）；回退到 /data/data.db"
        );
        db_path = "/data/data.db".to_string();
    }

    // 2) 打开 SQLite 配置库（失败即退出——没有可用的配置真源就无法对外承诺持久化）
    match SqliteConfigStore::open(&db_path) {
        Ok(store) => {
            // 首次安装自举：Docker 首启没有 data.db 时，把内置默认配置立即落库
            // （cfg_documents + source="bootstrap"），而非等首次保存。失败不阻断启动：
            // load() 对空库本就返回默认值，只是少了这条可回滚的初始版本。
            match store.bootstrap_default_if_empty() {
                Ok(true) => {
                    tracing::info!("配置库 {db_path} 为空，已写入内置默认配置（首次安装自举）")
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("空配置库自举写默认配置失败（以内置默认启动）: {e:#}"),
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
            tracing::error!("打开 SQLite 配置库 {db_path} 失败: {e:#}");
            panic!(
                "无法打开配置库 {db_path}（配置+memory 唯一真源）：{e:#}。\
                 请检查路径所在目录是否存在且可写（容器部署请挂载目录卷，如 ./server-data:/data）。"
            );
        }
    }
}
