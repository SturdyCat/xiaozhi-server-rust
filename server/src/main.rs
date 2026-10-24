//! xiaozhi-server-rust 入口（单二进制、单进程）。
//!
//! 启动流程：[`load_config`] 加载配置 → 构建进程级共享引擎 [`crate::runtime::Engines`]
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

mod workflow;
mod device;
mod gateway;
mod config;
mod benchmark;
mod media;
mod observability;
mod runtime;
mod plugins;
mod serve;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use anyhow::Result;
use tokio::net::TcpListener;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::EnvFilter;

use crate::gateway::ws::router;
use crate::config::store::sqlite::SqliteConfigStore;
use crate::config::store::ConfigStore;
use crate::config::Config;
use crate::runtime::Engines;

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
    // 默认 info，但压掉 ort::logging（ONNX Runtime C 库转发的会话初始化明细，
    // 每次加载模型上百行 INFO）；ort 本体的 dylib 加载提示保留 INFO。
    // RUST_LOG 显式设置时完全尊重用户配置——所以镜像/compose 不能内置 RUST_LOG，
    // 否则这个兜底永不生效（2026-10 ort::logging 刷屏事故根因，见 default_log_filter）。
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_log_filter()));
    let subscriber = tracing_subscriber::registry()
        .with(filter) // 全局日志采集层：每条 INFO+ 事件进入环形缓冲 + 广播，
        // 供 `GET /api/logs` WS 客户端（macApp / AI 调试）实时读取。
        .with(crate::gateway::ws::log_stream::layer())
        // 时间戳统一本地 `YYYY-MM-DD HH:MM:SS`（observability::logtime 是唯一时间出口，
        // /api/logs 日志流与控制台两处同格式；Docker 镜像默认 TZ=Asia/Shanghai）
        .with(tracing_subscriber::fmt::layer().with_timer(crate::observability::logtime::LocalDatetime));
    tracing::subscriber::set_global_default(subscriber)?;

    tracing::info!(
        "配置加载完成：ASR=SenseVoice, TTS=Kokoro, LLM=http, 监听=0.0.0.0:{}, tokio worker={} 线程",
        config.server.port,
        config.server.worker_threads.max(1)
    );

    let engines = Engines::new(&config, config_store)?;
    // 延迟/事件聚合（P1-1）：进程最早的订阅者——spawn 越早，broadcast 环形缓冲越不丢
    // 早期事件（LAGGED 只告警；聚合口径是近似值，见 app/metrics 模块注释）。
    tokio::spawn(crate::observability::metrics::subscriber(
        engines.metrics.clone(),
        engines.event_bus.subscribe(),
    ));
    // 配置来源可见性：把实际加载的配置来源打到启动日志（"改配置不生效"类问题第一现场）
    match engines.config_store.location() {
        Some(p) => {
            let kind = engines.config_store.backend();
            if crate::gateway::ws::config_is_persistent(p) {
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
    crate::device::voices::init();
    crate::device::voices::spawn_startup_probe(engines.clone());
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
    // 旧格式遗留（`.hot` 容器 / `managed.json` 登记表）不做自动清理：内容寻址缓存
    // 永远不会再命中它们，磁盘占用由使用者手动处理（见 `media::hot_audio` 模块文档）。
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
    // clone 而非 move：专用端口监听（下方 echomuse 段）还需要同一份 Engines。
    let app = router(engines.clone());

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
    match crate::device::local_ipv4() {
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
    // 广播端口跟随实际监听：`[echomuse].plain_port`（默认 8767，官方明文 controller
    // 端口——上游固件 tcp_peer() 只认远端 8767/8770 的连接为 controller）额外监听
    // 成功则指向专用端口；绑定失败或 plain_port=0 时回退主端口（/em/* 两边都在）。
    if config.echomuse.enabled {
        let mdns_port = match crate::device::echomuse::listen::dedicated_port(&config.echomuse) {
            Some(p) => crate::device::echomuse::listen::spawn_dedicated(engines.clone(), p)
                .await
                .unwrap_or(config.server.port),
            None => config.server.port,
        };
        crate::device::echomuse::spawn_mdns(mdns_port);
        // 设备语音门模型（Silero v5）只在启用 EchoMuse 时才需要：后台按官方源补一份
        // 到模型卷（`$XIAOZHI_MODELS_DIR/oww/silero_vad.onnx`），供「安装 VAD」接口下发。
        // 不阻断启动、失败只告警；未启用 EchoMuse 的部署不会下载它。
        crate::device::echomuse::oww::spawn_prefetch();
    }
    // ESPHome Native API 桥（可选模块）：`--features esp_home` 编译期开关 +
    // `[esp_home].enabled` 运行期开关，默认关闭。暴露 Dot 2 状态（binary_sensor：
    // 在线/聆听/思考/播报/静音）+ 音量控制（number），并在设备侧 BLE 代理开启时
    // 宣告 `bluetooth_proxy`（Dot 2 当 BLE 代理）。见
    // docs/research/emx-integration-analysis.md §4.1.1 / §4.1.2。
    #[cfg(feature = "esp_home")]
    if config.esp_home.enabled {
        crate::device::esp_home::spawn(config.esp_home.clone(), engines.clone());
    }
    // 管理客户端发现（常驻、无配置项）：广播 `_xiaozhi-admin._tcp`，macApp 等
    // 非 web 客户端经 Bonjour 浏览后自动连接，免手填 IP（见 app/discovery.rs）。
    crate::device::discovery::spawn_admin_mdns(config.server.port);
    // 设备主动拨回的受限 shell 面必须绑定当前 control 的真实对端地址；
    // ConnectInfo 来自 TCP 连接，不信任可伪造的转发头。
    // 带 TCP keepalive（src/serve.rs）：设备断电不发 FIN，半开连接靠内核探测
    // ~35s 内判死——WS 会话收尾、hub 摘除，管理台才能把断电设备转成离线。
    crate::serve::serve_with_keepalive(listener, app).await?;
    Ok(())
}

/// 默认日志过滤（`RUST_LOG` 未设置时生效）。
///
/// `ort::logging` 是 ONNX Runtime C 库经 ort crate 转发的内部日志：会话初始化时的
/// Session Options / GraphTransformer / 内存分配明细，每次加载模型刷上百行 INFO。
/// 压到 WARN 等效于"明细改成 DEBUG"：默认不刷屏，排障时 `RUST_LOG=debug` 全量可见；
/// `ort` 本体（dylib 加载一行）与业务日志保持默认 info 档。
/// 部署文件不得内置 `RUST_LOG=info` 之类的默认值，否则本兜底永不生效。
fn default_log_filter() -> String {
    "info,ort::logging=warn".to_string()
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
            // 存量配置的默认值对齐：老库把「标准行为」存成了显式 false，代码把默认值
            // 改成 true 也不会生效（serde 的默认值只对**缺失键**生效）。这里按库内
            // `defaults_version` 对齐一次：仍是旧默认的项 → 打开；用户显式改过的项 → 保留。
            // 见 `config/defaults.rs`（为什么不能用"一律写默认值"解决）。
            match store.align_defaults() {
                Ok(Some(report)) => tracing::info!("{}", report.summary()),
                Ok(None) => {}
                Err(e) => tracing::warn!("存量配置默认值对齐失败（本次按库内原值启动）: {e:#}"),
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
